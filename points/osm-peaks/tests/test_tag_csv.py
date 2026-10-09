"""外部通信をせず、タグの付与と CSV の補完結果を検証する。"""

import gzip
import json
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import patch

from scripts.tag_csv import apply_tag_csvs, complete_rows, process_csv, read_csv
from scripts.build_data import build
from scripts.point_tags import validate_tags


class TagCsvTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.tags = self.root / "tags"
        self.tags.mkdir()
        self.points = [
            {"id": "204683948", "osmId": 204683948, "name": "羊蹄山", "aliases": ["蝦夷富士"], "latitude": 42.8, "longitude": 140.8},
            {"id": "14255518396", "osmId": 14255518396, "name": "丸山", "latitude": 35, "longitude": 139},
            {"id": "3", "osmId": 3, "name": "丸山", "latitude": 36, "longitude": 140},
        ]

    def csv(self, text, name="日本百名山.csv"):
        path = self.tags / name
        path.write_text(text, encoding="utf-8", newline="\n")
        return path

    def test_adds_multiple_tags_preserves_existing_and_untagged_points(self):
        self.points[0]["tags"] = ["既存", "日本百名山"]
        self.csv("osmId,name\n204683948,羊蹄山\n")
        self.csv("osmId,name\n204683948,蝦夷富士\n", "花の百名山.csv")
        apply_tag_csvs(self.points, self.tags)
        self.assertEqual(self.points[0]["tags"], ["既存", "日本百名山", "花の百名山"])
        self.assertNotIn("tags", self.points[1])
        validate_tags(self.points)

    def test_missing_id_aborts_without_partial_changes(self):
        self.csv("osmId,name\n204683948,羊蹄山\n999,不明\n")
        with self.assertRaises(ValueError):
            apply_tag_csvs(self.points, self.tags)
        self.assertNotIn("tags", self.points[0])

    def test_missing_directory_is_error(self):
        with self.assertRaises(ValueError):
            apply_tag_csvs(self.points, self.root / "missing")

    def test_invalid_csv(self):
        for text in ("name\n羊蹄山\n", "osmId,name\n0,山\n", "osmId,name\n1.0,山\n",
                     "osmId,name\n1,山\n1,山\n", "osmId,name\n1, 山\n", "osmId,name\n1,\n",
                     "osmId,name\n,\n", "osmId,name\n1,山,追加\n", "osmId,osmId\n1,1\n",
                     "osmId,name\n1\n", "osmId,name\n-1,山\n"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                read_csv(self.csv(text), complete=True)
        with self.assertRaises(ValueError):
            read_csv(self.csv("osmId,name\n1,山\n", " 日本百名山.csv"))

    def test_name_mismatch_warns_but_uses_id(self):
        self.csv("osmId,name\n204683948,別の山名\n")
        with self.assertLogs(level="WARNING"):
            apply_tag_csvs(self.points, self.tags)
        self.assertEqual(self.points[0]["tags"], ["日本百名山"])

    def test_bidirectional_completion_and_preservation(self):
        rows = [{"osmId": "", "name": "蝦夷富士"}, {"osmId": "14255518396", "name": ""}]
        completed, report, unresolved = complete_rows(rows, self.points)
        self.assertFalse(unresolved)
        self.assertEqual(completed, [{"osmId": "204683948", "name": "蝦夷富士"}, {"osmId": "14255518396", "name": "丸山"}])
        self.assertEqual(rows[0]["osmId"], "")
        completed, report, unresolved = complete_rows([{"osmId": "204683948", "name": "違う名前"}], self.points)
        self.assertEqual(completed[0]["name"], "違う名前")
        self.assertIn("要確認", report[0][1])

    def test_ambiguous_and_missing_are_unresolved(self):
        rows = [{"osmId": "", "name": "丸山"}, {"osmId": "999", "name": ""}, {"osmId": "", "name": "不明"}]
        completed, report, unresolved = complete_rows(rows, self.points)
        self.assertTrue(unresolved)
        self.assertEqual(completed, rows)
        self.assertEqual(len(report[0][2]), 2)

    def test_duplicate_after_completion_is_error(self):
        with self.assertRaises(ValueError):
            complete_rows([{"osmId": "", "name": "蝦夷富士"}, {"osmId": "204683948", "name": ""}], self.points)

    def test_plain_gzip_bom_and_source_preservation(self):
        for compressed in (False, True):
            with self.subTest(compressed=compressed):
                path = self.root / ("points.json.gz" if compressed else "points.json")
                raw = json.dumps(self.points, ensure_ascii=False).encode("utf-8")
                path.write_bytes(gzip.compress(raw) if compressed else raw)
                source = self.csv("\ufeffosmId\n204683948\n")
                before = source.read_bytes()
                output_csv, output_html, unresolved = process_csv(source, path, self.root / "output")
                self.assertFalse(unresolved)
                self.assertEqual(source.read_bytes(), before)
                self.assertEqual(read_csv(output_csv, complete=True), [{"osmId": "204683948", "name": "羊蹄山"}])
                self.assertIn("https://www.openstreetmap.org/node/204683948", output_html.read_text(encoding="utf-8-sig"))
                self.assertFalse(output_html.read_bytes().startswith(b"\xef\xbb\xbf"))
                self.assertTrue(output_csv.read_bytes().startswith(b"\xef\xbb\xbf#osmId,name\n"))
                self.assertNotIn(b"\r", output_csv.read_bytes())
                with self.assertRaises(ValueError):
                    process_csv(source, path, source.parent)

    def test_unresolved_html_contains_candidates_and_escapes_names(self):
        path = self.root / "points.json"
        self.points[1]["name"] = "丸|山<script>"
        self.points[2]["name"] = "丸|山<script>"
        path.write_text(json.dumps(self.points), encoding="utf-8")
        source = self.csv("name\n丸|山<script>\n")
        _, output_html, unresolved = process_csv(source, path, self.root / "output")
        self.assertTrue(unresolved)
        report = output_html.read_text(encoding="utf-8-sig")
        self.assertIn("丸|山&lt;script&gt;", report)
        self.assertIn("node/14255518396", report)
        self.assertIn("node/3", report)
        payload = report.split('<script id="points" type="application/json">', 1)[1].split('</script>', 1)[0]
        entries = json.loads(payload)
        self.assertEqual(len(entries), 2)
        self.assertTrue(all(entry["review"] for entry in entries))
        self.assertEqual(entries[0]["pointName"], "丸|山<script>")

    def test_build_includes_csv_tags_in_distribution(self):
        self.csv("osmId,name\n204683948,羊蹄山\n")
        def extract(command, **kwargs):
            Path(command[-1]).write_text("<osm><node id='204683948' lat='42.8' lon='140.8' timestamp='2026-09-29T00:00:00Z'><tag k='natural' v='peak'/><tag k='name' v='羊蹄山'/></node></osm>", encoding="utf-8")
        with patch("scripts.build_data.verified_source_url", return_value="https://example.com/source"), \
                patch("scripts.build_data.subprocess.check_output", return_value="2026-09-30T00:00:00Z"), \
                patch("scripts.build_data.subprocess.run", side_effect=extract):
            manifest = build(self.root / "input.pbf", self.root / "dist", "test", tags_directory=self.tags)
        with gzip.open(self.root / "dist/osm-peaks.json.gz", "rt", encoding="utf-8") as stream:
            self.assertEqual(json.load(stream)[0]["tags"], ["日本百名山"])
        self.assertEqual(manifest["dataSchemaVersion"], 5)

    def test_cli_exit_codes_and_reports(self):
        points = self.root / "points.json"
        points.write_text(json.dumps(self.points), encoding="utf-8")
        script = Path(__file__).resolve().parents[1] / "scripts/tag_csv.py"
        for text, expected in (("name\n羊蹄山\n", 0), ("name\n丸山\n", 1), ("osmId\n0\n", 2)):
            with self.subTest(expected=expected):
                source = self.csv(text)
                output = self.root / f"cli-{expected}"
                result = subprocess.run([sys.executable, str(script), str(source), "--points", str(points),
                                         "--output-dir", str(output)], capture_output=True)
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertEqual((output / "日本百名山.html").exists(), expected != 2)

    def test_html_escapes_script_termination_and_keeps_unlocated_rows(self):
        from scripts.tag_csv import make_html
        name = '</script><script>alert(1)</script>__DATA__'
        point = dict(self.points[0], name=name)
        report = [({"osmId": "204683948", "name": name}, "確認済み", [point]),
                  ({"osmId": "999", "name": "見つからない山"}, "ID が見つかりません", [])]
        page = make_html(name, report)
        self.assertNotIn('<script>alert(1)</script>', page)
        payload = page.split('<script id="points" type="application/json">', 1)[1].split('</script>', 1)[0]
        entries = json.loads(payload)
        self.assertEqual(entries[0]["pointName"], name)
        self.assertIsNone(entries[1]["latitude"])
        self.assertIn('https://www.openstreetmap.org/node/999', page)
        self.assertIn('地図ライブラリを読み込めません', page)
        self.assertIn('https://cyberjapandata.gsi.go.jp/xyz/std/{z}/{x}/{y}.png', page)
        self.assertIn('地理院タイル', page)
        self.assertNotIn('location.protocol', page)
        self.assertNotIn('http.server', page)

    def test_all_points_plain_and_gzip_cli(self):
        script = Path(__file__).resolve().parents[1] / "scripts/tag_csv.py"
        for compressed in (False, True):
            with self.subTest(compressed=compressed):
                source = self.root / ("all.json.gz" if compressed else "all.json")
                data = json.dumps(self.points).encode("utf-8")
                source.write_bytes(gzip.compress(data) if compressed else data)
                before = source.read_bytes()
                output = self.root / f"all-{compressed}"
                result = subprocess.run([sys.executable, str(script), "--all", "--points", str(source),
                                         "--output-dir", str(output)], capture_output=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                page = (output / "osm-peaks.html").read_text(encoding="utf-8")
                payload = page.split('<script id="points" type="application/json">', 1)[1].split('</script>', 1)[0]
                entries = json.loads(payload)
                self.assertEqual(len(entries), len(self.points))
                self.assertEqual(entries[1]["osmId"], "14255518396")
                self.assertEqual(source.read_bytes(), before)
                self.assertEqual(list(output.glob("*.csv")), [])

    def test_all_points_rejects_missing_osm_id(self):
        from scripts.tag_csv import process_all_points
        source = self.root / "missing.json"
        source.write_text(json.dumps([{"id": "manual", "name": "山"}]), encoding="utf-8")
        with self.assertRaises(ValueError):
            process_all_points(source, self.root / "output")

    def test_report_embeds_full_json_and_original_tags(self):
        from scripts.tag_csv import make_html
        point = dict(self.points[0], tags=["日本百名山", "花の百名山"], graphic={"svg": "<svg>例</svg>", "scale": 1.5}, wikipediaUrl=None)
        page = make_html("確認", [({"osmId": "204683948", "name": "羊蹄山"}, "確認済み", [point])])
        payload = page.split('<script id="points" type="application/json">', 1)[1].split('</script>', 1)[0]
        entry = json.loads(payload)[0]
        self.assertEqual(entry["source"], point)
        self.assertEqual(entry["tags"], point["tags"])
        self.assertNotIn("nameReading", entry["source"])
        self.assertNotIn("<svg>例</svg>", payload)

    def test_standalone_viewer_cli_without_input(self):
        script = Path(__file__).resolve().parents[1] / "scripts/tag_csv.py"
        output = self.root / "viewer"
        result = subprocess.run([sys.executable, str(script), "--viewer", "--output-dir", str(output)], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        page = (output / "point-viewer.html").read_text(encoding="utf-8")
        self.assertNotIn('<script id="points"', page)
        self.assertIn("let points=[];", page)
        self.assertIn('id="json-file"', page)
        self.assertIn("DecompressionStream('gzip')", page)
        for args in (["--all"], ["--viewer", "--all"], ["--viewer", "--points", "input.json"]):
            with self.subTest(args=args):
                result = subprocess.run([sys.executable, str(script), *args], capture_output=True)
                self.assertEqual(result.returncode, 2)

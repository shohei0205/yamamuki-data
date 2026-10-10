"""外部通信をせず、タグの付与と JSON の補完結果を検証する。"""

import gzip
import json
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import patch

from scripts.tag_json import apply_tag_jsons, complete_rows, process_json, read_tag_json
from scripts.build_data import build
from scripts.point_tags import validate_tags


class TagJsonTests(unittest.TestCase):
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

    def tag_json(self, rows, tag="日本百名山", **metadata):
        path = self.tags / (tag + ".json")
        path.write_text(json.dumps({"tag": tag, **metadata, "points": rows}, ensure_ascii=False), encoding="utf-8")
        return path

    def test_multiple_tags_preserve_source_and_manifest_metadata(self):
        self.points[0]["tags"] = ["既存"]
        self.tag_json([{"osmId": 204683948, "note": "羊蹄山"}], source="Wikipedia", license="CC BY-SA 4.0")
        self.tag_json([{"osmId": 204683948, "note": "蝦夷富士"}], tag="花の百名山", source="別の出典")
        sources = apply_tag_jsons(self.points, self.tags)
        self.assertEqual(set(self.points[0]["tags"]), {"既存", "日本百名山", "花の百名山"})
        self.assertNotIn("tags", self.points[1])
        self.assertEqual([item["tag"] for item in sources], ["日本百名山", "花の百名山"])
        self.assertEqual(sources[0]["license"], "CC BY-SA 4.0")

    def test_supplement_reading_override_and_alias_append(self):
        self.points[0]["nameReading"] = "もとのよみ"
        self.tag_json([{"osmId": 204683948, "note": "羊蹄山", "nameReading": "ようていざん", "aliases": ["蝦夷富士", "後方羊蹄山"]}])
        apply_tag_jsons(self.points, self.tags)
        self.assertEqual(self.points[0]["nameReading"], "ようていざん")
        self.assertEqual(self.points[0]["aliases"], ["蝦夷富士", "後方羊蹄山"])
        self.assertEqual(self.points[0]["tags"], ["日本百名山"])

    def test_supplement_without_tag_preserves_source_and_completes(self):
        source = self.tags / "よみがな補足.json"
        source.write_text(json.dumps({"source": "確認資料", "points": [{"osmId": 204683948, "nameReading": "ようていざん", "aliases": ["後方羊蹄山"]}]}), encoding="utf-8")
        points = self.root / "points.json"
        points.write_text(json.dumps(self.points), encoding="utf-8")
        output, _, unresolved = process_json(source, points, self.root / "completed")
        self.assertFalse(unresolved)
        data = json.loads(output.read_text(encoding="utf-8"))
        self.assertNotIn("tag", data)
        self.assertEqual(data["points"][0]["nameReading"], "ようていざん")
        self.assertEqual(data["points"][0]["aliases"], ["後方羊蹄山"])
        source.write_text(json.dumps(data), encoding="utf-8")
        sources = apply_tag_jsons(self.points, self.tags)
        self.assertEqual(sources, [{"name": "よみがな補足", "source": "確認資料"}])
        self.assertNotIn("tags", self.points[0])
        from scripts.tag_json import validate_legacy_sources
        validate_legacy_sources(sources)

    def test_conflicting_readings_are_atomic(self):
        self.tag_json([{"osmId": 204683948, "note": "羊蹄山", "nameReading": "ようていざん"}])
        self.tag_json([{"osmId": 204683948, "note": "羊蹄山", "nameReading": "しりべしやま"}], tag="別資料")
        before = json.dumps(self.points)
        with self.assertRaisesRegex(ValueError, "よみがなが競合"):
            apply_tag_jsons(self.points, self.tags)
        self.assertEqual(json.dumps(self.points), before)

    def test_supplement_values_are_strict(self):
        for extra in [{"nameReading": ""}, {"nameReading": " よみ"}, {"nameReading": None}, {"aliases": "別名"}, {"aliases": [""]}, {"aliases": ["別名", "別名"]}, {"aliases": [1]}]:
            path = self.tag_json([{"osmId": 204683948, "note": "羊蹄山", **extra}])
            with self.subTest(extra=extra), self.assertRaises(ValueError):
                read_tag_json(path, complete=True)

    def test_invalid_values_and_unknown_fields(self):
        path = self.tags / "日本百名山.json"
        valid = {"tag": "日本百名山", "points": [{"osmId": 1, "note": "山"}]}
        for row in [{"osmId": 0, "note": "山"}, {"osmId": True, "note": "山"}, {"osmId": "1", "note": "山"}, {"osmId": 1, "note": 123}, {"osmId": 1, "name": "山"}, {"note": "山"}, {"osmId": 1, "note": "山", "extra": 0}]:
            path.write_text(json.dumps({**valid, "points": [row]}), encoding="utf-8")
            with self.subTest(row=row), self.assertRaises(ValueError):
                read_tag_json(path, complete=True)
        for data in [[], {**valid, "tag": "別名"}, {**valid, "source": []}, {**valid, "extra": 0}, {**valid, "points": valid["points"] * 2}]:
            path.write_text(json.dumps(data), encoding="utf-8")
            with self.subTest(data=data), self.assertRaises(ValueError):
                read_tag_json(path, complete=True)

    def test_missing_id_does_not_partially_modify_points(self):
        self.tag_json([{"osmId": 204683948, "note": "羊蹄山"}, {"osmId": 999, "note": "不明"}])
        with self.assertRaises(ValueError):
            apply_tag_jsons(self.points, self.tags)
        self.assertNotIn("tags", self.points[0])

    def test_missing_directory_and_empty_directory(self):
        with self.assertRaises(ValueError):
            apply_tag_jsons(self.points, self.root / "missing")
        self.assertEqual(apply_tag_jsons(self.points, self.tags), [])

    def test_note_does_not_change_or_compare_name(self):
        self.tag_json([{"osmId": 204683948, "note": "別名ではない"}])
        apply_tag_jsons(self.points, self.tags)
        self.assertEqual(self.points[0]["name"], "羊蹄山")
        self.assertEqual(self.points[0]["tags"], ["日本百名山"])

    def test_completion_preserves_metadata_input_and_encoding(self):
        source = self.tag_json([{"note": "蝦夷富士"}, {"osmId": 14255518396}], source="Wikipedia, 日本百名山", license="CC BY-SA 4.0", attribution="投稿者", changes="抽出\n補完")
        before = source.read_bytes()
        for compressed in (False, True):
            points = self.root / ("points.json.gz" if compressed else "points.json")
            raw = json.dumps(self.points).encode("utf-8")
            points.write_bytes(gzip.compress(raw) if compressed else raw)
            output, _, unresolved = process_json(source, points, self.root / "output")
            self.assertFalse(unresolved)
            self.assertEqual(source.read_bytes(), before)
            data = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(data["source"], "Wikipedia, 日本百名山")
            self.assertEqual(data["changes"], "抽出\n補完")
            self.assertEqual(data["points"][0], {"osmId": 204683948, "note": "蝦夷富士"})
            self.assertEqual(data["points"][1]["note"], "丸山")
            self.assertFalse(output.read_bytes().startswith(b"\xef\xbb\xbf"))
        with self.assertRaises(ValueError):
            process_json(source, points, self.tags)

    def test_cli_exit_codes(self):
        points = self.root / "points.json"
        points.write_text(json.dumps(self.points), encoding="utf-8")
        script = Path(__file__).resolve().parents[1] / "scripts/tag_json.py"
        for rows, expected in [([{"note": "羊蹄山"}], 0), ([{"note": "丸山"}], 1), ([{"osmId": 0}], 2)]:
            source = self.tag_json(rows)
            output = self.root / str(expected)
            result = subprocess.run([sys.executable, str(script), str(source), "--points", str(points), "--output-dir", str(output)], capture_output=True)
            self.assertEqual(result.returncode, expected, result.stderr)
            self.assertEqual((output / source.name).exists(), expected != 2)
            self.assertFalse(list(output.glob("*.html")))

    def test_bidirectional_completion_and_preservation(self):
        rows = [{"osmId": "", "note": "蝦夷富士"}, {"osmId": "14255518396", "note": ""}]
        completed, report, unresolved = complete_rows(rows, self.points)
        self.assertFalse(unresolved)
        self.assertEqual(completed, [{"osmId": "204683948", "note": "蝦夷富士"}, {"osmId": "14255518396", "note": "丸山"}])
        self.assertEqual(rows[0]["osmId"], "")
        completed, report, unresolved = complete_rows([{"osmId": "204683948", "note": "違う名前"}], self.points)
        self.assertEqual(completed[0]["note"], "違う名前")
        self.assertEqual(report[0][1], "確認済み")

    def test_ambiguous_and_missing_are_unresolved(self):
        rows = [{"osmId": "", "note": "丸山"}, {"osmId": "999", "note": ""}, {"osmId": "", "note": "不明"}]
        completed, report, unresolved = complete_rows(rows, self.points)
        self.assertTrue(unresolved)
        self.assertEqual(completed, rows)
        self.assertEqual(len(report[0][2]), 2)

    def test_duplicate_after_completion_is_error(self):
        with self.assertRaises(ValueError):
            complete_rows([{"osmId": "", "note": "蝦夷富士"}, {"osmId": "204683948", "note": ""}], self.points)

    def test_build_includes_tags_and_sources_in_distribution(self):
        self.tag_json([{ "osmId": 204683948, "note": "羊蹄山" }], source="固定リンク", license="CC BY-SA 4.0", attribution="投稿者", changes="ID を追加")
        self.tag_json([{"osmId": 204683948, "note": "羊蹄山"}], tag="花の百名山", source="別の出典", license="CC BY 4.0")
        (self.tags / "よみがな補足.json").write_text(json.dumps({"source": "よみがな資料", "license": "CC0-1.0", "points": [{"osmId": 204683948, "note": "羊蹄山", "nameReading": "ようていざん", "aliases": ["後方羊蹄山"]}]}), encoding="utf-8")
        def extract(command, **kwargs):
            Path(command[-1]).write_text("<osm><node id='204683948' lat='42.8' lon='140.8' timestamp='2026-09-29T00:00:00Z'><tag k='natural' v='peak'/><tag k='name' v='羊蹄山'/></node></osm>", encoding="utf-8")
        with patch("scripts.build_data.verified_source_url", return_value="https://example.com/source"), \
                patch("scripts.build_data.subprocess.check_output", return_value="2026-09-30T00:00:00Z"), \
                patch("scripts.build_data.subprocess.run", side_effect=extract):
            manifest = build(self.root / "input.pbf", self.root / "dist", "test", tags_directory=self.tags)
        with gzip.open(self.root / "dist/osm-peaks.json.gz", "rt", encoding="utf-8") as stream:
            point = json.load(stream)[0]
            self.assertEqual(point["tags"], ["日本百名山", "花の百名山"])
            self.assertEqual(point["nameReading"], "ようていざん")
            self.assertEqual(point["aliases"], ["後方羊蹄山"])
        self.assertEqual(manifest["dataSchemaVersion"], 5)

        self.assertEqual(manifest["license"], "ODbL-1.0")
        self.assertEqual(manifest["attribution"], "© OpenStreetMap contributors")
        self.assertEqual(manifest["supplementSources"], {"よみがな補足": { "source": "よみがな資料", "license": "CC0-1.0"}, "日本百名山": { "source": "固定リンク", "license": "CC BY-SA 4.0", "attribution": "投稿者", "changes": "ID を追加"}, "花の百名山": { "source": "別の出典", "license": "CC BY 4.0"}})


        from scripts.check_release import validate
        self.assertEqual(validate(self.root / "dist")[0]["supplementSources"], manifest["supplementSources"])
        saved_manifest = json.loads((self.root / "dist/manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(saved_manifest["supplementSources"], manifest["supplementSources"])

    def test_legacy_sources_validation(self):
        from scripts.tag_json import validate_legacy_sources
        validate_legacy_sources([{"tag": "分類", "source": "出典", "license": "CC BY-SA 4.0"}])
        for sources in [None, {}, [{"tag": "分類", "name": "補足"}], [{"name": ""}], [{"tag": "分類", "license": []}], [{"tag": "分類"}, {"tag": "分類"}], [{"tag": ""}], [{"tag": "分類", "unknown": "値"}]]:
            with self.subTest(sources=sources), self.assertRaises(ValueError):
                validate_legacy_sources(sources)

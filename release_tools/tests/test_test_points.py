"""開発版専用の架空データを検証する。"""

import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from release_tools import test_points as target


class TestPoints(unittest.TestCase):
    def test_generate(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            manifest = target.generate("test-1", directory)
            contents = (directory / manifest["fileName"]).read_bytes()
            rows = json.loads(gzip.decompress(contents))
            self.assertEqual(len(rows), manifest["pointCount"])
            self.assertEqual(len({row["type"] for row in rows}), 11)
            self.assertEqual(rows[0]["graphic"]["scale"], 1.5)
            self.assertEqual(rows[0]["tags"], ["テスト用百名山", "展望確認"])
            self.assertEqual(hashlib.sha256(contents).hexdigest(), manifest["sha256"])
            self.assertIn("/testdata-dev-test-1/", manifest["downloadUrl"])
            self.assertEqual(target.generate("test-1", directory), manifest)

    def test_refuses_other_branches(self):
        for branch in ("refs/heads/main", "refs/heads/topic", ""):
            with self.subTest(branch=branch), patch.dict(os.environ, {"GITHUB_REF": branch}), patch.object(target.release_data, "gh") as gh:
                with self.assertRaisesRegex(ValueError, "dev"):
                    target.publish("test-1", Path("unused"))
                gh.assert_not_called()

    def test_optional_type_and_graphic_can_be_null(self):
        row = {"id": "a", "name": "地点", "latitude": 35, "longitude": 138}
        target.validate_rows([row])
        target.validate_rows([{**row, "type": None, "graphic": None, "tags": None}])

    def test_invalid_rows(self):
        row = {"id": "a", "name": "地点", "type": "peak", "latitude": 35, "longitude": 138}
        for rows in ([], [row, row], [{**row, "tags": ["重複", "重複"]}], [{**row, "latitude": True}], [{**row, "longitude": 181}], [{**row, "graphic": {"svg": "<script/>"}}]):
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                target.validate_rows(rows)

    def test_publish_preserves_other_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment = {"GH_REPO": "owner/repo", "GITHUB_REF": "refs/heads/dev", "GITHUB_SHA": "abc", "GITHUB_RUN_ID": "123", "PAGES_DIRECTORY": str(root / "pages")}
            with patch.dict(os.environ, environment):
                original = target.generate("old", root / "old")
                catalog = target.release_data.Catalog({"points/other/manifest.json": original, "points/other-dev/manifest.json": original}, {"points/other/history.json": []})
                def gh(*args):
                    if args[:2] == ("release", "download"):
                        checked = Path(args[args.index("--dir") + 1])
                        checked.mkdir()
                        for name in ("manifest.json", "test-points.json.gz"):
                            shutil.copyfile(root / "dist" / name, checked / name)
                    return ""
                with patch.object(target.release_data, "read_catalog", return_value=catalog), patch.object(target.release_data, "gh", side_effect=gh) as calls, patch.object(target.release_data, "append_summary"):
                    target.publish("test-1", root / "dist")
                stable = json.loads((root / "pages/points/catalog.json").read_text(encoding="utf-8"))
                dev = json.loads((root / "pages/points/catalog-dev.json").read_text(encoding="utf-8"))
                self.assertEqual([entry["id"] for entry in stable["datasets"]], ["other"])
                self.assertEqual({entry["id"] for entry in dev["datasets"]}, {"other", "testdata"})
                self.assertEqual(json.loads((root / "pages/points/other/manifest.json").read_text(encoding="utf-8")), original)
                self.assertEqual(catalog.histories, {"points/other/history.json": []})
                self.assertEqual(calls.call_args.args[:2], ("release", "edit"))


if __name__ == "__main__":
    unittest.main()

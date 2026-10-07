"""地点 JSON 内の SVG・表示倍率・生成と公開時の検査を確かめる。"""

import json
import tempfile
from pathlib import Path
import xml.etree.ElementTree as ET
import unittest
from unittest.mock import patch

from scripts.build_data import write_distribution, build
from scripts.check_release import validate
from scripts.graphics import validate_svg, validate_graphics
from scripts import release_data


SVG = b'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 24 L12 0 L24 24 Z"/></svg>'


class GraphicsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "graphics"
        self.source.mkdir()
        (self.source / "fuji.svg").write_bytes(SVG)
        self.output = self.root / "dist"
        self.rows = [{"id": "1", "type": "peak", "name": "富士山", "latitude": 35,
                      "longitude": 139, "graphic": {"svg": SVG.decode("utf-8"), "scale": 1.5}}]

    def generate(self, channel="stable"):
        return write_distribution(self.rows, self.output, "test", "2026-09-29T20:00:00Z",
                                  "2026-09-29T12:00:00Z", channel=channel)

    def test_distribution_contains_svg_without_extra_files(self):
        for channel in ("stable", "dev"):
            manifest = self.generate(channel)
            self.assertNotIn("graphics", manifest)
            self.assertEqual(self.rows, validate(self.output, channel=channel)[1])
            self.assertEqual({"manifest.json", "japan-mountains.json.gz"}, {p.name for p in self.output.iterdir()})
            with patch.dict(release_data.os.environ, {"GH_REPO": "shohei0205/yamamuki-data"}):
                entry = release_data.points_catalog({release_data.manifest_path(channel): manifest}, channel)["datasets"][0]
                self.assertNotIn("graphics", entry["manifest"])

    def test_build_applies_point_to_asset_map(self):
        mappings = self.root / "points.json"
        mappings.write_text('{"1":"fuji"}', encoding="utf-8")
        rows = [{key: value for key, value in self.rows[0].items() if key != "graphic"}]
        with patch("scripts.build_data.verified_source_url", return_value="https://download.geofabrik.de/asia/japan-260929.osm.pbf"), \
                patch("scripts.build_data.subprocess.check_output", return_value="2026-09-29T20:00:00Z"), \
                patch("scripts.build_data.subprocess.run"), \
                patch("scripts.build_data.read_mountains", return_value=(rows, "2026-09-29T12:00:00Z")):
            manifest = build(self.root / "unused.pbf", self.output, "test", graphics_directory=self.source, graphics_map=mappings)
        self.assertEqual({"svg": SVG.decode("utf-8")}, rows[0]["graphic"])
        self.assertNotIn("graphics", manifest)

    def test_inline_svg_and_scale_are_validated(self):
        for scale in (1, 1.5, 0.5):
            validate_graphics([dict(self.rows[0], graphic={"svg": SVG.decode("utf-8"), "scale": scale})])
        validate_graphics([dict(self.rows[0], graphic={"svg": SVG.decode("utf-8")})])
        for value in (None, "fuji", {}, {"assetId": "fuji"}, {"svg": 1}, {"svg": "bad"},
                      {"svg": SVG.decode("utf-8"), "scale": 0},
                      {"svg": SVG.decode("utf-8"), "scale": -1},
                      {"svg": SVG.decode("utf-8"), "scale": True},
                      {"svg": SVG.decode("utf-8"), "scale": float("inf")},
                      {"svg": SVG.decode("utf-8"), "scale": float("nan")}):
            with self.subTest(value=value), self.assertRaises((ValueError, ET.ParseError)):
                validate_graphics([dict(self.rows[0], graphic=value)])

    def test_invalid_svg_stops_generation(self):
        self.rows[0]["graphic"]["svg"] = "bad"
        with self.assertRaises(ET.ParseError):
            self.generate()
        self.assertFalse((self.output / "manifest.json").exists())

    def test_valid_checksum_does_not_bypass_svg_validation(self):
        import gzip
        import hashlib
        manifest = self.generate()
        self.rows[0]["graphic"]["svg"] = SVG.decode("utf-8").replace("<path", "<script/><path")
        raw = json.dumps(self.rows).encode("utf-8")
        archive = gzip.compress(raw)
        (self.output / "japan-mountains.json.gz").write_bytes(archive)
        manifest.update(sizeBytes=len(archive), uncompressedSizeBytes=len(raw), sha256=hashlib.sha256(archive).hexdigest())
        (self.output / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        with self.assertRaises(ValueError):
            validate(self.output)

    def test_unsupported_svg_rejected(self):
        for contents in (
            SVG.replace(b"viewBox", b"bad"),
            SVG.replace(b"0 0 24 24", b"0 0 -1 24"),
            SVG.replace(b"<path", b'<script/><path'),
            SVG.replace(b"<path", b'<text>text</text><path'),
            SVG.replace(b"<path", b'<image href="https://example.com/a.png"/><path'),
            SVG.replace(b"<path", b'<path onclick="bad()"/><path'),
            SVG.replace(b"<path", b'<path fill="url(https://example.com/a)"/><path'),
            b'<!DOCTYPE svg>' + SVG,
            b'x' * 100_001,
        ):
            with self.subTest(contents=contents[:80]), self.assertRaises((ValueError, ET.ParseError)):
                validate_svg(contents)

    def test_fetch_only_downloads_data_and_manifest(self):
        manifest = self.generate()
        with patch.object(release_data, "gh") as download, \
                patch.object(release_data, "validate", return_value=(manifest, self.rows)):
            release_data.fetch("peaks-test", self.root / "download")
            self.assertEqual(1, download.call_count)
            self.assertNotIn("graphic-fuji.svg", download.call_args.args)

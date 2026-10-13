"""外部通信をせずに抽出・圧縮・manifest の対応を確かめる。"""

import gzip
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

from scripts.download_source import save_source_info
from scripts.build_data import verified_source_url
from scripts.build_data import (
    FILE_NAME, build, normalize_timestamp, parse_elevation, read_mountains,
    write_distribution,
    read_aliases, wikipedia_url, wikidata_url,
)


OSM = '''<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6" generator="test">
  <node id="1" timestamp="2026-09-29T12:00:00Z" lat="35.3606" lon="138.7274">
    <tag k="natural" v="peak"/><tag k="name" v="Mount Fuji"/>
    <tag k="name:ja" v=" 富士山 "/><tag k="ele" v="3,776 m"/>
    <tag k="name:ja-Hira" v=" ふじさん "/><tag k="alt_name:ja" v="富岳"/>
    <tag k="alt_name" v=" 富岳 ; 富士山 ;; 芙蓉峰 "/>
    <tag k="wikipedia" v="ja:富士山"/><tag k="wikidata" v="Q39231"/>
  </node>
  <node id="2" timestamp="2026-09-29T12:00:00Z" lat="35" lon="139">
    <tag k="natural" v="volcano"/><tag k="name" v="火山"/><tag k="ele" v="1000 ft"/>
  </node>
  <node id="3" lat="36" lon="140"><tag k="natural" v="peak"/></node>
  <node id="4" lat="36" lon="140"><tag k="natural" v="tree"/><tag k="name" v="木"/></node>
  <node id="5" timestamp="2026-09-29T12:00:00Z" lat="36" lon="140">
    <tag k="natural" v="peak"/><tag k="name" v="標高不明の山"/><tag k="name:ja" v=" "/>
    <tag k="ele" v="不明"/>
  </node>
  <node id="6" timestamp="2026-09-29T12:00:00Z" lat="36" lon="140">
    <tag k="natural" v="peak"/><tag k="name:ja" v="日本語名だけの山"/>
  </node>
  <way id="1"><nd ref="1"/><tag k="natural" v="peak"/><tag k="name" v="線の山"/></way>
  <relation id="1"><tag k="natural" v="volcano"/><tag k="name" v="関係の火山"/></relation>
</osm>
'''
TIMESTAMP = "2026-09-30T20:21:22Z"


class BuildDataTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.xml = self.root / "fixture.osm"
        self.xml.write_text(OSM, encoding="utf-8")

    def test_named_nodes_and_display_names(self):
        mountains = read_mountains(self.xml)[0]
        self.assertEqual(["1", "2", "5", "6"], [m["id"] for m in mountains])
        self.assertEqual({"id": "1", "type": "peak", "osmId": 1, "name": "富士山", "latitude": 35.3606,
                          "longitude": 138.7274, "elevationM": 3776,
                          "nameReading": "ふじさん", "aliases": ["富岳", "芙蓉峰"],
                          "wikipediaUrl": "https://ja.wikipedia.org/wiki/%E5%AF%8C%E5%A3%AB%E5%B1%B1",
                          "wikidataUrl": "https://www.wikidata.org/wiki/Q39231"}, mountains[0])
        self.assertTrue(all(mountain["type"] == "peak" for mountain in mountains))
        self.assertAlmostEqual(304.8, mountains[1]["elevationM"])
        self.assertEqual("標高不明の山", mountains[2]["name"])
        self.assertIsNone(mountains[2]["elevationM"])

    def test_missing_optional_details(self):
        mountain = read_mountains(self.xml)[0][1]
        self.assertIsNone(mountain["nameReading"])
        self.assertEqual([], mountain["aliases"])
        self.assertIsNone(mountain["wikipediaUrl"])
        self.assertIsNone(mountain["wikidataUrl"])

    def test_latest_timestamp_only_uses_included_peaks(self):
        xml = OSM.replace('<node id="2" timestamp="2026-09-29T12:00:00Z"',
                          '<node id="2" timestamp="2026-09-30T05:00:00+09:00"')
        xml = xml.replace('<node id="3"', '<node id="3" timestamp="2026-10-01T00:00:00Z"')
        xml = xml.replace('<node id="4"', '<node id="4" timestamp="2026-10-02T00:00:00Z"')
        self.xml.write_text(xml, encoding="utf-8")
        mountains, latest = read_mountains(self.xml)
        self.assertEqual(4, len(mountains))
        self.assertEqual("2026-09-29T20:00:00Z", latest)

    def test_missing_or_invalid_peak_timestamp_fails(self):
        for replacement in ('', 'timestamp="invalid"', 'timestamp="2026-09-29T12:00:00"'):
            with self.subTest(replacement=replacement):
                self.xml.write_text(OSM.replace('timestamp="2026-09-29T12:00:00Z"', replacement, 1), encoding="utf-8")
                with self.assertRaises(ValueError):
                    read_mountains(self.xml)

    def test_peak_timestamp_after_source_fails_before_writing(self):
        with self.assertRaises(ValueError):
            write_distribution(read_mountains(self.xml)[0], self.root / "future", "test", TIMESTAMP, "2026-10-01T00:00:00Z")
        self.assertFalse((self.root / "future").exists())

    def test_japanese_alias_and_deduplication(self):
        self.assertEqual(["天城山"], read_aliases({"alt_name:ja": " 天城山 ;", "alt_name": "天城山;万三郎岳"}, "万三郎岳"))
        self.assertEqual([], read_aliases({"alt_name": " ; 山 ; "}, "山"))

    def test_wikipedia_link_encoding_and_section(self):
        self.assertEqual("https://en.wikipedia.org/wiki/Mount_Test#First_peak", wikipedia_url(" en:Mount Test#First peak "))
        self.assertEqual("https://ja.wikipedia.org/wiki/A%2FB%3FC%25", wikipedia_url("ja:A/B?C%"))
        for raw in (None, "", "富士山", "ja:", "ja:#節", "https://example.com", "ja.evil:山", "javascript:alert(1)"):
            with self.subTest(raw=raw):
                self.assertIsNone(wikipedia_url(raw))

    def test_wikidata_identifier(self):
        self.assertEqual("https://www.wikidata.org/wiki/Q39231", wikidata_url(" Q39231 "))
        for raw in (None, "", "Q0", "Q1;Q2", "Q1/other", "https://example.com"):
            with self.subTest(raw=raw):
                self.assertIsNone(wikidata_url(raw))

    def test_elevation_formats(self):
        for raw, expected in [(None, None), ("", None), ("-12.5", -12.5),
                              ("3776;3775", 3776), ("1,234 metres", 1234),
                              ("10 feet", 3.048), ("10'", 3.048), ("1e3", None),
                              ("NaN", None), ("Infinity", None), ("9" * 400, None)]:
            with self.subTest(raw=raw):
                self.assertEqual(expected, parse_elevation(raw))

    def test_empty_or_wrong_document_fails(self):
        for xml in ['<osm/>', '<html/>', '<osm><node']:
            with self.subTest(xml=xml):
                self.xml.write_text(xml, encoding="utf-8")
                with self.assertRaises((ValueError, ET.ParseError)):
                    read_mountains(self.xml)[0]

    def test_invalid_coordinates_fail(self):
        for lat in ["NaN", "inf", "91", "-91", "abc"]:
            with self.subTest(lat=lat):
                self.xml.write_text(OSM.replace('lat="35.3606"', f'lat="{lat}"'), encoding="utf-8")
                with self.assertRaises(ValueError):
                    read_mountains(self.xml)[0]

    def test_duplicate_id_fails(self):
        self.xml.write_text(OSM.replace('<node id="2"', '<node id="1"'), encoding="utf-8")
        with self.assertRaises(ValueError):
            read_mountains(self.xml)[0]

    def test_distribution_roundtrip_and_checksum(self):
        mountains = read_mountains(self.xml)[0]
        manifest = write_distribution(mountains, self.root, "20260930-1", TIMESTAMP, "2026-09-29T12:00:00Z")
        archive = (self.root / FILE_NAME).read_bytes()
        raw = gzip.decompress(archive)
        self.assertEqual(mountains, json.loads(raw))
        self.assertEqual(manifest, json.loads((self.root / "manifest.json").read_text(encoding="utf-8")))
        self.assertEqual(5, manifest["schemaVersion"])
        self.assertEqual("山頂", manifest["name"])
        self.assertEqual("20260930-1", manifest["version"])
        self.assertEqual(TIMESTAMP, manifest["sourceTimestamp"])
        self.assertEqual("2026-09-29T12:00:00Z", manifest["latestPointTimestamp"])
        self.assertEqual(hashlib.sha256(archive).hexdigest(), manifest["sha256"])
        self.assertEqual(len(archive), manifest["sizeBytes"])
        self.assertEqual(len(raw), manifest["uncompressedSizeBytes"])
        self.assertEqual(4, manifest["pointCount"])
        write_distribution(mountains, self.root / "again", "20260930-2", TIMESTAMP, "2026-09-29T12:00:00Z")
        self.assertEqual(archive, (self.root / "again" / FILE_NAME).read_bytes())

    def test_oversized_data_does_not_replace_previous_distribution(self):
        mountains = read_mountains(self.xml)[0]
        write_distribution(mountains, self.root, "first", TIMESTAMP, "2026-09-29T12:00:00Z")
        previous = [(self.root / name).read_bytes() for name in (FILE_NAME, "manifest.json")]
        with patch("scripts.build_data.MAX_SIZE_BYTES", 1):
            with self.assertRaises(ValueError):
                write_distribution(mountains, self.root, "second", TIMESTAMP, "2026-09-29T12:00:00Z")
        self.assertEqual(previous, [(self.root / name).read_bytes() for name in (FILE_NAME, "manifest.json")])

    def test_timestamp_requires_timezone(self):
        self.assertEqual(TIMESTAMP, normalize_timestamp("2026-10-01T05:21:22+09:00"))
        for stamp in ["", "2026-09-30", "2026-09-30T20:21:22", "bad"]:
            with self.subTest(stamp=stamp), self.assertRaises(ValueError):
                normalize_timestamp(stamp)

    def test_bad_version_does_not_write_files(self):
        for version in ["", "../test", "a/b", "version\n"]:
            with self.subTest(version=version), self.assertRaises(ValueError):
                write_distribution(read_mountains(self.xml)[0], self.root / "output", version, TIMESTAMP, "2026-09-29T12:00:00Z")
        self.assertFalse((self.root / "output").exists())

    def test_missing_source_timestamp_stops_before_extraction(self):
        pbf = self.root / "input.osm.pbf"
        pbf.write_bytes(b"fixture")
        self.save_fixture_source(pbf)
        with patch("scripts.build_data.subprocess.check_output", return_value=""), \
                patch("scripts.build_data.subprocess.run") as extract:
            with self.assertRaises(ValueError):
                build(self.root / "input.osm.pbf", self.root / "output", "test")
            extract.assert_not_called()

    def save_fixture_source(self, pbf):
        save_source_info(pbf, {"url": "https://download.geofabrik.de/asia/japan-260929.osm.pbf",
                              "sizeBytes": pbf.stat().st_size,
                              "md5": hashlib.md5(pbf.read_bytes()).hexdigest()})

    def test_source_info_missing_or_mismatched_stops_generation(self):
        pbf = self.root / "source.osm.pbf"
        pbf.write_bytes(b"fixture")
        with self.assertRaises(ValueError):
            verified_source_url(pbf)
        self.save_fixture_source(pbf)
        self.assertTrue(verified_source_url(pbf).endswith("japan-260929.osm.pbf"))
        pbf.write_bytes(b"changed")
        with self.assertRaises(ValueError):
            verified_source_url(pbf)
        pbf.write_bytes(b"different size")
        with self.assertRaises(ValueError):
            verified_source_url(pbf)

    def test_manifest_uses_provided_source_url(self):
        url = "https://download.geofabrik.de/asia/japan-260929.osm.pbf"
        manifest = write_distribution(read_mountains(self.xml)[0], self.root / "dated", "test",
                                      TIMESTAMP, "2026-09-29T12:00:00Z", source_url=url)
        self.assertEqual(url, manifest["sourceUrl"])
        self.assertEqual(url, json.loads((self.root / "dated/manifest.json").read_text(encoding="utf-8"))["sourceUrl"])

    @unittest.skipUnless(shutil.which("osmium"), "osmium がないため、PBF の生成テストは CI で実行")
    def test_real_pbf_to_distribution(self):
        pbf = self.root / "fixture.osm.pbf"
        subprocess.run(["osmium", "cat", str(self.xml), "-o", str(pbf),
                        f"--output-header=osmosis_replication_timestamp={TIMESTAMP}"], check=True)
        self.save_fixture_source(pbf)
        supplements = self.root / "supplements.json"
        supplements.write_text(json.dumps({"schemaVersion": 1, "sources": {}, "points": []}), encoding="utf-8")
        manifest = build(pbf, self.root / "output", "test", supplements_path=supplements)
        self.assertEqual("https://download.geofabrik.de/asia/japan-260929.osm.pbf", manifest["sourceUrl"])
        self.assertEqual(4, manifest["pointCount"])
        self.assertEqual(TIMESTAMP, manifest["sourceTimestamp"])
        self.assertEqual(read_mountains(self.xml)[0], json.loads(gzip.decompress((self.root / "output" / FILE_NAME).read_bytes())))


if __name__ == "__main__":
    unittest.main()

"""日本百名山の対応と出典の保持を確かめる。"""

import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
GSI = ROOT / "points/gsi-peaks"
sys.path.insert(0, str(GSI))
from supplement_support import supplement_points
from document_metadata import metadata_maps


class GsiSupplementTests(unittest.TestCase):
    def test_hundred_tags_sources_and_optional_notes_with_other_corrections(self):
        osm = json.loads((ROOT / "points/osm-peaks/supplements.json").read_bytes())
        gsi = json.loads((GSI / "supplements.json").read_bytes())
        hundred = [p for p in gsi["points"] if "日本百名山" in p.get("tags", [])]
        self.assertEqual(len(hundred), 100)
        self.assertEqual(len({p["id"] for p in gsi["points"]}), len(gsi["points"]))
        for migrated in hundred:
            self.assertEqual(migrated["tags"], ["日本百名山"])
            self.assertNotIn("osmId", migrated)
            if "note" in migrated:
                self.assertIsInstance(migrated["note"], str)
        osm_source = metadata_maps(osm["metadata"])["sources"]["日本百名山"]
        gsi_source = metadata_maps(gsi["metadata"])["sources"]["日本百名山"]
        self.assertIn(osm_source["source"], gsi_source["source"])
        for key in ("license", "attribution"):
            self.assertEqual(osm_source[key], gsi_source[key])
        points = json.loads((GSI / "gsi-peaks-source.json").read_bytes())["points"]
        original_points = json.dumps(points, ensure_ascii=False)
        actual, sources = supplement_points(points)
        self.assertEqual(json.dumps(points, ensure_ascii=False), original_points)
        excluded = {p["id"] for p in gsi["points"] if p.get("exclude")}
        self.assertFalse(excluded & {p["id"] for p in actual})
        tagged = {p["id"] for p in actual if "日本百名山" in p.get("tags", [])}
        self.assertEqual(tagged, {p["id"] for p in hundred})
        self.assertTrue({"gsi-mountain-list:17", "gsi-mountain-list:100", "gsi-mountain-list:510",
                         "gsi-mountain-list:368", "gsi-vector:6a6975159e2a0acd6117cb58"} <= tagged)
        self.assertEqual(sources, metadata_maps(gsi["metadata"])["sources"])

    def test_missing_id_stops_generation(self):
        with self.assertRaises(ValueError):
            supplement_points([])

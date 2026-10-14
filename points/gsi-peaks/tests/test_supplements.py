"""日本百名山の対応と出典の保持を確かめる。"""

import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
GSI = ROOT / "points/gsi-peaks"
sys.path.insert(0, str(GSI))
from supplement_support import supplement_points


class GsiSupplementTests(unittest.TestCase):
    def test_hundred_tags_notes_and_order_are_preserved(self):
        osm = json.loads((ROOT / "points/osm-peaks/supplements.json").read_bytes())
        gsi = json.loads((GSI / "supplements.json").read_bytes())
        self.assertEqual(len(gsi["points"]), 100)
        self.assertEqual(len({p["id"] for p in gsi["points"]}), 100)
        for original, migrated in zip(osm["points"], gsi["points"]):
            self.assertEqual(original["note"], migrated["note"])
            self.assertEqual(original["tags"], migrated["tags"])
            self.assertNotIn("osmId", migrated)
        for key in ("source", "license", "attribution"):
            self.assertEqual(osm["metadata"]["sources"]["日本百名山"][key], gsi["metadata"]["sources"]["日本百名山"][key])
        points = json.loads((GSI / "gsi-peaks.json").read_bytes())["points"]
        actual, sources = supplement_points(points)
        self.assertEqual(actual, points)
        tagged = {p["id"] for p in actual if "日本百名山" in p.get("tags", [])}
        self.assertEqual(tagged, {p["id"] for p in gsi["points"]})
        self.assertTrue({"gsi-mountain-list:17", "gsi-mountain-list:100", "gsi-mountain-list:510",
                         "gsi-mountain-list:368", "gsi-vector:6a6975159e2a0acd6117cb58"} <= tagged)
        self.assertEqual(sources, json.loads((GSI / "snapshot.json").read_bytes())["supplementSources"])

    def test_missing_id_stops_generation(self):
        with self.assertRaises(ValueError):
            supplement_points([])

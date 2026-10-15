"""全部入り OSM の対象・出典と、保存 PBF からの再生成を確認する。"""

import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

from scripts.build_all import node_point, build


class AllOsmTests(unittest.TestCase):
    def test_named_non_peak_retains_all_classifications(self):
        point = node_point(1, 35, 139, {"name": "店", "amenity": "toilets", "shop": "hairdresser"})
        self.assertEqual(point["id"],"osm:1")
        self.assertNotIn("osmId",point)
        self.assertIsNone(point["type"])
        self.assertIsNone(point["elevationM"])
        self.assertEqual(point["provenance"], {"dataSource": "osm", "classification": "amenity=toilets; shop=hairdresser"})
        self.assertIsNone(node_point(2, 35, 139, {"name": " "}))
        peak = node_point(3, 35, 139, {"name": "山", "natural": "peak", "ele": "100 ft"})
        self.assertEqual(peak["id"],"osm:3")
        self.assertEqual(peak["type"], "peak")
        self.assertEqual(peak["elevationM"], 30.48)
        self.assertEqual(peak["provenance"]["elevationSource"], "osm")

    def test_peak_and_all_nodes_use_same_global_id(self):
        from scripts.build_data import read_mountains
        with tempfile.TemporaryDirectory() as folder:
            source=Path(folder)/"input.osm"
            source.write_text('<osm version="0.6"><node id="123456" lat="35" lon="139" timestamp="2026-10-01T00:00:00Z"><tag k="name" v="山"/><tag k="natural" v="peak"/></node></osm>',encoding="utf-8")
            peak=read_mountains(source)[0][0]
            all_point=node_point(123456,35,139,{"name":"山","natural":"peak"})
            self.assertEqual(peak["id"],"osm:123456")
            self.assertEqual(peak["id"],all_point["id"])
            self.assertNotIn("osmId",peak)
            self.assertNotIn("osmId",all_point)

    @unittest.skipUnless(importlib.util.find_spec('osmium'), 'Pyosmium が必要です')
    def test_pbf_nodes_only_and_peak_overrides(self):
        import osmium
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            pbf = root / 'input.osm.pbf'
            header = osmium.io.Header()
            header.set('osmosis_replication_timestamp', '2026-09-29T20:22:51Z')
            with osmium.SimpleWriter(str(pbf), header=header) as writer:
                writer.add_node(osmium.osm.mutable.Node(id=1, location=(139, 35), tags={'name': '山', 'natural': 'peak', 'ele': '100'}))
                writer.add_node(osmium.osm.mutable.Node(id=2, location=(140, 36), tags={'name:ja': '駅', 'railway': 'station'}))
                writer.add_node(osmium.osm.mutable.Node(id=3, location=(140, 36)))
                writer.add_way(osmium.osm.mutable.Way(id=4, nodes=[1, 2], tags={'name': '道'}))
            Path(str(pbf)+'.source.json').write_text(json.dumps({'url': 'https://download.geofabrik.de/asia/japan-260929.osm.pbf', 'sizeBytes': pbf.stat().st_size, 'md5': hashlib.md5(pbf.read_bytes()).hexdigest()}), encoding='utf-8')
            peaks = root/'peaks.json'
            peak = node_point(1, 35, 139, {'name': '補足済みの山', 'natural': 'peak', 'ele': '100'})
            peaks.write_text(json.dumps([peak]), encoding='utf-8')
            report = build(pbf, root/'out', peaks)
            rows = json.loads((root/'out/osm-all-points.json').read_bytes())['points']
            self.assertEqual([p['id'] for p in rows], ['osm:1', 'osm:2'])
            self.assertEqual(rows[0]['name'], '補足済みの山')
            self.assertEqual(report['peakCount'], 1)
            self.assertEqual(gzip.decompress((root/'out/osm-all-points.json.gz').read_bytes()), (root/'out/osm-all-points.json').read_bytes())

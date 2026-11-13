"""公式概念図の概略提供範囲と、その再生成・出典保持を確かめる。"""

from copy import deepcopy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from source_coverage import DATA, trace_coverage, generated_coverage, record_source, SOURCE_NAME
from coverage_geometry import check_points
from shapely.geometry import shape, Point


class SourceCoverageTests(unittest.TestCase):
    def setUp(self):
        self.trace = json.loads((DATA / "coverage-trace.json").read_bytes())

    def test_official_trace_keeps_northern_territories_and_offshore_without_foreign_land(self):
        coverage, record = generated_coverage()
        area = shape(coverage["geometry"])
        for name, lon, lat in [("東京湾",139.8,35.4), ("沖合",140,30), ("択捉島周辺",148,45),
                              ("北方領土東方の海域",149,44), ("南鳥島",153.98,24.29), ("沖ノ鳥島",136.074,20.423)]:
            with self.subTest(name=name):
                self.assertTrue(area.covers(Point(lon,lat)))
        for name, lon, lat in [("ハルビン",126.63,45.75), ("長春",125.32,43.82),
                              ("釜山",129.07,35.18), ("台北",121.56,25.03), ("樺太",142.7,47)]:
            with self.subTest(name=name):
                self.assertFalse(area.covers(Point(lon,lat)))
        points = json.loads((DATA / "gsi-peaks-source.json").read_bytes())["points"]
        check_points(coverage["geometry"], points)
        self.assertTrue(record["approximate"])
        self.assertEqual(record["kind"], "jcg-concept-map-trace")
        self.assertIn("海上保安庁", record["metadataSources"][0]["attribution"])

    def test_saved_geometry_is_reproducible_from_trace_without_source_image(self):
        coverage, alignment = trace_coverage(self.trace)
        self.assertEqual(coverage, generated_coverage()[0])
        self.assertLess(alignment["maxControlResidualPixels"], 4)
        self.assertEqual(len(coverage["geometry"]["coordinates"]), 2)
        self.assertEqual(len(coverage["geometry"]["coordinates"][0]), 2)

    def test_extended_continental_shelf_is_excluded(self):
        coverage, alignment = trace_coverage(self.trace)
        # 元図で桃色の延長大陸棚に当たる画素を、同じ位置合わせで照合する。
        import math
        x,y = alignment["xCoefficients"],alignment["yCoefficients"]
        for px,py in [(610,850),(940,900)]:
            lon=(px-x[1])/x[0]
            lat=math.degrees(math.atan(math.sinh((py-y[1])/y[0])))
            self.assertFalse(shape(coverage["geometry"]).covers(Point(lon,lat)))

    def test_invalid_pixels_rings_and_alignment_are_rejected(self):
        for mutation in [
            lambda t:t["polygons"][0]["exteriorPixels"].__setitem__(0,[-1,100]),
            lambda t:t["polygons"][0]["exteriorPixels"].pop(),
            lambda t:t["controlPoints"][0].__setitem__("pixel",[100,100]),
            lambda t:t["polygons"][1]["ellipsePixels"].__setitem__("radius",[0,100])]:
            trace=deepcopy(self.trace);mutation(trace)
            with self.assertRaises(ValueError):trace_coverage(trace)

    def test_saved_geometry_and_trace_hashes_are_checked(self):
        with tempfile.TemporaryDirectory() as folder:
            data=Path(folder)
            for name in ["coverage.geojson","coverage-source.json","coverage-trace.json"]:
                (data/name).write_bytes((DATA/name).read_bytes())
            self.assertEqual(generated_coverage(data=data), generated_coverage())
            original=(data/"coverage.geojson").read_bytes()
            (data/"coverage.geojson").write_bytes(original+b" ")
            with self.assertRaisesRegex(ValueError,"SHA-256"):generated_coverage(data=data)
            (data/"coverage.geojson").write_bytes(original)
            (data/"coverage-trace.json").write_bytes(b"{}")
            with self.assertRaisesRegex(ValueError,"トレース"):generated_coverage(data=data)

    def test_record_keeps_points_adds_attribution_and_never_copies_image(self):
        with tempfile.TemporaryDirectory() as folder:
            directory=Path(folder)
            for name in ["gsi-peaks-source.json","snapshot.json"]:
                (directory/name).write_bytes((DATA/name).read_bytes())
            before=json.loads((directory/"gsi-peaks-source.json").read_bytes())["points"]
            record_source(directory)
            body=json.loads((directory/"gsi-peaks-source.json").read_bytes())
            self.assertEqual(before,body["points"])
            names={s["name"] for s in body["metadata"]["sources"]}
            self.assertIn(SOURCE_NAME,names)
            self.assertFalse(any(p.suffix==".png" for p in directory.iterdir()))
            snapshot=json.loads((directory/"snapshot.json").read_bytes())
            self.assertEqual(hashlib.sha256((directory/"gsi-peaks-source.json").read_bytes()).hexdigest(),snapshot["inputSha256"])
            self.assertEqual(snapshot["sources"],body["metadata"]["sources"])
            before_raw = (directory/"gsi-peaks-source.json").read_bytes()
            record_source(directory)
            self.assertEqual(before_raw, (directory/"gsi-peaks-source.json").read_bytes())
            image=directory/"unrelated-image.bin";image.write_bytes(b"broken")
            with self.assertRaisesRegex(ValueError,"元画像"):record_source(directory,source_image=image)

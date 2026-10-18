"""合成地形で尾根・鞍部・欠損・境界を検証する。通信は使わない。"""

import io
import math
import gzip
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import numpy as np
from PIL import Image

import ridges
import reference


class DemTests(unittest.TestCase):
    def test_decode_signed_and_missing(self):
        image = np.zeros((256, 256, 3), dtype=np.uint8)
        image[0, :4] = [[0, 0, 100], [255, 255, 156], [128, 0, 0], [0, 0, 0]]
        data = io.BytesIO()
        Image.fromarray(image).save(data, format="PNG")
        values = ridges.decode_png(data.getvalue())[0, :4]
        np.testing.assert_allclose(values, [1, -1, np.nan, 0], equal_nan=True)

    def test_tile_coordinates_round_trip(self):
        for lon, lat in [(138.37, 35.97), (140, -30)]:
            x, y = ridges.tile_position(lon, lat, 14)
            a, b = ridges.lon_lat(x*256, y*256, 14)
            self.assertAlmostEqual(lon, a)
            self.assertAlmostEqual(lat, b)

    def test_missing_is_not_filled_by_smoothing(self):
        dem = np.ones((5, 5)) * 100
        dem[2, 2] = np.nan
        result = ridges.smooth_dem(dem, 2)
        self.assertTrue(math.isnan(result[2, 2]))
        self.assertEqual(result[1, 1], 100)

    def test_missing_cannot_be_interpolated_across(self):
        dem = np.array([[10.0, np.nan], [20.0, 30.0]])
        self.assertTrue(math.isnan(ridges.sample_height(dem, (0.5, 0.5))))
        self.assertEqual(ridges.sample_height(dem, (0, 0)), 10)

    def test_large_region_fails_before_network(self):
        with mock.patch.object(ridges, "read_tile") as read:
            with self.assertRaisesRegex(ValueError, "64"):
                ridges.load_dem([130, 30, 140, 40], 14, Path("unused"), False)
            read.assert_not_called()

    def test_offline_missing_tile_does_not_fetch(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch("urllib.request.urlopen") as request:
            with self.assertRaisesRegex(ValueError, "保存されていない"):
                ridges.read_tile(14, 1, 1, Path(directory), True)
            request.assert_not_called()


class ExtractionTests(unittest.TestCase):
    def test_flow_strictly_descends_and_never_wraps(self):
        dem = np.array([[5.0, 3, 1], [8, np.nan, 0], [9, 7, 4]])
        route = ridges.flow_routes(dem)
        for source, target in enumerate(route):
            if source == target:
                continue
            self.assertLess(dem.ravel()[target], dem.ravel()[source])
            self.assertLessEqual(abs(source//3-target//3), 1)
            self.assertLessEqual(abs(source%3-target%3), 1)

    def test_watershed_follows_ridge_through_saddle(self):
        y, x = np.mgrid[-12:13, -15:16]
        # 東西に並ぶ二峰の間に鞍部があり、南北へ斜面が下る地形。
        dem = 1000 + 50*np.exp(-((x-8)/4)**2) + 50*np.exp(-((x+8)/4)**2) - y*y
        paths = ridges.trace_paths(ridges.watershed_edges(dem, 1))
        crossing = [path for path in paths if min(p[0] for p in path) <= 8 and max(p[0] for p in path) >= 22
                    and all(abs(p[1]-12) <= 1 for p in path)]
        self.assertTrue(crossing, "主稜線が二峰の間で切れないこと")
        middle = ridges.sample_height(dem, (15, 12.5))
        peak = ridges.sample_height(dem, (23, 12.5))
        self.assertLess(middle, peak-30)

    def test_inverse_flow_accumulates_toward_peak(self):
        y, x = np.mgrid[-5:6, -5:6]
        dem = 100-x*x-2*y*y
        edges = ridges.inverse_flow_edges(dem, 3)
        self.assertTrue(edges)
        for a, b in edges:
            self.assertGreater(dem[b[1], b[0]], dem[a[1], a[0]])

    def test_flat_ground_has_no_inverse_routes(self):
        self.assertEqual(ridges.inverse_flow_edges(np.ones((5, 5)), 1), [])

    def test_trace_preserves_branches_and_cycles(self):
        edges = [((0, 0), (1, 0)), ((1, 0), (2, 0)), ((1, 0), (1, 1)),
                 ((5, 5), (6, 5)), ((6, 5), (6, 6)), ((6, 6), (5, 5))]
        paths = ridges.trace_paths(edges)
        recovered = {tuple(sorted((a, b))) for path in paths for a, b in zip(path, path[1:])}
        self.assertEqual(recovered, {tuple(sorted(edge)) for edge in edges})
        self.assertEqual(len(paths), 4)

    def test_adjacent_crops_share_exact_boundary_points(self):
        paths = [[(0, 0), (2, 2), (4, 0)]]
        left = ridges.clip_paths(paths, (0, 0, 1, 4))
        right = ridges.clip_paths(paths, (1, 0, 4, 4))
        self.assertEqual(left[0][-1], right[0][0])
        self.assertEqual(left[0][-1], (1, 1))

    def test_reentry_is_split_and_tangent_does_not_join(self):
        paths = [[(0, 1), (2, 1), (2, 3), (0, 3)]]
        result = ridges.clip_paths(paths, (0, 0, 1, 4))
        self.assertEqual(result, [[(0, 1), (1, 1)], [(1, 3), (0, 3)]])

    def test_simplification_preserves_saddle_and_endpoint(self):
        points = [(0, 0, 100), (1, 0, 100), (2, 0, 60), (3, 0, 100), (4, 0, 100)]
        result = ridges.simplify(points, 10, 2)
        self.assertIn((2, 0, 60), result)
        self.assertEqual(result[0], points[0])
        self.assertEqual(result[-1], points[-1])

    def test_simplification_bounds_horizontal_deviation(self):
        points = [(0, 0, 0), (1, 10, 0), (2, 0, 0)]
        self.assertEqual(ridges.simplify(points, 1, 1), points)


class PipelineTests(unittest.TestCase):
    def test_mesh_does_not_cover_missing_sample_between_vertices(self):
        dem = np.ones((9, 9))*100
        dem[2, 2] = np.nan
        with tempfile.TemporaryDirectory() as directory:
            args = ridges.parser().parse_args(["--bbox", "138", "35", "139", "36", "--output", directory])
            with mock.patch.object(ridges, "load_dem", return_value=(dem, (0, 0), (0, 0, 8, 8), [])), \
                    mock.patch("builtins.print"):
                ridges.run(args)
            html = (args.output / "preview.html").read_text(encoding="utf-8")
            payload = json.loads(html.split("const data = ", 1)[1].split(";\n", 1)[0])
            self.assertEqual(payload["heightfield"]["shape"], [9, 9])
            import base64
            heights = np.frombuffer(gzip.decompress(base64.b64decode(payload["heightfield"]["values"])), dtype="<f4")
            self.assertTrue(np.isnan(heights.reshape(9, 9)[2, 2]))
            self.assertNotIn("indices", payload)


    def test_export_is_offline_readable_and_repeatable(self):
        y, x = np.mgrid[-12:13, -15:16]
        dem = 1000 + 50*np.exp(-((x-8)/4)**2) + 50*np.exp(-((x+8)/4)**2) - y*y
        with tempfile.TemporaryDirectory() as directory:
            args = ridges.parser().parse_args(["--bbox", "138.34", "35.95", "138.39", "36.01",
                                               "--output", directory, "--min-cells", "1", "--min-length", "1"])
            with mock.patch.object(ridges, "load_dem", return_value=(dem, (0, 0), (1, 1, 29, 23), [])), \
                    mock.patch("builtins.print"), mock.patch("urllib.request.urlopen") as network:
                ridges.run(args)
                first = (args.output / "watershed.geojson.gz").read_bytes()
                ridges.run(args)
                self.assertEqual(first, (args.output / "watershed.geojson.gz").read_bytes())
                network.assert_not_called()
            collection = json.loads(gzip.decompress(first))
            self.assertTrue(collection["features"])
            for feature in collection["features"]:
                self.assertTrue(all(len(p) == 3 for p in feature["geometry"]["coordinates"]))
            with np.load(args.output / "terrain.npz", allow_pickle=False) as archive:
                self.assertEqual(archive["heights"].shape, (23, 29))
                self.assertEqual(json.loads(str(archive["metadata"]))["zoom"], 14)
            html = (args.output / "preview.html").read_text(encoding="utf-8")
            self.assertNotIn("/*DATA*/", html)
            self.assertNotIn("NaN", html)
            payload = json.loads(html.split("const data = ", 1)[1].split(";\n", 1)[0])
            self.assertFalse(payload.get("dynamicLOD",False))
            self.assertEqual(payload["heightfield"]["shape"], [23, 29])
            self.assertNotIn("mesh", payload)
            self.assertTrue(json.loads((args.output / "report.json").read_bytes())["experimental"])
            cache = args.output / "reference-cache"
            with mock.patch("urllib.request.urlopen") as network:
                with self.assertRaisesRegex(ValueError, "保存されていない標準地図"):
                    reference.run(args.output, cache, True)
                tile = cache / "14/0/0.png"
                tile.parent.mkdir(parents=True)
                Image.new("RGB", (256, 256), "white").save(tile)
                with mock.patch("builtins.print"):
                    reference.run(args.output, cache, True)
                network.assert_not_called()
            self.assertIn("data:image/png;base64,", (args.output / "reference.html").read_text(encoding="utf-8"))
            self.assertEqual(len(json.loads((args.output / "reference-sources.json").read_bytes())), 1)

    def test_all_missing_region_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            args = ridges.parser().parse_args(["--bbox", "138", "35", "139", "36", "--output", directory])
            with mock.patch.object(ridges, "load_dem", return_value=(np.full((5, 5), np.nan), (0, 0), (0, 0, 4, 4), [])):
                with self.assertRaisesRegex(ValueError, "有効な標高"):
                    ridges.run(args)


if __name__ == "__main__":
    unittest.main()

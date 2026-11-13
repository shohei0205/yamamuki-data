"""対象範囲の和集合が、地点の密度や外接矩形に左右されないことを確かめる。"""

import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from coverage_geometry import tile_geometry, union_geometry, parse_poly, check_points
from coverage_metadata import validate_coverage
from shapely.geometry import shape, Point


class CoverageGeometryTests(unittest.TestCase):
    def test_tiles_keep_holes_islands_and_empty_source_tiles(self):
        tiles = [(3, 0, 0), *((3, x, y) for x in range(3, 6) for y in range(3, 6) if (x, y) != (4, 4))]
        geometry = tile_geometry(tiles)
        area = shape(geometry)
        self.assertEqual(len(area.geoms), 2)
        self.assertEqual(sum(len(p.interiors) for p in area.geoms), 1)
        validate_coverage({"geometry": geometry})
        self.assertEqual(geometry, tile_geometry(list(reversed(tiles)) + tiles))

    def test_mixed_zooms_and_world_are_not_approximated_by_bounds(self):
        parent = tile_geometry([(2, 1, 1)])
        children = [(3, x, y) for x in (2, 3) for y in (2, 3)]
        self.assertEqual(parent, tile_geometry(children))
        self.assertEqual(parent, tile_geometry([(2, 1, 1), *children]))
        validate_coverage({"geometry": tile_geometry([(0, 0, 0)])})
        for bad in [[], [(31, 0, 0)], [(2, 4, 0)], [(True, 0, 0)]]:
            with self.subTest(tiles=bad), self.assertRaises(ValueError):
                tile_geometry(bad)

    def test_poly_supports_holes_and_multiple_outer_rings(self):
        content = b"test\n1\n130 30\n140 30\n140 40\n130 40\nEND\n!2\n133 33\n137 33\n137 37\n133 37\nEND\n3\n145 40\n150 40\n150 45\n145 45\nEND\nEND\n"
        geometry = parse_poly(content)
        area = shape(geometry)
        self.assertFalse(area.covers(Point(135, 35)))
        self.assertTrue(area.covers(Point(147, 42)))
        self.assertFalse(area.covers(Point(143, 42)))
        validate_coverage({"geometry": geometry})
        self.assertTrue(shape(union_geometry([geometry, geometry])).equals(area))
        for bad in [b"", content[:-5], content + b"extra", b"test\nEND\n"]:
            with self.subTest(content=bad), self.assertRaises(ValueError):
                parse_poly(bad)

    def test_fallback_points_must_be_covered_by_source_scope(self):
        geometry = parse_poly(b"test\n1\n130 30\n140 30\n140 40\n130 40\nEND\nEND\n")
        check_points(geometry, [{"id": "inside", "latitude": 35, "longitude": 139}])
        with self.assertRaisesRegex(ValueError, "outside"):
            check_points(geometry, [{"id": "outside", "latitude": 35, "longitude": 145}])

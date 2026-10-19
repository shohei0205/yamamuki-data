"""PNG入力から出力メッシュまでを、生成器と別の格子走査で照合する。"""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / 'build/terrain-rust-target/release/terrain-adaptive-tin.exe'


def raster_error(grid, mesh):
    points = np.asarray(mesh['vertices'])
    predicted = np.full(grid.shape, np.nan)
    for face in mesh['triangles']:
        a, b, c = points[face]
        low = np.maximum(0, np.ceil(np.minimum.reduce([a[:2], b[:2], c[:2]]))).astype(int)
        high = np.minimum(np.array(grid.shape[::-1])-1, np.floor(np.maximum.reduce([a[:2], b[:2], c[:2]]))).astype(int)
        y, x = np.mgrid[low[1]:high[1]+1, low[0]:high[0]+1]
        area = (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
        assert area > 0
        u = ((x-a[0])*(c[1]-a[1])-(y-a[1])*(c[0]-a[0]))/area
        v = ((b[0]-a[0])*(y-a[1])-(b[1]-a[1])*(x-a[0]))/area
        mask = (u >= -1e-10) & (v >= -1e-10) & (u+v <= 1+1e-10)
        predicted[y[mask], x[mask]] = (a[2]+u*(b[2]-a[2])+v*(c[2]-a[2]))[mask]
    assert np.isfinite(predicted).all(), '標高格子に未被覆の点があります'
    return float(np.abs(predicted-grid).max())


class AdaptiveTinCLI(unittest.TestCase):
    def run_mesh(self, grid, *options):
        self.assertTrue(BINARY.exists(), 'build.ps1 -AdaptiveTin を先に実行してください')
        with tempfile.TemporaryDirectory(dir=ROOT / 'build', prefix='adaptive-tin-test-') as directory:
            source, output = Path(directory)/'height.png', Path(directory)/'mesh.json'
            Image.fromarray(grid).save(source)
            process = subprocess.run([str(BINARY), '--input', str(source), '--output', str(output), *options],
                                     check=True, capture_output=True, text=True, encoding='utf-8')
            return json.loads(output.read_text(encoding='utf-8')), json.loads(process.stdout)

    def test_gray16_rough_terrain_meets_error_and_covers_entire_grid(self):
        y, x = np.indices((33, 41))
        grid = (1000+250*np.sin(x/4)*np.cos(y/5)+(x*31+y*17)%41).astype(np.uint16)
        mesh, report = self.run_mesh(grid, '--max-vertices', '1300', '--max-error', '4')
        error = raster_error(grid.astype(float), mesh)
        self.assertLessEqual(error, 4+1e-8)
        self.assertAlmostEqual(error, mesh['maxError'], places=8)
        self.assertTrue(report['toleranceReached'])
        self.assertLessEqual(len(mesh['vertices']), 1300)

    def test_gray8_scaling_and_vertex_limit(self):
        grid = np.zeros((17, 17), np.uint8)
        grid[8, 8] = 255
        mesh, _ = self.run_mesh(grid, '--max-vertices', '4', '--max-error', '1', '--scale', '2', '--offset', '-100')
        self.assertEqual(len(mesh['vertices']), 4)
        self.assertEqual(mesh['maxError'], 510)
        self.assertEqual(mesh['stopReason'], 'maxVertices')
        self.assertFalse(mesh['toleranceReached'])
        self.assertEqual(raster_error(grid.astype(float)*2-100, mesh), 510)

    def test_planar_grid_and_output_spacing(self):
        y, x = np.indices((17, 23))
        grid = (2*x+3*y+100).astype(np.uint16)
        mesh, _ = self.run_mesh(grid, '--spacing', '10', '--max-error', '0.000001')
        self.assertEqual(len(mesh['vertices']), 4)
        self.assertEqual(len(mesh['triangles']), 2)
        self.assertEqual(mesh['vertices'][2], [220, 160, float(grid[-1, -1])])

    def test_minimum_vertices_are_independent_of_error_tolerance(self):
        y, x = np.indices((17, 17))
        grid = np.random.default_rng(123).integers(0, 151, size=(17, 17), dtype=np.uint16)
        coarse, _ = self.run_mesh(grid, '--max-error', '1000')
        detailed, _ = self.run_mesh(grid, '--max-error', '1000', '--min-vertices', '128')
        self.assertEqual(len(coarse['vertices']), 4)
        self.assertEqual(len(detailed['vertices']), 128)
        self.assertTrue(detailed['minimumVerticesReached'])
        self.assertAlmostEqual(raster_error(grid.astype(float), detailed), detailed['maxError'], places=8)
        self.assertLess(detailed['maxError'], coarse['maxError'])


if __name__ == '__main__':
    unittest.main()

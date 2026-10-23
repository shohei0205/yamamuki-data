"""可変サイズタイルの境界接続を合成地形で確認する。"""
import unittest

import numpy as np
from scipy.spatial import Delaunay

from hierarchical_variable_tiles import Registry, SIDE, cross, partition, stitch, validate


def faces(reg):
    result = []
    for f in Delaunay(np.asarray(reg.points)[:,:2]).simplices:
        f = tuple(map(int,f))
        result.append(f if cross(*(reg.points[i] for i in f))>0 else tuple(reversed(f)))
    return result


class VariableTilesTest(unittest.TestCase):
    def test_mixed_boundary_and_inherited_vertices(self):
        yy,xx = np.mgrid[:513,:513]
        reg = Registry(xx*.4+yy*.2)
        for y in range(3):
            for x in range(3):
                reg.add([x*SIDE//2,y*SIDE//2,0])
        parent = [p[:] for p in reg.points]
        coarse = partition(reg,faces(reg),SIDE//2)
        for y in range(5):
            for x in range(5):
                reg.add([x*SIDE//4,y*SIDE//4,0])
        fine = partition(reg,faces(reg),SIDE//4)
        self.assertEqual(reg.points[:len(parent)],parent)
        self.assertFalse(validate(reg.points,fine)[1])
        leaves = [(SIDE//2,k,f) for k,f in coarse.items() if k[0]==0]
        leaves += [(SIDE//4,k,f) for k,f in fine.items() if k[0]>=2]
        self.assertGreater(len(validate(reg.points,{str(i):f for i,(_,_,f) in enumerate(leaves)})[1]),0)
        points,tiles,_ = stitch(reg,leaves)
        stats,bad = validate(points,tiles)
        self.assertFalse(bad)
        self.assertEqual(stats['doubledArea'],2*SIDE*SIDE)
        self.assertEqual(points[:len(parent)],parent)

    def test_cut_points_sample_dem(self):
        grid = np.zeros((513,513));grid[256,256] = 100
        reg = Registry(grid)
        i = reg.add([SIDE//2,SIDE//2,-999])
        self.assertEqual(reg.points[i][2],100)
        self.assertEqual(reg.add([SIDE//2,SIDE//2,999]),i)
        self.assertEqual(reg.points[i][2],100)


if __name__ == '__main__':
    unittest.main()

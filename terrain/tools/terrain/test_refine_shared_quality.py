"""広域メッシュの形状補修を小さな標高格子で検証する。"""
import unittest
import numpy as np
from refine_shared_quality import refine,triangulate,angles

class QualityTests(unittest.TestCase):
    def test_budget_boundary_and_shape(self):
        grid=np.zeros((65,65));p=np.array([[0,0,0],[64,0,0],[64,64,0],[0,64,0],[1,30,0],[2,32,0],[3,34,0],[32,32,0]],float)
        out,f,r=refine(p,np.arange(len(p)),grid,32,reserve=.2)
        self.assertLessEqual(len(out),32)
        self.assertTrue(all(tuple(v) in set(map(tuple,out)) for v in p[:4]))
        t=out[f,:2];cross=np.linalg.det(np.stack([t[:,1]-t[:,0],t[:,2]-t[:,0]],axis=2))
        self.assertTrue((cross>0).all());self.assertAlmostEqual(cross.sum()/2,4096)
        self.assertTrue((out[:,2]==0).all());self.assertGreater(r['qualityInserted'],0)
        self.assertGreater(angles(out,f).min(),angles(p,triangulate(p)).min())
    def test_heights_come_from_grid(self):
        y,x=np.mgrid[:33,:33];grid=x*.3+y*.2
        p=np.array([[0,0,grid[0,0]],[32,0,grid[0,32]],[32,32,grid[32,32]],[0,32,grid[32,0]],[1,16,grid[16,1]]])
        out,faces,report=refine(p,np.arange(5),grid,32)
        np.testing.assert_allclose(out[:,2],grid[out[:,1].astype(int),out[:,0].astype(int)])
        self.assertLessEqual(len(out),32)
    def test_flat_square_needs_no_extra_points(self):
        grid=np.zeros((17,17));p=np.array([[0,0,0],[16,0,0],[16,16,0],[0,16,0]],float)
        out,faces,report=refine(p,np.arange(4),grid,32)
        self.assertEqual(len(out),4);self.assertEqual(report['qualityInserted'],0)
    def test_outer_boundary_over_budget_is_rejected(self):
        grid=np.zeros((17,17));p=np.array([[0,0,0],[16,0,0],[16,16,0],[0,16,0]],float)
        with self.assertRaises(ValueError):refine(p,np.arange(4),grid,3)
    def test_deterministic(self):
        grid=np.zeros((17,17));p=np.array([[0,0,0],[16,0,0],[16,16,0],[0,16,0],[1,8,0]],float)
        a=refine(p,np.arange(5),grid,20);b=refine(p,np.arange(5),grid,20)
        np.testing.assert_array_equal(a[0],b[0]);np.testing.assert_array_equal(a[1],b[1])

if __name__=='__main__':unittest.main()

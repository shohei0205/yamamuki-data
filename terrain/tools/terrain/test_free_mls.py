"""自由配置MLSの面の再現、連続性、保存形式を検証する。"""
import json,sqlite3,unittest,zlib
import numpy as np
from scipy.stats import qmc
from free_mls import operator,SPAN,LocalSupport
class MLSTests(unittest.TestCase):
    def setUp(self):
        self.points=(qmc.Sobol(2,scramble=True,seed=39).random_base2(9)-.5)*SPAN
        self.query=(qmc.Sobol(2,scramble=True,seed=42).random_base2(8)-.5)*SPAN
    def test_plane_and_constant_are_preserved(self):
        a=operator(self.points,self.query,7000);np.testing.assert_allclose(np.asarray(a.sum(axis=1)).ravel(),1,atol=1e-8)
        z=600+.1*self.points[:,0]-.05*self.points[:,1];expected=600+.1*self.query[:,0]-.05*self.query[:,1]
        np.testing.assert_allclose(a@z,expected,atol=1e-4)
    def test_no_crease_when_a_point_enters_support(self):
        radius=7000;center=self.points[200];q=center+[radius,0]
        if np.max(abs(q))>15000:q=center-[radius,0]
        eps=.05;xy=np.array([q+[-eps,0],q,q+[eps,0]])
        z=200*np.sin(self.points[:,0]/3000)+100*np.cos(self.points[:,1]/2000);h=operator(self.points,xy,radius)@z
        self.assertLess(abs((h[2]-h[1])/eps-(h[1]-h[0])/eps),.001)
    def test_local_support_plane_and_smooth_radius(self):
        support=LocalSupport(self.points);r=support(self.query)
        self.assertTrue(np.isfinite(r).all());self.assertTrue((r>0).all())
        a=operator(self.points,self.query,support,curvature=1e-5);z=600+.1*self.points[:,0]-.05*self.points[:,1]
        np.testing.assert_allclose(a@z,600+.1*self.query[:,0]-.05*self.query[:,1],atol=1e-4)
        q=np.array([[-.05,100],[0,100],[.05,100]]);v=support(q)
        self.assertLess(abs((v[2]-v[1])/.05-(v[1]-v[0])/.05),1e-4)

    def test_radius_presmoothing_can_be_disabled(self):
        from scipy.spatial import cKDTree
        support=LocalSupport(self.points,pre_smooth=False);smoothed=LocalSupport(self.points,pre_smooth=True)
        grid=np.array([[0.,0.],[1000.,2000.],[-5000.,7500.]])
        raw=np.maximum(2*SPAN/np.sqrt(len(self.points)),1.25*cKDTree(self.points).query(grid,k=12)[0][:,-1])
        np.testing.assert_allclose(support(grid),raw,rtol=1e-9,atol=1e-6)
        self.assertGreater(np.max(abs(support(grid)-smoothed(grid))),.01)
        a=operator(self.points,self.query,support,curvature=1e-5);z=600+.1*self.points[:,0]-.05*self.points[:,1]
        np.testing.assert_allclose(a@z,600+.1*self.query[:,0]-.05*self.query[:,1],atol=1e-4)

    def test_saved_points_are_free_and_finite(self):
        with sqlite3.connect('build/free-mls-v3/mls.sqlite3') as db:rows=db.execute('SELECT * FROM surfaces').fetchall()
        self.assertEqual(len(rows),21)
        for region,count,raw,blob in rows:
            info=json.loads(raw);data=np.frombuffer(zlib.decompress(blob),'<f4');radius=float(data[0]);points=data[1:].reshape(-1,3)
            self.assertEqual(len(points),count);self.assertEqual(len(blob),info['storedBytes']);self.assertTrue(np.isfinite(points).all());self.assertGreater(radius,0)
            self.assertGreater(len(np.unique(points[:,0])),count*.95);self.assertGreater(len(np.unique(points[:,1])),count*.95)
            support=LocalSupport(points[:,:2],scale=radius) if info.get('supportMode') else radius
            h=operator(points[:,:2],self.query,support,curvature=info.get('curvaturePenalty',1e-3))@points[:,2];self.assertTrue(np.isfinite(h).all())
if __name__=='__main__':unittest.main()

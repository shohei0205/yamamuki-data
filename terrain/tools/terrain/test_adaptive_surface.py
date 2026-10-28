"""自由配置した三次曲面の復元と接続を確かめる。"""
import json
import sqlite3
import unittest
import zlib
from pathlib import Path
import numpy as np
import adaptive_surface as a

class AdaptiveTests(unittest.TestCase):
    def test_thin_plate_matches_reference(self):
        from scipy.interpolate import RBFInterpolator
        rng=np.random.default_rng(132)
        xy=rng.uniform(-16000,16000,(120,2));z=1000*np.exp(-np.sum((xy/7000)**2,axis=1))+.01*xy[:,0]
        points=np.column_stack((xy,z)).astype('<f4');base,features=a.thin_plate(points)
        query=rng.uniform(-14000,14000,(200,2))
        reference=RBFInterpolator(points[:,:2].astype(float)/a.SPAN,points[:,2].astype(float),kernel='thin_plate_spline',smoothing=1e-7,degree=1)(query/a.SPAN)
        np.testing.assert_allclose(a.thin_plate_values(base,features,query),reference,atol=.03)
        grid=a.thin_plate_grid(base,features,n=32);axis=np.linspace(-a.SPAN/2,a.SPAN/2,33);xx,yy=np.meshgrid(axis,axis);query_grid=np.column_stack((xx.ravel(),yy.ravel()))
        np.testing.assert_allclose(grid[:,0],a.thin_plate_values(base,features,query_grid),atol=.002)
        for axis_id in range(2):
            delta=np.eye(2)[axis_id]*.01;gradient=(a.thin_plate_values(base,features,query_grid+delta)-a.thin_plate_values(base,features,query_grid-delta))/.02
            np.testing.assert_allclose(grid[:,axis_id+1],gradient,atol=1e-5)
        plane=np.column_stack((xy,200+.03*xy[:,0]-.02*xy[:,1])).astype('<f4');base,features=a.thin_plate(plane)
        np.testing.assert_allclose(a.thin_plate_values(base,features,query),200+.03*query[:,0]-.02*query[:,1],atol=.002)
        with self.assertRaises(ValueError):a.thin_plate(np.zeros((2049,3)))

    def test_plane(self):
        rng=np.random.default_rng(37)
        xy=np.vstack((np.array([[-16000,-16000],[16000,-16000],[16000,16000],[-16000,16000]]),rng.uniform(-16000,16000,(100,2))))
        points=np.column_stack((xy,100+.02*xy[:,0]-.03*xy[:,1])).astype('<f4')
        t,_,_=a.cubic_packet(points)
        uv=np.array([[.2,.3],[.7,.1]])
        tri=t[:,:6].reshape(-1,3,2).astype(float)
        q=tri[:,2,None]+uv[None,:,0,None]*(tri[:,0,None]-tri[:,2,None])+uv[None,:,1,None]*(tri[:,1,None]-tri[:,2,None])
        actual=t[:,6:]@a.cubic_terms(uv).T
        np.testing.assert_allclose(actual,100+.02*q[:,:,0]-.03*q[:,:,1],atol=.001)

    def test_saved_models_and_polynomial_reference(self):
        path=Path('build/adaptive-free-surface-v6/adaptive.sqlite3')
        with sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True) as db:
            rows=db.execute('select region,info,model from surfaces').fetchall()
        self.assertEqual(len(rows),12)
        self.report=[]
        for region,info,blob in rows:
            info=json.loads(info);points=np.frombuffer(zlib.decompress(blob),'<f4').reshape(-1,3)
            self.assertEqual(len(points),info['controls']);self.assertEqual(info['corrections'],0)
            self.assertLess(info['peakError'],1e-6)
            t,lookup,indices=a.cubic_packet(points)
            self.assertLessEqual(lookup[:,1].max(),1024)
            self.assertTrue(np.all(np.isfinite(t)))
            ids=np.linspace(0,len(t)-1,min(200,len(t))).astype(int);sub=t[ids,:6].reshape(-1,3,2).astype(float)
            uv=np.array([[.2,.3],[.6,.2],[.1,.7]])
            q=sub[:,2,None]+uv[None,:,0,None]*(sub[:,0,None]-sub[:,2,None])+uv[None,:,1,None]*(sub[:,1,None]-sub[:,2,None])
            curve=a.CloughTocher2DInterpolator(points[:,:2],points[:,2],tol=1e-8,maxiter=1000)
            reference=curve(q.reshape(-1,2)).reshape(-1,3);actual=t[ids,6:]@a.cubic_terms(uv).T
            error=float(np.max(abs(actual-reference)));self.assertLess(error,.02)
            # 共有辺の両側へ近づいたとき、高さ・一次微分が連続であること。
            tri=curve.tri;valid=np.argwhere(tri.neighbors>=0)[::max(1,len(tri.simplices)//40)]
            max_gradient_jump=0.
            for k,opposite in valid:
                edge=np.delete(tri.simplices[k],opposite);m=tri.points[edge].mean(axis=0);d=tri.points[edge[1]]-tri.points[edge[0]];normal=np.array([-d[1],d[0]])/np.linalg.norm(d)
                grads=[]
                for side in [-1,1]:
                    x=m+normal*side*.001;eps=.0001
                    grads.append(np.array([(curve(x+np.eye(2)[axis]*eps)-curve(x-np.eye(2)[axis]*eps)).item()/(2*eps) for axis in range(2)]))
                max_gradient_jump=max(max_gradient_jump,float(np.max(abs(grads[0]-grads[1]))))
            self.assertLess(max_gradient_jump,.15)
            self.report.append(dict(region=region,budget=info['budget'],polynomialMaxError=error,gradientJump=max_gradient_jump))
        Path('build/adaptive-free-surface-v6/numeric-verification.json').write_text(json.dumps(self.report,indent=2),encoding='utf-8')

if __name__=='__main__':unittest.main()

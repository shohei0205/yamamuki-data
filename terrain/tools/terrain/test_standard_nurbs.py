"""標準NURBSの同次座標評価・微分・逆写像・保存を確認する。"""
import unittest
import numpy as np
from scipy.interpolate import NdBSpline
from standard_nurbs import NurbsSurface,knots,regular_xy,design,encode,decode,adapted_xy,refine_to,fit_nurbs
class TestNurbs(unittest.TestCase):
 def make(self):
  n=9;t=knots(n);xy=regular_xy(n);z=.2*xy[:,0]+.1*xy[:,1]+30;w=np.exp(np.random.default_rng(9).uniform(-.5,.5,n*n));return NurbsSurface(np.column_stack((xy,z)),w,t,t)
 def test_homogeneous_reference(self):
  m=self.make();uv=np.random.default_rng(3).uniform(0,1,(200,2));n=len(m.tu)-4;homogeneous=np.column_stack((m.points*m.weights[:,None],m.weights)).reshape(n,n,4);reference=NdBSpline((m.tv,m.tu),homogeneous,(3,3))(uv[:,::-1]);np.testing.assert_allclose(m.evaluate(uv),reference[:,:3]/reference[:,3,None],atol=1e-10)
  changed=NurbsSurface(m.points,np.ones(len(m.points)),m.tu,m.tv);self.assertGreater(np.max(abs(changed.evaluate(uv)[:,:2]-m.evaluate(uv)[:,:2])),1)
 def test_inverse_plane_and_derivatives(self):
  m=self.make();q=np.random.default_rng(4).uniform(-7999,7999,(100,2));field=m.height_gradient(q);np.testing.assert_allclose(field[:,0],q[:,0]*.2+q[:,1]*.1+30,atol=1e-5);np.testing.assert_allclose(field[:,1:],np.tile([.2,.1],(len(q),1)),atol=1e-12);uv=m.inverse_xy(q);np.testing.assert_allclose(m.evaluate(uv)[:,:2],q,atol=1e-5)
  p,u,v=m.evaluate(uv,True);d=1e-6;np.testing.assert_allclose(u,(m.evaluate(uv+[d,0])-m.evaluate(uv-[d,0]))/(2*d),atol=1e-5);np.testing.assert_allclose(v,(m.evaluate(uv+[0,d])-m.evaluate(uv-[0,d]))/(2*d),atol=1e-5)
 def test_codec_and_boundary(self):
  m=self.make();restored=decode(encode(m));uv=np.random.default_rng(7).uniform(0,1,(100,2));self.assertLess(np.max(abs(m.evaluate(uv)-restored.evaluate(uv))),30);corners=np.array([[0,0],[1,0],[0,1],[1,1]]);n=len(m.tu)-4;np.testing.assert_allclose(m.evaluate(corners),m.points[[0,n-1,n*(n-1),n*n-1]])
  p=m.points.copy();p[:,2]=42;constant=NurbsSurface(p,m.weights,m.tu,m.tv);np.testing.assert_allclose(constant.evaluate(uv)[:,2],42,atol=1e-10)
 def test_exact_knot_refinement(self):
  model=self.make();uv=np.random.default_rng(881).uniform(0,1,(1000,2));refined=refine_to(model,18);self.assertEqual(len(refined.points),18**2)
  for a,b in zip(model.evaluate(uv,True),refined.evaluate(uv,True)):np.testing.assert_allclose(a,b,atol=1e-9)
  with self.assertRaises(ValueError):refine_to(model,8)
 def test_refined_model_fit_preserves_knots(self):
  initial=refine_to(self.make(),18);progress=[];target=lambda q:q[:,0]*.2+q[:,1]*.1+30
  fitted,info=fit_nurbs(initial.points[:,:2],18,target,25,initial=initial,iterations=2,progress=progress.append)
  np.testing.assert_array_equal(fitted.tu,initial.tu);np.testing.assert_array_equal(fitted.tv,initial.tv);np.testing.assert_array_equal(fitted.points[:,:2],initial.points[:,:2]);self.assertGreater(len(progress),0);self.assertGreaterEqual(info['trainingSamples'],18*18*16)
  q=np.random.default_rng(32).uniform(-7500,7500,(100,2));np.testing.assert_allclose(fitted(q),target(q),atol=1e-5)
 def test_quantized_roundtrip_exact(self):
  n=7;t=knots(n);xy=np.round(regular_xy(n)*16)/16;rng=np.random.default_rng(38);points=np.column_stack((xy,np.round(rng.uniform(-100,4000,n*n)*8)/8));weights=2**(rng.integers(32,97,n*n)/32-2);m=NurbsSurface(points,weights,t,t);other=decode(encode(m));np.testing.assert_array_equal(other.points,m.points);np.testing.assert_array_equal(other.weights,m.weights);np.testing.assert_array_equal(other.tu,t);uv=rng.uniform(0,1,(100,2));np.testing.assert_allclose(m.evaluate(uv),other.evaluate(uv),atol=1e-10)
 def test_deformed_control_net(self):
  target=lambda q:1000*np.exp(-np.sum((q-[1500,-2000])**2,axis=1)/2000000)
  xy=adapted_xy(12,target);base=regular_xy(12);self.assertGreater(np.max(abs(xy-base)),100);m=NurbsSurface(np.column_stack((xy,target(xy))),np.ones(len(xy)),knots(12),knots(12));uv=np.random.default_rng(8).uniform(0,1,(1000,2));_,u,v=m.evaluate(uv,True);self.assertTrue(np.all(u[:,0]*v[:,1]-u[:,1]*v[:,0]>0))
if __name__=='__main__':unittest.main()

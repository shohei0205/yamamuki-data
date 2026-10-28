"""局所パッチの平面再現・Bezier係数・混合サイズの接続を検証する。"""
import unittest
import numpy as np
from control_patch_comparison import QuadSurface,TriangleSurface,quad_eval,quads
class PatchTests(unittest.TestCase):
 def test_plane_and_bezier_payload(self):
  rect=np.array([0.,0.,10,10]);f=np.array([[[2,3,-.5,0],[-3,3,-.5,0]],[[32,3,-.5,0],[27,3,-.5,0]]]);q=np.random.default_rng(5).uniform(0,10,(50,2));truth=2+3*q[:,0]-.5*q[:,1]
  for cubic in [False,True]:np.testing.assert_allclose(quad_eval(q,rect,f,cubic),truth,atol=1e-10)
  net=np.frombuffer(QuadSurface([(rect,f)],True).payload(),'<f4')[4:].reshape(4,4);u=q[:,0]/10;v=q[:,1]/10
  bu=np.array([(1-u)**3,3*u*(1-u)**2,3*u*u*(1-u),u**3]);bv=np.array([(1-v)**3,3*v*(1-v)**2,3*v*v*(1-v),v**3]);h=sum(net[i,j]*bu[i]*bv[j] for i in range(4) for j in range(4));np.testing.assert_allclose(h,truth,atol=1e-5)
 def test_triangle_plane(self):
  xy=np.array([[0.,0.],[10,0],[0,10],[10,10],[4,3]]);p=np.column_stack((xy,2+3*xy[:,0]-.5*xy[:,1]));m=TriangleSurface(p,np.tile([3.,-.5],(len(p),1)));q=np.random.default_rng(4).uniform(.1,9.9,(50,2));np.testing.assert_allclose(m(q),2+3*q[:,0]-.5*q[:,1],atol=1e-10)
 def test_shared_edges(self):
  def source(q):return 500*np.exp(-((q[:,0]-800)**2+(q[:,1]-1200)**2)/2000**2)
  for model in quads(source,25):
   for i,(a,fa) in enumerate(model.patches):
    for b,fb in model.patches[i+1:]:
     for axis in [0,1]:
      if a[axis+2]!=b[axis] and b[axis+2]!=a[axis]:continue
      other=1-axis;lo=max(a[other],b[other]);hi=min(a[other+2],b[other+2])
      if hi<=lo:continue
      edge=a[axis+2] if a[axis+2]==b[axis] else b[axis+2];q=np.empty((5,2));q[:,axis]=edge;q[:,other]=np.linspace(lo,hi,5)
      np.testing.assert_allclose(quad_eval(q,a,fa,model.cubic),quad_eval(q,b,fb,model.cubic),atol=1e-5)
      if model.cubic:
       for dx,dy in [(1,0),(0,1)]:np.testing.assert_allclose(quad_eval(q,a,fa,True,dx,dy),quad_eval(q,b,fb,True,dx,dy),atol=1e-6)
if __name__=='__main__':unittest.main()

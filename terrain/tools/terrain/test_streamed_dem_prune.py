import unittest
import numpy as np
from streamed_dem_prune import DemReference,reduce_streamed,removal_box,removal_score
from prune_errorfit_tile import reduce_points
from sibson_aux_surface import AuxiliarySibsonSurface

class StreamedTest(unittest.TestCase):
 def test_matches_full_array_reference(self):
  guards=np.array([[-1000,-1000,0],[1000,-1000,0],[-1000,1000,0],[1000,1000,0.]])
  xy=np.array([[x,y,40*np.exp(-(x*x+y*y)/10)] for y in [-8,-4,0,4,8] for x in [-8,-4,0,4,8]])
  points=np.vstack((guards,xy));m=AuxiliarySibsonSurface(points);yy,xx=np.mgrid[:256,:256];dem=m(np.column_stack((xx.ravel()+.5-128,yy.ravel()+.5-128))).reshape(256,256)
  r=DemReference([128,128],10,read=lambda x,y:dem,step=1);q,z=r.block((0,0));fixed=np.any(abs(points[:,:2])>=8,axis=1)
  a,_,audit=reduce_streamed(points,r,3,fixed);b,_,other=reduce_points(points,q,z,3,fixed)
  np.testing.assert_array_equal(a,b);self.assertLessEqual(audit['max'],3);self.assertAlmostEqual(audit['max'],other['max'])
 def test_local_ranking_matches_global_deletion(self):
  rng=np.random.default_rng(730)
  for n in [32,128,512]:
   xy=np.vstack((np.array([[-100,-100],[100,-100],[-100,100],[100,100]]),rng.uniform(-90,90,(n,2))))
   p=np.column_stack((xy,rng.normal(0,100,len(xy))));m=AuxiliarySibsonSurface(p)
   for k in rng.choice(np.arange(4,len(p)),20,replace=False):
    reference=AuxiliarySibsonSurface(np.delete(p,k,axis=0));expected=abs(float(reference(p[k:k+1,:2])[0])-p[k,2])
    self.assertAlmostEqual(removal_score(m,k),expected,delta=1e-6)
 def test_sample_cache_is_bounded_and_reused(self):
  reads=[]
  def read(x,y):reads.append((x,y));return np.full((256,256),3.)
  reference=DemReference([256,128],256,read=read,step=1,sample_cache_bytes=256*256*24)
  reference.block((0,0));reference.block((0,0));self.assertEqual(len(reads),1)
  reference.block((1,0));self.assertLessEqual(reference.sample_bytes,reference.sample_limit)
  reference.block((0,0));self.assertEqual(len(reads),3)
 def test_partial_audit_preserves_unchanged_pixels_and_rejected_trial(self):
  dem=np.zeros((256,256));reference=DemReference([128,128],128,read=lambda x,y:dem,step=1)
  reference.commit(reference.audit(lambda q:np.zeros(len(q))))
  box=np.array([-2.,-2.,2.,2.]);evaluated=[]
  def changed(q):
   evaluated.append(len(q));return np.all((q>=box[:2])&(q<=box[2:]),axis=1).astype(float)
  updates=reference.audit(changed,box,2);self.assertLess(sum(evaluated),100);reference.commit(updates)
  self.assertEqual(reference.summary()['max'],1.)
  self.assertIsNone(reference.audit(lambda q:np.full(len(q),100.),box,2))
  self.assertEqual(reference.summary()['max'],1.)
  accepted=reference.audit(changed,box,2);reference.commit(accepted)
  q,z=reference.block((0,0));e=abs(changed(q)-z)
  self.assertEqual(reference.summary()['max'],float(e.max()))
  self.assertAlmostEqual(reference.summary()['rmse'],float(np.sqrt(np.mean(e*e))))
  self.assertLessEqual(reference.residual_bytes,reference.residual_limit)
 def test_removal_cannot_change_outside_support_box(self):
  rng=np.random.default_rng(522);xy=np.vstack((np.array([[-100,-100],[100,-100],[-100,100],[100,100]]),rng.uniform(-20,20,(40,2))));p=np.column_stack((xy,rng.uniform(0,100,len(xy))));m=AuxiliarySibsonSurface(p);q=rng.uniform(-80,80,(20000,2))
  for i in [12,17,22]:
   box=np.array(removal_box(m,i,1e-5));outside=np.any((q<box[:2])|(q>box[2:]),axis=1);other=AuxiliarySibsonSurface(np.delete(p,i,axis=0));self.assertLess(np.max(abs(m(q[outside])-other(q[outside]))),1e-6)

if __name__=='__main__':unittest.main()

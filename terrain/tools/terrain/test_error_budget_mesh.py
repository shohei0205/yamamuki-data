"""新仕様の不要頂点削減・角度修復の局所条件を確認する。"""
import unittest
import numpy as np
from error_budget_mesh import repair,prune
from refine_regional_quality import triangulate,angles
from compare_shared_allocation import residual

class ErrorBudgetTest(unittest.TestCase):
 def test_redundant_plane_vertices_are_removed_with_error_and_angle_preserved(self):
  y,x=np.mgrid[:17,:17];grid=x*.5+y*.25;xy=np.array([[x,y] for y in range(0,17,4) for x in range(0,17,4)]);p=np.c_[xy,grid[xy[:,1],xy[:,0]]];f=triangulate(p)
  out,faces,ids,report=prune(p,f,grid,notify=lambda s:None)
  self.assertGreater(report['removed'],0);self.assertGreaterEqual(angles(out,faces).min(),10.-1e-7);self.assertLess(np.abs(residual(out,faces,grid)).max(),1e-8)
  boundary=np.any((p[:,:2]==0)|(p[:,:2]==16),axis=1);self.assertTrue(set(np.flatnonzero(boundary)).issubset(set(ids)))
 def test_peak_is_not_deleted(self):
  y,x=np.mgrid[:17,:17];grid=30-np.hypot(x-8,y-8)*2;xy=np.array([[x,y] for y in range(0,17,4) for x in range(0,17,4)]);p=np.c_[xy,grid[xy[:,1],xy[:,0]]];out,f,ids,_=prune(p,triangulate(p),grid,error=2.,notify=lambda s:None)
  self.assertTrue(np.any(np.all(out[:,:2]==[8,8],axis=1)))
 def test_angle_repair_retains_original_points(self):
  y,x=np.mgrid[:33,:33];grid=x*.1;xy=np.array([[0,0],[32,0],[32,32],[0,32],[16,1]]);p=np.c_[xy,grid[xy[:,1],xy[:,0]]];out,f=repair(p,grid,notify=lambda s:None)
  self.assertTrue(np.array_equal(out[:len(p)],p));self.assertGreaterEqual(angles(out,f).min(),10.-1e-7)
 def test_different_storage_widths_keep_parent_positions_and_normals(self):
  import struct,zlib
  from uncut_mesh_topology import tile_topology
  y,x=np.mgrid[:33,:33];grid=20*np.exp(-((x-16)**2+(y-16)**2)/100);xy=np.c_[x[::4,::4].ravel(),y[::4,::4].ravel()];p=np.c_[xy,grid[xy[:,1],xy[:,0]]];f=triangulate(p);seen={};shared=0
  for width in (8,16):
   for (tx,ty),body,_ in tile_topology(p,f,width=width,sample_count=33):
    raw=zlib.decompress(body);_,_,nv,ni,_=struct.unpack_from('<4sIIII',raw);q=np.frombuffer(raw,'<f4',nv*3,20).reshape(-1,3).copy();q[:,:2]+=[tx*width,ty*width];normals=np.frombuffer(raw,'<f4',nv*3,20+nv*12+ni*4).reshape(-1,3)
    for v,normal in zip(q,normals):
     key=tuple(v[:2])
     if key in seen:self.assertTrue(np.array_equal(seen[key][0],v));self.assertTrue(np.array_equal(seen[key][1],normal));shared+=1
     else:seen[key]=(v,normal)
  self.assertGreater(shared,0)
if __name__=='__main__':unittest.main()

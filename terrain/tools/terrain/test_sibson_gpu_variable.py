"""制御点パケットの補間値と、外周セルの有限化を確認する。"""
import json,struct,unittest
import numpy as np
from scipy.stats import qmc
from sibson_gpu_variable import packet,close_cells
from sibson_variable_server import database,Regional
from sibson_variable128 import restore
from sibson_aux_surface import AuxiliarySibsonSurface

def decode(blob):
 size=struct.unpack_from('<I',blob)[0];info=json.loads(blob[4:4+size]);offset=4+size;arrays=[]
 for name,dtype,columns in [('pointBytes','<u4',8),('cellBytes','<f4',2),('nodeBytes','<u4',8),('referenceBytes','<u4',1)]:
  length=info[name];arrays.append(np.frombuffer(blob,dtype,count=length//4,offset=offset).reshape(-1,columns));offset+=length
 return info,*arrays

def evaluate(blob,queries):
 info,records,cells,nodes,refs=decode(blob);points=records.view('<f4');answer=[]
 for q in np.asarray(queries)/1000:
  low=np.array(info['indexBounds'][:2])/1000;high=np.array(info['indexBounds'][2:])/1000;node=nodes[0]
  while node[0]!=0xffffffff:
   mid=(low+high)/2;bits=q>=mid;index=int(bits[0])+2*int(bits[1]);low=np.where(bits,mid,low);high=np.where(bits,high,mid);node=nodes[node[index]]
  total=0.;height=0.
  for i in refs[node[4]:node[4]+node[5],0]:
   d=points[i,:2].astype(float)-q
   if d@d<1e-16:height=float(points[i,2]);total=1.;break
   poly=cells[records[i,3]:records[i,3]+records[i,6]].astype(float);signed=poly@d+float(d@d)*.5;clipped=[]
   for j in range(len(poly)):
    previous=(j-1)%len(poly);a,b=signed[previous],signed[j]
    if (a<=0)!=(b<=0):clipped.append(poly[previous]+(poly[j]-poly[previous])*a/(a-b))
    if b<=0:clipped.append(poly[j])
   if len(clipped)<3:continue
   v=np.asarray(clipped);v-=v[0];area=abs(np.sum(v[:,0]*np.roll(v[:,1],-1)-v[:,1]*np.roll(v[:,0],-1)))*.5;total+=area;height+=area*points[i,2]
  answer.append(height/total)
 return np.array(answer)

class TestGPUVariable(unittest.TestCase):
 def test_sparse_tiles_do_not_expand_tree_without_reducing_candidates(self):
  for error,x,y in [(12.5,55,43),(12.5,32,58),(25,16,30)]:
   blob=packet('fuji',error,x,y);info,*_=decode(blob);self.assertLess(len(blob),1048576);self.assertLessEqual(info['maxCandidates'],256)
   with database() as db:p=restore(Regional(db,'fuji'),error,x,y).astype(float)
   model=AuxiliarySibsonSurface(p);bounds=np.array(info['worldFieldBounds']);q=bounds[:2]+qmc.Sobol(2,scramble=True,seed=91).random_base2(6)*(bounds[2:]-bounds[:2]);np.testing.assert_allclose(evaluate(blob,q-np.array(info['origin'])),model(q),atol=.01)

 def test_all_levels_and_regions(self):
  reports=[]
  for region in ['fuji','yari','tsukuba']:
   for error,side in [(800,128000),(400,64000),(200,32000),(100,16000),(50,8000),(25,4000),(12.5,2000)]:
    x=y=int(64000/side) if side<128000 else 0
    blob=packet(region,error,x,y);info,*_=decode(blob);self.assertEqual(info['unsafeLeaves'],0)
    with database() as db:p=restore(Regional(db,region),error,x,y).astype(float)
    model=AuxiliarySibsonSurface(p);lo=np.maximum(info['coreBounds'][:2],[1,1]);hi=np.minimum(info['coreBounds'][2:],[127999,127999]);q=lo+(hi-lo)*qmc.Sobol(2,scramble=True,seed=32).random_base2(6)
    # 外周の鏡像延長は別の境界条件。ここでは地域中央側を比較する。
    q=np.clip(q,[32000,32000],[96000,96000]) if side>=64000 else q
    expected=model(q);local=q-np.array(info['origin']);actual=evaluate(blob,local);difference=float(abs(actual-expected).max());self.assertLess(difference,.01,(region,error,difference));reports.append(dict(region=region,error=error,maximumDifference=difference,bytes=len(blob),controls=info['controls']))
  from pathlib import Path
  Path('build/sibson-gpu-variable-v1/packet-verification.json').write_text(json.dumps(reports,indent=2),encoding='utf-8')

 def test_finite_cells_preserve_stolen_area(self):
  with database() as db:p=restore(Regional(db,'fuji'),100,1,2).astype(float)
  p[:,:2]-=[24000,40000];model=AuxiliarySibsonSurface(p);before=model(np.array([[-7999.,-7999.],[0.,0.],[7999.,7999.]]));close_cells(model)
  self.assertTrue(np.all(np.diff(model.cell_off)>0))
  np.testing.assert_allclose(model(np.array([[-7999.,-7999.],[0.,0.],[7999.,7999.]])),before,atol=1e-6)

if __name__=='__main__':unittest.main()

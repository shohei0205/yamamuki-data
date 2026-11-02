"""階層生成の継承誤差、制御点の由来、元DEMの最終誤差を別実装で照合する。"""
import json,sqlite3,unittest
from pathlib import Path
import numpy as np
from sibson_aux_surface import AuxiliarySibsonSurface
from sibson_native_codec import decode_independent
from streamed_dem_prune import DemReference
ROOT=Path(__file__).resolve().parents[2]
WORLD=40075016.68557849
FIXTURES=[('natural-hierarchy-smoke-fuji-v3',13,7252,3234),('natural-hierarchy-smoke-yari-v3',13,7228,3207),('natural-hierarchy-smoke-tsukuba-v3',13,7284,3210),('natural-hierarchy-smoke-fuji-v1',12,3626,1617),('natural-hierarchy-repair-13-7253-3242',13,7253,3242),('natural-hierarchy-repair-13-7248-3242',13,7248,3242),('natural-hierarchy-repair-12-3616-1598',12,3616,1598),('natural-hierarchy-quant-repair',13,7244,3226),('natural-hierarchy-edge-7243-3227',13,7243,3227),('natural-hierarchy-edge-7243-3226',13,7243,3226),('natural-hierarchy-edge-7242-3226',13,7242,3226),('natural-hierarchy-edge-7256-3221',13,7256,3221),('natural-hierarchy-edge-7239-3227',13,7239,3227)]
def tile(root,z,x,y):
 folder=root/f'tiles/{z}/{x}/{y}';meta=json.loads((folder/'packet.json').read_text());audit=json.loads((folder/'selection.json').read_text());p=decode_independent((folder/'control-xyz.nqc1').read_bytes()).astype(float);side=WORLD/(1<<z);core=np.array([x*side,y*side,(x+1)*side,(y+1)*side]);world=(core[:2]+core[2:])/2;origin=np.floor(core[:2]/.0625+.5);p[:,:2]=(p[:,:2]+origin)*.0625-world;p[:,2]*=.125;return folder,p,world,meta,audit
class HierarchyTest(unittest.TestCase):
 def test_immediate_children_and_error_budget(self):
  for name,z,x,y in FIXTURES:
   root=ROOT/'build'/name;_,p,w,meta,a=tile(root,z,x,y);self.assertEqual(a['generationMode'],'fine-to-coarse');self.assertTrue(a['allDemPixelsAudited']);self.assertTrue(all(cz==z+1 for cz,_,_ in a['fineReferences']));self.assertGreater(a['curveErrorBudget'],0);self.assertAlmostEqual(a['curveErrorBudget']+a['fineReferenceMaxError']+.125,a['errorTarget']);self.assertLessEqual(a['curveSampleMaxError'],a['curveErrorBudget']);self.assertLessEqual(a['max'],a['errorTarget'])
 def test_full_source_pixels_are_within_target(self):
  for name,z,x,y in FIXTURES:
   with self.subTest(name=name,z=z):
    root=ROOT/'build'/name;_,p,world,meta,a=tile(root,z,x,y);domains=np.asarray(json.loads((root/'settings.json').read_text())['domainBounds']);reference=DemReference(world,meta['side']/2+250);model=AuxiliarySibsonSurface(p);maximum=0.;count=0
    for key in reference.keys:
     q,values=reference.block(key);globalq=q+world;keep=np.any(np.all((globalq[:,None,:]>=domains[None,:,:2])&(globalq[:,None,:]<=domains[None,:,2:]),axis=2),axis=1)
     if keep.any():maximum=max(maximum,float(np.max(abs(model(q[keep])-values[keep]))));count+=int(keep.sum())
    self.assertLessEqual(maximum,a['errorTarget']+1e-6);self.assertAlmostEqual(maximum,a['max'],delta=1e-5);self.assertEqual(count,a['validPixels'])
 def test_real_control_heights_come_from_finer_curves(self):
  name,z,x,y=FIXTURES[0];root=ROOT/'build'/name;_,p,world,meta,a=tile(root,z,x,y);patches=[]
  for cz,cx,cy in a['fineReferences']:
   _,child,cworld,cmeta,ca=tile(root,cz,cx,cy);side=WORLD/(1<<cz);core=np.array([cx*side,cy*side,(cx+1)*side,(cy+1)*side]);patches.append((AuxiliarySibsonSurface(child),cworld,core))
  q=p[4:,:2]+world;total=np.zeros(len(q));value=np.zeros(len(q))
  for model,cworld,core in patches:
   d=np.maximum(np.maximum(core[:2]-q,q-core[2:]),0);t=np.clip(1-d/250,0,1);weight=np.prod(t*t*(3-2*t),axis=1);use=weight>0
   if use.any():value[use]+=weight[use]*model(q[use]-cworld);total[use]+=weight[use]
  self.assertTrue((total>0).all());self.assertLessEqual(float(np.max(abs(value/total-p[4:,2]))),.062501)
 def test_no_dem_reads_in_curve_fitting_and_one_final_scan(self):
  for name,z,x,y in FIXTURES[:3]:
   _,_,_,_,a=tile(ROOT/'build'/name,z,x,y);self.assertFalse(a['DEMUsedDuringCurveFit']);self.assertTrue(a['PointHeightsFromFineCurves']);self.assertEqual(a['sourceAuditPasses'],1);self.assertEqual(a['finalDEMDrivenRefinementPasses'],0)
if __name__=='__main__':unittest.main()

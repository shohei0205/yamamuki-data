"""Rust地域生成の実データ検査。生成条件、元DEM誤差、Rice、C1と再開を照合する。"""
import json,subprocess,sys,tempfile,unittest
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from sibson_aux_surface import AuxiliarySibsonSurface
from sibson_global_c1 import GlobalC1
from sibson_c1 import SibsonC1
from sibson_native_codec import decode_independent
from streamed_dem_prune import DemReference
ROOT=Path(__file__).resolve().parents[2]
ENGINE=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-regional.exe'
SMOKES=[('natural-xyz-rust-smoke-v1',14,14505,6469),('natural-xyz-rust-smoke-yari-v1',14,14456,6414),('natural-xyz-rust-smoke-tsukuba-v1',14,14568,6421)]
def load(row):
 name,z,x,y=row;folder=ROOT/f'build/{name}/tiles/{z}/{x}/{y}';data=json.loads((folder/'control-gradient.json').read_text());info=json.loads((folder/'packet.json').read_text());return folder,np.asarray(data['points']),np.asarray(data['gradient']),info
class RustRegionalTest(unittest.TestCase):
 def test_sibson_matches_python(self):
  rng=np.random.default_rng(791);p=np.column_stack((np.vstack((np.array([[-100,-100],[100,-100],[-100,100],[100,100]]),rng.uniform(-90,90,(120,2)))),rng.normal(0,100,124)));q=rng.uniform(-80,80,(4096,2));expected=AuxiliarySibsonSurface(p)(q)
  with tempfile.TemporaryDirectory(dir=ROOT/'build') as tmp:
   inp=Path(tmp)/'input.json';out=Path(tmp)/'output.json';inp.write_text(json.dumps(dict(points=p.tolist(),queries=q.tolist())));subprocess.run([str(ENGINE),'--evaluate',str(inp),'--output',str(out)],check=True,cwd=ROOT);actual=np.asarray(json.loads(out.read_text()));self.assertLess(float(np.max(abs(actual-expected))),1e-6)
 def test_three_regions_integer_roundtrip_and_full_audit(self):
  for row in SMOKES:
   with self.subTest(region=row[0]):
    folder,p,g,info=load(row);world=np.asarray(info['origin']);ints=decode_independent((folder/'control-xyz.nqc1').read_bytes());origin=np.floor(np.asarray(info['coreBounds'][:2])/.0625+.5);restored=ints.astype(float);restored[:,:2]=(restored[:,:2]+origin)*.0625-world;restored[:,2]*=.125;np.testing.assert_allclose(restored,p,atol=1e-8,rtol=0)
    ref=DemReference(world,info['side']/2+250);ref.cache=ref.audit(AuxiliarySibsonSurface(restored));maximum,rmse,count=ref.summary().values();self.assertLessEqual(maximum,info['error']+1e-6);self.assertGreater(count,0);self.assertEqual(info['unsafeLeaves'],0)
    packet=(folder/'packet.bin').read_bytes();n=int.from_bytes(packet[:4],'little');meta=json.loads(packet[4:4+n]);self.assertEqual(len(packet),4+n+sum(meta[k] for k in ['pointBytes','cellBytes','nodeBytes','referenceBytes']));self.assertEqual(meta['pointBytes'],meta['controls']*32)
 def test_python_checkpoint_is_resumed_without_losing_points(self):
  from import_regional_xyz_python import encode
  from sibson_native_codec import decode_independent
  old=ROOT/'build/natural-xyz-tight-errors-v1';p=np.load(old/'fine/xyz/10/14505-6469/control-gradient.npz')['points'];world=np.asarray(json.loads((old/'seed/xyz/10/14505-6469/packet.json').read_text())['origin']);p[:,:2]=np.floor((p[:,:2]+world)/.0625+.5)*.0625-world;p[:,2]=np.floor(p[:,2]/.125+.5)*.125;mask=np.load(old/'tiles/14/14505/6469/prune-state.npz')['active'];expected=decode_independent((old/'tiles/14/14505/6469/control-xyz.nqc1').read_bytes())
  with tempfile.TemporaryDirectory(dir=ROOT/'build') as tmp:
   root=Path(tmp);folder=root/'tiles/14/14505/6469';folder.mkdir(parents=True);(folder/'rust-state.bin').write_bytes(encode(1,p,mask));subprocess.run([str(ENGINE),'--output',str(root),'--tile','14','14505','6469','--workers','1'],check=True,cwd=ROOT);np.testing.assert_array_equal(decode_independent((folder/'control-xyz.nqc1').read_bytes()),expected)
 def test_c1_objective_matches_reference(self):
  _,p,g,info=load(SMOKES[0]);bounds=np.asarray(info['bounds']);q=qmc.Sobol(2,scramble=True,seed=9124).random_base2(max(14,int(np.ceil(np.log2(len(p)*4)))))*(bounds[2:]-bounds[:2])+bounds[:2];ref=GlobalC1(p,strength=.01,queries=q,surface=AuxiliarySibsonSurface(p));actual=SibsonC1(p,gradient=g);difference=ref(q[:2048])-actual(q[:2048]);self.assertLess(float(np.sqrt(np.mean(difference*difference))),.05);self.assertLess(float(np.max(abs(difference))),.5)
if __name__=='__main__':unittest.main()

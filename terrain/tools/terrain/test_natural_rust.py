"""Rust版SibsonとRice符号化を既存の参照実装で検査する。"""
import json,subprocess,tempfile,unittest
from pathlib import Path
import numpy as np
from sibson_surface import SibsonSurface
from sibson_native_codec import encode_independent,encode_delta,decode_independent,decode_delta_arrays
ROOT=Path(__file__).resolve().parents[2]
EXE=ROOT/'build/natural-national-target/release/terrain-natural-national.exe'
class RustNatural(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory(dir=ROOT/'build');self.addCleanup(self.tmp.cleanup);self.folder=Path(self.tmp.name)
 def run_native(self,mode,value):
  a=self.folder/'input.json';b=self.folder/'output.bin';a.write_text(json.dumps(value));subprocess.run([str(EXE),'--'+mode,str(a),'--output',str(b)],check=True,cwd=ROOT);return b.read_bytes()
 def test_sibson_matches_c_reference(self):
  rng=np.random.default_rng(137);xy=np.vstack(([[0,0],[32000,0],[0,32000],[32000,32000]],rng.uniform(10,31990,(300,2))));z=500+np.sin(xy[:,0]/1800)*1000+np.cos(xy[:,1]/2500)*300;points=np.column_stack((xy,z));queries=rng.uniform(1,31999,(2000,2));expected=SibsonSurface(points)(queries)
  actual=np.asarray(json.loads(self.run_native('evaluate',dict(points=points.tolist(),queries=queries.tolist()))));self.assertLess(float(abs(actual-expected).max()),1e-7)
 def test_independent_bytes(self):
  rng=np.random.default_rng(44)
  for n in [1,3,128,129,1024]:
   points=np.cumsum(rng.integers(-3000,3001,(n,3)),axis=0).astype(np.int32);actual=self.run_native('pack',dict(points=points.tolist()));self.assertEqual(actual,encode_independent(points));np.testing.assert_array_equal(decode_independent(actual),points)
 def test_delta_bytes(self):
  rng=np.random.default_rng(7);parents=[rng.integers(-10000,10001,(200,3)).tolist(),rng.integers(-10000,10001,(200,3)).tolist()];points=[];refs=[];raw=[];dz=[];fresh=[]
  for i in range(400):
   if i%3==0:p=rng.integers(-1000,1001,3).tolist();refs.append(None);raw.append(0xffffffff);dz.append(0);fresh.append(p)
   else:slot=i%2;index=i%200;p=parents[slot][index].copy();p[2]+=i;refs.append([slot,index]);raw.append((slot<<24)|index);dz.append(i)
   points.append(p)
  actual=self.run_native('pack',dict(points=points,parents=parents,refs=refs));self.assertEqual(actual,encode_delta(raw,dz,fresh,2));decode_delta_arrays(actual)
if __name__=='__main__':unittest.main()

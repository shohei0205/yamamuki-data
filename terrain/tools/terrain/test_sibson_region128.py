"""128 km版の復元、共通境界、欠損サンプリングを確認する。"""
import json,sqlite3,struct,tempfile,unittest,sys
from pathlib import Path
import numpy as np
import sibson_region128 as generator
import sibson_region128_server as server
from mls_quant_codec import encode_quant,decode_quant
from sibson_shared_codec import encode_shared
from sibson_delta_codec import checksum

def fixture(root):
 root=Path(root)
 for region in ['fuji','yari','tsukuba']:
  folder=root/region;folder.mkdir(parents=True,exist_ok=True)
  db=sqlite3.connect(folder/'terrain.sqlite3');db.execute('CREATE TABLE IF NOT EXISTS tiles(x INTEGER,y INTEGER,error REAL,parent_error REAL,blob BLOB,sha256 BLOB,info TEXT,PRIMARY KEY(x,y,error)) WITHOUT ROWID')
  db.execute('CREATE TABLE IF NOT EXISTS models(error REAL PRIMARY KEY,blob BLOB,info TEXT)')
  previous=None;pe=None
  for e in generator.ERRORS:
   xy=np.array([[0,0],[128000,0],[128000,128000],[0,128000]],float)
   p=decode_quant(encode_quant(np.column_stack((xy,1000+xy[:,0]*.001-xy[:,1]*.002+e*.125)),xy=.0625,zstep=.125))
   b=encode_quant(p,xy=.0625,zstep=.125) if e in [800,400,50] else encode_shared(p,previous,pe)
   db.execute('INSERT OR REPLACE INTO models VALUES(?,?,?)',(e,encode_quant(p,xy=.0625,zstep=.125),json.dumps(dict(maxError=0,samples=2000000,testFixture=True))))
   for y in range(16):
    for x in range(16):
     m=dict(x=x,y=y,error=e,bounds=[x*8000,y*8000,(x+1)*8000,(y+1)*8000],points=4,bytes=len(b),haloPoints=4)
     db.execute('INSERT OR REPLACE INTO tiles VALUES(?,?,?,?,?,?,?)',(x,y,e,None if e in [800,400,50] else pe,b,checksum(p),json.dumps(m)))
   previous=p;pe=e
  db.commit();db.close();generator.write(folder/'progress.json',dict(region=region,stage='complete'));generator.write(folder/'source.json',dict(center=[0,0],testFixture=True))

def unpack(body):
 size=struct.unpack_from('<I',body)[0];meta=json.loads(body[4:4+size]);return meta,np.frombuffer(body[4+size:],'<f4').reshape(meta['ny']+1,meta['nx']+1,3)

class Region128Test(unittest.TestCase):
 @classmethod
 def setUpClass(cls):
  cls.temp=tempfile.TemporaryDirectory(dir=Path('build').resolve());fixture(cls.temp.name);cls.old=server.ROOT;server.ROOT=Path(cls.temp.name);server.cache.clear()
 @classmethod
 def tearDownClass(cls):server.ROOT=cls.old;server.cache.clear();cls.temp.cleanup()
 def test_same_quality_boundary(self):
  a,fa=unpack(server.field('fuji',7,7,50,32));b,fb=unpack(server.field('fuji',8,7,50,32))
  ia=round((64000-a['fieldBounds'][0])/a['step']);ib=round((64000-b['fieldBounds'][0])/b['step'])
  np.testing.assert_array_equal(fa[:,ia],fb[:,ib])
 def test_parent_restore_and_field(self):
  m,f=unpack(server.field('yari',7,7,12.5,32));self.assertEqual(m['points'],4);self.assertTrue(np.isfinite(f).all())
  np.testing.assert_allclose(f[:,:,1],.001,atol=1e-7);np.testing.assert_allclose(f[:,:,2],-.002,atol=1e-7)
 def test_masked_source_sampling(self):
  raw=generator.RegionDem.__new__(generator.RegionDem);raw.center=np.array([0.,0.]);raw.left=raw.top=0;raw.grid=np.array([[100.,np.nan],[200.,300.]],np.float32)
  q=np.array([[.5,.5],[1.,1.]])*generator.STEP+64000
  result=raw.sample(q);self.assertAlmostEqual(result[0],100);self.assertAlmostEqual(result[1],200)
 def test_final_boundary_verifier(self):
  from verify_sibson_region128 import verify
  verify(Path(self.temp.name),regions=['fuji'],errors=[800.])
  result=json.loads((Path(self.temp.name)/'verification.json').read_text());self.assertTrue(result['passed']);self.assertEqual(result['levels'][0]['edges'],480)
  self.assertFalse((Path(self.temp.name)/'all-work-complete.json').exists())
 def test_distribution_excludes_working_models(self):
  from verify_sibson_region128 import export_distribution
  folder=Path(self.temp.name)/'fuji';size=export_distribution(folder);self.assertGreater(size,0)
  db=sqlite3.connect(folder/'terrain-distribution.sqlite3')
  try:
   self.assertEqual(db.execute('SELECT count(*) FROM tiles').fetchone()[0],1792)
   self.assertEqual(db.execute("SELECT count(*) FROM sqlite_master WHERE name='models'").fetchone()[0],0)
   self.assertEqual(json.loads(db.execute("SELECT value FROM metadata WHERE key='independentErrors'").fetchone()[0]),[800,400,50])
   self.assertEqual(len(server.restore(db,7,7,12.5)),4)
  finally:db.close()

if __name__=='__main__':
 if len(sys.argv)>1 and sys.argv[1]=='--fixture':fixture(sys.argv[2])
 else:unittest.main()

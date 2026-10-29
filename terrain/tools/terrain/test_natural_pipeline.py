"""小さな既知DEMで全国生成の全工程と失敗時の確定防止を試す。"""
from contextlib import closing
import json,sqlite3,subprocess,tempfile,unittest
from pathlib import Path
import numpy as np
from PIL import Image
from sibson_surface import SibsonSurface
from raw_dem_reference import STEP
ROOT=Path(__file__).resolve().parents[2]
EXE=ROOT/'build/natural-national-target/release/terrain-natural-national.exe'
class NativePipeline(unittest.TestCase):
 def test_all_stages_and_invalid_finalization(self):
  with tempfile.TemporaryDirectory(dir=ROOT/'build',prefix='native pipeline ') as folder:
   root=Path(folder);cache=root/'cache/14/14500';cache.mkdir(parents=True);source=root/'source.sqlite3';output=root/'output'
   y,x=np.mgrid[:256,:256];h=500+x*1.25+y*1.75+400*np.exp(-((x-90)**2+(y-120)**2)/2400);raw=np.floor(h*100+.5)/100;v=np.floor(h*100+.5).astype(np.uint32);rgb=np.stack((v>>16,v>>8,v),axis=-1).astype(np.uint8);Image.fromarray(rgb).save(cache/'6450.png')
   with closing(sqlite3.connect(source)) as c,c:c.execute('CREATE TABLE tiles(z,x,y)');c.execute('INSERT INTO tiles VALUES(14,14500,6450)')
   def run(*args,ok=True):
    r=subprocess.run([str(EXE),*map(str,args),'--output',str(output),'--workers','2'],cwd=ROOT,capture_output=True,text=True,encoding='utf-8',timeout=120)
    self.assertEqual(r.returncode==0,ok,r.stderr);return r
   run('--inventory','--source',source,'--cache',root/'cache');run('--lpf');run('--fit');run('--pack-tiles');run('--verify')
   report=json.loads((output/'verification.json').read_text());self.assertEqual(report['verified_tiles'],7);self.assertEqual(report['coverage_unresolved'],4095)
   run('--verify','--finalize',ok=False);self.assertTrue((output/'distribution.partial.sqlite3').exists());self.assertFalse(list(output.glob('terrain-natural-*.sqlite3')))
   with closing(sqlite3.connect(output/'work.sqlite3')) as c:
    fingerprint=c.execute("SELECT value FROM metadata WHERE key='input_manifest_sha256'").fetchone()[0]
    previous=set()
    queries=np.column_stack((((14500*256+x+.5)*STEP).ravel(),((6450*256+y+.5)*STEP).ravel()))
    for z,error in [(8,800),(9,400),(10,200),(11,100),(12,50),(13,25),(14,12.5)]:
     points=np.asarray(c.execute('SELECT p.qx,p.qy,l.qz FROM points p JOIN point_levels l ON l.id=p.id WHERE l.z=? ORDER BY p.id',(z,)).fetchall());ids=set(map(tuple,points[:,:2]));self.assertTrue(previous<=ids);previous=ids
     actual=SibsonSurface(points*[.0625,.0625,.125])(queries);self.assertLessEqual(float(abs(actual-raw.ravel()).max()),error)
   rx,ry=14500>>6,6450>>6
   with closing(sqlite3.connect(output/'coverage-source.sqlite3')) as c,c:
    c.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)');c.execute('INSERT INTO metadata VALUES(?,?)',('input_manifest_sha256',fingerprint));c.execute('CREATE TABLE cells(x INTEGER,y INTEGER,state TEXT,sha TEXT,bytes INTEGER,PRIMARY KEY(x,y))')
    c.executemany('INSERT INTO cells VALUES(?,?,?,NULL,0)',((rx*64+dx,ry*64+dy,'cached' if (rx*64+dx,ry*64+dy)==(14500,6450) else 'verified404') for dy in range(64) for dx in range(64)))
   run('--pack-tiles');run('--verify')
   with closing(sqlite3.connect(output/'distribution.partial.sqlite3')) as c:
    original=c.execute('SELECT xyz_sha256 FROM tiles WHERE z=8').fetchone()[0]
    with c:c.execute('UPDATE tiles SET xyz_sha256=zeroblob(32) WHERE z=8')
   run('--verify',ok=False)
   with closing(sqlite3.connect(output/'distribution.partial.sqlite3')) as c,c:c.execute('UPDATE tiles SET xyz_sha256=? WHERE z=8',(original,));c.execute('UPDATE tiles SET dependency_count=dependency_count+1 WHERE z=14')
   run('--verify',ok=False)
   with closing(sqlite3.connect(output/'distribution.partial.sqlite3')) as c,c:c.execute('UPDATE tiles SET dependency_count=dependency_count-1 WHERE z=14')
   run('--verify','--finalize');report=json.loads((output/'verification.json').read_text());self.assertTrue(report['complete']);self.assertFalse(report['distributable']);self.assertEqual(report['coverage_unresolved'],0);self.assertTrue(Path(report['path']).is_file());self.assertFalse((output/'distribution.partial.sqlite3').exists())
   run('--verify','--finalize');self.assertTrue(json.loads((output/'verification.json').read_text())['reverified'])
if __name__=='__main__':unittest.main()

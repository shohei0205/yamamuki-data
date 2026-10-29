"""固定LPFは元DEMの最大差で弱めず、適合はLPFを基準とする。"""
from contextlib import closing
import json,sqlite3,subprocess,tempfile,unittest
from pathlib import Path
import numpy as np
from PIL import Image
from sibson_surface import SibsonSurface
from test_fixed_lpf_regions import decompress,STEP
ROOT=Path(__file__).resolve().parents[2]
EXE=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-national.exe'
class FixedLpf(unittest.TestCase):
 def test_fixed_sigma_and_filtered_reference(self):
  with tempfile.TemporaryDirectory(dir=ROOT/'build',prefix='fixed-lpf-test-') as folder:
   p=Path(folder);cache=p/'cache/14/14500';cache.mkdir(parents=True);source=p/'source.sqlite3';out=p/'output';cfg=json.loads((ROOT/'NATIONAL_NATURAL_SETTINGS.json').read_text(encoding='utf-8-sig'));cfg.update(lpf_policy='fixed-error-half',fit_reference='lpf',fit_error_ratio=.125);settings=p/'settings.json';settings.write_text(json.dumps(cfg))
   y,x=np.mgrid[:256,:256];h=500+600*np.exp(-((x-128)**2+(y-128)**2)/(2*9**2));v=np.floor(h*100+.5).astype(np.uint32);Image.fromarray(np.stack([v>>16,v>>8,v],axis=-1).astype(np.uint8)).save(cache/'6450.png')
   with closing(sqlite3.connect(source)) as c,c:c.execute('CREATE TABLE tiles(z,x,y)');c.execute('INSERT INTO tiles VALUES(14,14500,6450)')
   for mode in ['inventory','lpf','fit']:
    args=[str(EXE),'--'+mode,'--output',str(out),'--workers','2']
    if mode=='inventory':args+=['--source',str(source),'--cache',str(p/'cache'),'--settings',str(settings)]
    r=subprocess.run(args,cwd=ROOT,capture_output=True,text=True,encoding='utf-8',timeout=180);self.assertEqual(r.returncode,0,r.stderr)
   queries=np.column_stack((((14500*256+x+.5)*STEP).ravel(),((6450*256+y+.5)*STEP).ravel()))
   with closing(sqlite3.connect(out/'work.sqlite3')) as c:
    meta=dict(c.execute('SELECT key,value FROM metadata'))
    for z,e in [(8,800),(9,400),(10,200),(11,100),(12,50),(13,25),(14,12.5)]:
     self.assertEqual(float(meta[f'lpf-sigma-z{z}']),e/2)
     path=c.execute("SELECT path FROM artifacts WHERE stage='lpf' AND z=?",(z,)).fetchone()[0];blob=decompress(Path(path).read_bytes());grid=np.frombuffer(blob,offset=16,dtype='<f4').reshape(1024,1024);target=grid[(6450%4)*256:(6450%4+1)*256,(14500%4)*256:(14500%4+1)*256].ravel()
     points=np.asarray(c.execute('SELECT p.qx,p.qy,l.qz FROM points p JOIN point_levels l ON l.id=p.id WHERE l.z=?',(z,)).fetchall())*[.0625,.0625,.125]
     self.assertLessEqual(float(abs(SibsonSurface(points)(queries)-target).max()),e*.125)
     if z==8:self.assertGreater(float(abs(target-h.ravel()).max()),e/2)
if __name__=='__main__':unittest.main()

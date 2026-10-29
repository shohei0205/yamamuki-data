"""スケール配置の再現性・親点包含・量子化・誤差保証の非宣言を確認する。"""
from contextlib import closing
import json,sqlite3,subprocess,tempfile,unittest
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[2]
EXE=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-national.exe'
class ScalePlacement(unittest.TestCase):
 def test_nested_deterministic_and_measured_afterwards(self):
  with tempfile.TemporaryDirectory(dir=ROOT/'build',prefix='scale-placement-test-') as folder:
   p=Path(folder);cache=p/'cache/14/14500';cache.mkdir(parents=True);src=p/'source.sqlite3';out=p/'output';cfg=json.loads((ROOT/'NATIONAL_NATURAL_SETTINGS.json').read_text(encoding='utf-8-sig'));cfg.update(lpf_policy='fixed-error-half',fit_reference='lpf',placement_policy='lpf-scale-poisson');settings=p/'settings.json';settings.write_text(json.dumps(cfg))
   y,x=np.mgrid[:256,:256];h=500+600*np.exp(-((x-128)**2+(y-128)**2)/(2*9**2));v=np.floor(h*100+.5).astype(np.uint32);Image.fromarray(np.stack([v>>16,v>>8,v],axis=-1).astype(np.uint8)).save(cache/'6450.png')
   with closing(sqlite3.connect(src)) as c,c:c.execute('CREATE TABLE tiles(z,x,y)');c.execute('INSERT INTO tiles VALUES(14,14500,6450)')
   def run(mode,*extra):
    r=subprocess.run([str(EXE),'--'+mode,'--output',str(out),'--workers','2',*map(str,extra)],cwd=ROOT,capture_output=True,text=True,encoding='utf-8',timeout=120);self.assertEqual(r.returncode,0,r.stderr)
   run('inventory','--source',src,'--cache',p/'cache','--settings',settings);run('lpf');run('place-scale')
   with closing(sqlite3.connect(out/'work.sqlite3')) as c:
    before=c.execute('SELECT * FROM point_levels ORDER BY z,id').fetchall();previous=set()
    for z in range(8,15):
     ids=set(r[0] for r in c.execute('SELECT id FROM point_levels WHERE z=?',(z,)));self.assertTrue(previous<=ids);previous=ids
     info=json.loads(c.execute('SELECT value FROM metadata WHERE key=?',(f'placement-info-z{z}',)).fetchone()[0]);self.assertFalse(info['max_error_guaranteed']);self.assertGreater(info['points'],0)
    self.assertEqual(c.execute("SELECT count(*) FROM tasks WHERE stage='fit'").fetchone()[0],0)
   run('place-scale')
   with closing(sqlite3.connect(out/'work.sqlite3')) as c:self.assertEqual(before,c.execute('SELECT * FROM point_levels ORDER BY z,id').fetchall())
if __name__=='__main__':unittest.main()

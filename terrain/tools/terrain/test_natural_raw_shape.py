"""LPF無効モードが元DEMを平滑化せず、誤差上限を監査することを確認。"""
from pathlib import Path
from contextlib import closing
import json,sqlite3,subprocess,tempfile,unittest
import numpy as np
from PIL import Image
from test_fixed_lpf_regions import decompress
ROOT=Path(__file__).resolve().parents[2];EXE=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-national.exe'
class RawShape(unittest.TestCase):
 def test_no_filter_and_audited_original_heights(self):
  with tempfile.TemporaryDirectory(dir=ROOT/'build') as temp:
   p=Path(temp);cache=p/'cache/14/14500';cache.mkdir(parents=True);source=p/'source.sqlite3';out=p/'out';cfg=json.loads((ROOT/'NATIONAL_NATURAL_SETTINGS.json').read_text(encoding='utf-8-sig'));cfg.update(lpf_policy='none',fit_reference='lpf',placement_policy='lpf-scale-poisson',placement_only_z=8);settings=p/'settings.json';settings.write_text(json.dumps(cfg))
   y,x=np.mgrid[:256,:256];v=(40000+20000*((x//8+y//8)%2)).astype(np.uint32);Image.fromarray(np.stack([v>>16,v>>8,v],axis=-1).astype(np.uint8)).save(cache/'6450.png')
   with closing(sqlite3.connect(source)) as c,c:c.execute('CREATE TABLE tiles(z,x,y)');c.execute('INSERT INTO tiles VALUES(14,14500,6450)')
   def run(mode,*extra):
    r=subprocess.run([str(EXE),'--'+mode,'--output',str(out),'--workers','2',*map(str,extra)],cwd=ROOT,capture_output=True,text=True,encoding='utf-8',timeout=120);self.assertEqual(r.returncode,0,r.stderr)
   run('inventory','--source',source,'--cache',p/'cache','--settings',settings);run('lpf')
   with closing(sqlite3.connect(out/'work.sqlite3')) as c:
    self.assertEqual(c.execute("SELECT value FROM metadata WHERE key='lpf-sigma-z8'").fetchone()[0],'0');path=c.execute("SELECT path FROM artifacts WHERE stage='lpf'").fetchone()[0];grid=np.frombuffer(decompress(Path(path).read_bytes()),'<f4',offset=16).reshape(1024,1024);part=grid[(6450%4)*256:(6450%4+1)*256,(14500%4)*256:(14500%4+1)*256];np.testing.assert_array_equal(part,v*.01)
   run('place-scale')
   with closing(sqlite3.connect(out/'work.sqlite3')) as c,c:c.execute("INSERT OR REPLACE INTO metadata VALUES('fit-phase-z8','fitting')");c.execute("INSERT OR REPLACE INTO metadata VALUES('fit-seeded-z8','true')")
   run('fit')
   with closing(sqlite3.connect(out/'work.sqlite3')) as c:
    audits=[(state,json.loads(info)) for state,info in c.execute("SELECT state,info FROM tasks WHERE stage='fit' AND z=8")];self.assertEqual(sum(a['valid'] for _,a in audits),65536);self.assertTrue(all(state=='passed' and a['maximum_error_bound']<=800 and a['reused_cells']==0 for state,a in audits))
if __name__=='__main__':unittest.main()

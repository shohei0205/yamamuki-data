"""12.5 mの共通曲面から7.25 mへ細かくし、1 kmタイルを追加する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,sqlite3,time,traceback
from pathlib import Path
from sibson_region128 import worker,write
from sibson_variable128 import generate,LEVELS,PARENTS,ROOT

OUTPUT=Path('build/sibson-7_25-v1')
def main():
 if not any(e==7.25 for e,_ in LEVELS):
  print("7.25 mは現在対象外です。既存の途中データは保持しています。")
  return
 OUTPUT.mkdir(exist_ok=True);guard=sqlite3.connect(OUTPUT/'run-lock.sqlite3',timeout=0);guard.execute('CREATE TABLE IF NOT EXISTS lock(id INTEGER)');guard.commit();guard.execute('BEGIN EXCLUSIVE');started=time.time()
 with sqlite3.connect(ROOT/'terrain.sqlite3',timeout=30) as db:
  config=json.loads(db.execute('SELECT value FROM metadata WHERE key="config"').fetchone()[0]);config['levels']=[dict(error=e,side=s,tiles=int(128000/s)**2,parent=PARENTS[e]) for e,s in LEVELS];db.execute('UPDATE metadata SET value=? WHERE key="config"',(json.dumps(config),))
 try:
  for region in ['yari','fuji','tsukuba']:
   write(OUTPUT/'progress.json',dict(stage='fitting',region=region,error=7.25,elapsedSeconds=time.time()-started));worker(region,model_only=True,errors=[12.5,7.25]);write(OUTPUT/'progress.json',dict(stage='tiling',region=region,error=7.25,elapsedSeconds=time.time()-started));generate(regions=[region],errors=[7.25])
  write(OUTPUT/'progress.json',dict(stage='complete',elapsedSeconds=time.time()-started))
 except Exception as error:
  write(OUTPUT/'progress.json',dict(stage='failed',error=str(error),elapsedSeconds=time.time()-started));traceback.print_exc();raise
 finally:guard.close()
if __name__=='__main__':main()

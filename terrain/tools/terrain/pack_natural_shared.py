"""共有曲面の支持点をRiceで保存し、SQLite実サイズを測る。"""
from pathlib import Path
import json,subprocess,argparse,sqlite3
ROOT=Path(__file__).resolve().parents[2]
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,default=ROOT/'build/natural-shared-packed-v1');a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
 source=ROOT/'build/natural-shared-128-v1';old=ROOT/'build/natural-xyz-hierarchy-z13-25-upquarter-local-v1';settings=json.loads((old/'settings.json').read_text());bounds={r['region']:r['bounds'] for r in settings['regions']};rows=json.loads((source/'summary-levels.json').read_text());jobs=[]
 for region in ['fuji','yari','tsukuba']:
  for r in sorted([r for r in rows if r['region']==region],key=lambda r:-r['error']):
   folder=source/region if r['error']==300 else source/region/'levels'/str(r['error'])
   jobs.append(dict(region=region,error=r['error'],z=r['z'],side=r['tileSide'],input=str(folder/'input.xyz64'),origin=bounds[region][:2]))
 path=a.output/'jobs.json';path.write_text(json.dumps(jobs,ensure_ascii=False),encoding='utf-8');exe=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-shared-pack.exe';raw=a.output/'support-points.bin'
 subprocess.run([str(exe),'--prepare','--input',str(path),'--output',str(raw)],check=True)
 subprocess.run([str(exe),'--input',str(raw),'--output',str(a.output)],check=True)
 # ページサイズをそろえ、管理情報を含む最終サイズ測定用ファイルを別に作る。
 final=a.output/'terrain-shared-xyz.sqlite3'
 if final.exists():raise RuntimeError('既存の測定ファイルを上書きしません')
 with sqlite3.connect(a.output/'hybrid.sqlite3') as source_db,sqlite3.connect(final) as dest_db:
  source_db.backup(dest_db)
  metadata=dict(format='natural-shared-xyz-size-trial-v1',xyStepM=.0625,zStepM=.125,crs='EPSG:3857 world-left-top east/south',levels=json.loads((source/'levels.json').read_text()),regions=settings['regions'],includesGPUAuxiliary=False,includesGlobalC1Gradient=False,demReaudited=False)
  for key,value in metadata.items():dest_db.execute('INSERT INTO metadata VALUES(?,?)',(key,json.dumps(value,ensure_ascii=False)))
  dest_db.commit();dest_db.execute('PRAGMA page_size=1024');dest_db.execute('VACUUM')
  assert dest_db.execute('PRAGMA integrity_check').fetchone()[0]=='ok'
 print('SQLite bytes',final.stat().st_size)

if __name__=='__main__':main()

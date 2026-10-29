"""全国と同じLPFを再利用し、3地域の128 kmを標準XYZ・全品質で試す。"""
from contextlib import closing
from concurrent.futures import ThreadPoolExecutor
import argparse,hashlib,json,math,os,shutil,sqlite3,subprocess,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
REGIONS=[('fuji',35.3606,138.7274),('yari',36.3419,137.6476),('tsukuba',36.2254,140.1067)]

def write(p,v):
 t=p.with_suffix('.next.json');t.write_text(json.dumps(v,ensure_ascii=False,indent=2)+'\n',encoding='utf-8',newline='\n');t.replace(p)
def run(args):
 parent=args.parent.resolve();engine=args.engine.resolve();started=time.monotonic()
 with closing(sqlite3.connect(parent/'regional-pilots-lock.sqlite3',timeout=0)) as guard:
  guard.execute('CREATE TABLE IF NOT EXISTS lock(id)');guard.commit();guard.execute('BEGIN EXCLUSIVE')
  def worker(region):
   name,lat,lon=region;output=ROOT/f'build/natural-xyz128-{name}-v1';output.mkdir(parents=True,exist_ok=True);binary=output/'bin'/('engine-'+hashlib.sha256(engine.read_bytes()).hexdigest()[:16]+'.exe');binary.parent.mkdir(exist_ok=True)
   if not binary.exists():shutil.copyfile(engine,binary)
   with closing(sqlite3.connect((parent/'work.sqlite3').as_uri()+'?mode=ro',uri=True)) as c:identity=json.loads(c.execute("SELECT value FROM metadata WHERE key='identity'").fetchone()[0]);global_manifest=c.execute("SELECT value FROM metadata WHERE key='input_manifest_sha256'").fetchone()[0]
   x=(lon+180)/360*16384;y=(1-math.asinh(math.tan(math.radians(lat)))/math.pi)/2*16384;half=args.span/2/(40075016.68557849/16384);window=[int(math.floor((x-half)/4)*4),int(math.floor((y-half)/4)*4),int(math.ceil((x+half)/4)*4),int(math.ceil((y+half)/4)*4)]
   write(output/'region.json',dict(region=name,latitude=lat,longitude=lon,requested_span_m=args.span,input_window=window,source_z=14,lpf_parent_source_manifest=global_manifest))
   def native(stage,*extra):
    tag=str(time.time_ns());out=output/f'{stage}-{tag}.stdout.log';err=output/f'{stage}-{tag}.stderr.log';env=os.environ.copy();temp=output/'tmp';temp.mkdir(exist_ok=True);env['TEMP']=env['TMP']=str(temp)
    with out.open('wb') as a,err.open('wb') as b:
     p=subprocess.Popen([str(binary),'--'+stage,'--output',str(output),'--workers',str(args.workers),*map(str,extra)],cwd=ROOT,stdout=a,stderr=b,env=env,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
    write(output/f'launch-{tag}.json',dict(pid=p.pid,stage=stage,executable=str(binary),stdout=str(out),stderr=str(err)))
    if p.wait():raise RuntimeError(f'{name} / {stage}: {err}')
   extra=['--resume'] if (output/'work.sqlite3').exists() else []
   native('inventory','--source',identity['source'],'--cache',identity['cache'],'--settings',parent/'settings.json','--window',*window,*extra)
   with closing(sqlite3.connect(output/'work.sqlite3')) as c,closing(sqlite3.connect((parent/'work.sqlite3').as_uri()+'?mode=ro',uri=True)) as source:
    blocks={(x//4,y//4) for x,y in c.execute('SELECT x,y FROM inputs')}
    with c:
     for z in range(8,15):
      sigma=source.execute('SELECT value FROM metadata WHERE key=?',(f'lpf-sigma-z{z}',)).fetchone()
      if sigma is None:raise RuntimeError('全国LPFの監査が未完了です')
      for bx,by in blocks:
       path,sha=source.execute("SELECT path,sha FROM artifacts WHERE stage='lpf' AND z=? AND x=? AND y=?",(z,bx,by)).fetchone();state,info=source.execute("SELECT state,info FROM tasks WHERE stage='lpf' AND z=? AND x=? AND y=?",(z,bx,by)).fetchone();c.execute("INSERT OR REPLACE INTO artifacts VALUES('lpf',?,?,?,?,?)",(z,bx,by,path,sha));c.execute("INSERT OR REPLACE INTO tasks VALUES('lpf',?,?,?,?,?)",(z,bx,by,state,info))
      c.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',(f'lpf-sigma-z{z}',sigma[0]))
     c.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('lpf-parent-source-manifest',global_manifest))
   native('fit');native('pack-tiles');native('verify');report=json.loads((output/'verification.json').read_text());report.update(region=name,requested_span_m=args.span,regional_geometry_verified=True,full_country_coverage=False,complete=False,distributable=False);write(output/'regional-test.json',report);return report
  with ThreadPoolExecutor(max_workers=3) as pool:reports=list(pool.map(worker,REGIONS))
  result=dict(stage='regional-tests-complete',reports=reports,elapsed_seconds=time.monotonic()-started,complete=False,distributable=False);write(parent/'regional-tests.json',result);return result

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--parent',type=Path,required=True);p.add_argument('--engine',type=Path,required=True);p.add_argument('--span',type=float,default=128000);p.add_argument('--workers',type=int,default=2);a=p.parse_args()
 if not 0<a.span<=200000 or not 1<=a.workers<=max(1,(os.cpu_count() or 1)//6):p.error('3地域の合計並列数を論理CPUの半分以下にしてください')
 print(json.dumps(run(a),ensure_ascii=False,indent=2))
if __name__=='__main__':main()

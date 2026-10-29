"""Rust全国生成の段階実行・再開。数値計算はRustへ渡す。"""
import argparse
from contextlib import closing,ExitStack
from datetime import datetime,timezone
import hashlib,json,os,shutil,sqlite3,subprocess,sys,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
class AlreadyRunning(RuntimeError):pass


def read(path):return json.loads(Path(path).read_text(encoding='utf-8'))
def write(path,value):
 path=Path(path);tmp=path.with_suffix('.next.json');tmp.write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n',encoding='utf-8',newline='\n');tmp.replace(path)
def sha(path):
 h=hashlib.sha256()
 with Path(path).open('rb') as f:
  for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
 return h.hexdigest()
def sql_value(output,key):
 with closing(sqlite3.connect((Path(output)/'work.sqlite3').resolve().as_uri()+'?mode=ro',uri=True)) as c:
  r=c.execute('SELECT value FROM metadata WHERE key=?',(key,)).fetchone();return None if r is None else r[0]
def wait_lock(file,callback):
 while Path(file).exists():
  try:
   with closing(sqlite3.connect(file,timeout=0)) as c:c.execute('BEGIN EXCLUSIVE');c.rollback()
   return
  except sqlite3.OperationalError as e:
   if 'locked' not in str(e):raise
   callback();time.sleep(5)
def free_space(output):
 if shutil.disk_usage(output).free<8*1024**3:raise RuntimeError('作業先の空き容量が8 GiB未満です。生成物を削除せず停止します')
def launch(output,argv,stage):
 tag=datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ');out=output/f'{stage}-{tag}.stdout.log';err=output/f'{stage}-{tag}.stderr.log';env=os.environ.copy();temp=output/'tmp';temp.mkdir(exist_ok=True)
 for key in ['TEMP','TMP','TMPDIR']:env[key]=str(temp)
 with out.open('wb') as a,err.open('wb') as b:
  p=subprocess.Popen([str(v) for v in argv],cwd=ROOT,stdout=a,stderr=b,env=env,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
 write(output/f'launch-{stage}-{tag}.json',dict(pid=p.pid,arguments=[str(v) for v in argv],stage=stage,started_utc=tag,stdout=str(out),stderr=str(err)))
 return p,err

def proof_state(output):
 path=output/'coverage-source.sqlite3'
 if not path.exists():return None,[]
 with closing(sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True)) as c:
  key=c.execute("SELECT value FROM metadata WHERE key='input_manifest_sha256'").fetchone()
  if key is None:return None,[]
  if key[0]!=sql_value(output,'input_manifest_sha256'):raise RuntimeError('取得確認と入力監査の世代が異なります')
  pending=c.execute("SELECT count(*) FROM cells WHERE state IN ('pending','error')").fetchone()[0]
  extra=c.execute("SELECT x,y,sha,bytes FROM cells WHERE state='verified200' ORDER BY y,x").fetchall()
 return pending,extra

def expanded_source(output,source,extra):
 rows=set()
 with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)) as c:rows.update(c.execute('SELECT x,y FROM tiles WHERE z=14'))
 rows.update((x,y) for x,y,*_ in extra);key=hashlib.sha256(json.dumps(sorted(rows)).encode()).hexdigest()[:12];path=output/f'expanded-input-index-{key}.sqlite3'
 if not path.exists():
  with closing(sqlite3.connect(path)) as c,c:
   c.execute('CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,PRIMARY KEY(z,x,y)) WITHOUT ROWID');c.executemany('INSERT INTO tiles VALUES(14,?,?)',sorted(rows))
 return path,output.with_name(output.name+'-expanded-'+key)

def copy_proof(old,new):
 target=new/'coverage-source.sqlite3'
 if target.exists():return
 with closing(sqlite3.connect((old/'coverage-source.sqlite3').resolve().as_uri()+'?mode=ro',uri=True)) as a,closing(sqlite3.connect(target)) as b:
  a.backup(b)
  with closing(sqlite3.connect((new/'work.sqlite3').resolve().as_uri()+'?mode=ro',uri=True)) as inputs:
   hashes=dict(((x,y),v) for x,y,v in inputs.execute('SELECT x,y,sha FROM inputs'))
  for x,y,v in b.execute("SELECT x,y,sha FROM cells WHERE state IN ('cached','verified200')"):
   if hashes.get((x,y))!=v:raise RuntimeError('再構成した入力と取得確認のSHA-256が一致しません')
  with b:
   b.execute("UPDATE cells SET state='cached' WHERE state='verified200'")
   b.execute("UPDATE metadata SET value=? WHERE key='input_manifest_sha256'",(sql_value(new,'input_manifest_sha256'),))
 write(new/'coverage-verification.json',dict(stage='coverage-inputs-complete',source_coverage_verified=True,new_inputs=[],complete=False,distributable=False,reused_from=str(old)))


def run(args):
 output=args.output.resolve();source=args.source.resolve();cache=args.cache.resolve();settings=args.settings.resolve();engine=args.engine.resolve();records=args.records.resolve();root_output=output
 output.mkdir(parents=True,exist_ok=True);maximum=max(1,(os.cpu_count() or 1)//2);workers=args.workers or maximum
 if not 1<=workers<=maximum:raise ValueError(f'並列数は1～論理CPUの半分（{maximum}）です')
 if not engine.is_file():raise FileNotFoundError('Rust生成器がありません。start_national_natural.ps1でビルドしてください')
 if (output/'work.sqlite3').exists() and not args.resume:raise ValueError('作業が存在します。--resumeで続けてください')
 def state(stage,**kw):
  record=dict(stage=stage,active_output=str(output),mode=args.mode,updated_utc=datetime.now(timezone.utc).isoformat(),workers=workers,complete=False);record.update(kw);write(root_output/'pipeline-progress.json',record)
 def native(stage,flag,*extra):
  free_space(output);wait_lock(output/'run-lock.sqlite3',lambda:state('waiting-for-existing-native-stage',requested_stage=stage))
  state(stage);argv=[engine,flag,'--output',output,'--workers',str(workers),*extra];p,err=launch(output,argv,stage)
  while p.poll() is None:
   if (root_output/'stop.request').exists():p.terminate();p.wait();state('stopped-by-user',child_pid=p.pid);raise RuntimeError('停止要求により中断しました。保存済み成果は保持しています')
   if shutil.disk_usage(output).free<8*1024**3:p.terminate();p.wait();raise RuntimeError('空き容量が8 GiB未満になったため停止しました')
   time.sleep(2)
  if p.returncode:raise RuntimeError(f'{stage}が失敗しました（終了コード{p.returncode}）。ログ: {err}')
 with ExitStack() as locks:
  for rebase in range(8):
   output.mkdir(parents=True,exist_ok=True);guard=locks.enter_context(closing(sqlite3.connect(output/'pipeline-lock.sqlite3',timeout=0)));
   try:guard.execute('CREATE TABLE IF NOT EXISTS lock(id)');guard.commit();guard.execute('BEGIN EXCLUSIVE')
   except sqlite3.OperationalError as e:
    if 'locked' in str(e):raise AlreadyRunning('同じ出力の段階実行が既に動いています') from e
    raise
   free_space(output);snapshot=output/'bin'/('terrain-natural-national-'+sha(engine)[:16]+engine.suffix);snapshot.parent.mkdir(exist_ok=True)
   if not snapshot.exists():shutil.copyfile(engine,snapshot)
   if sha(snapshot)!=sha(engine):raise RuntimeError('Rust実行ファイルのスナップショットが不一致です')
   engine=snapshot
   completed=read(output/'verification.json') if (output/'verification.json').exists() else None
   resume=['--resume'] if (output/'work.sqlite3').exists() else []
   native('inventory','--inventory','--source',source,'--cache',cache,'--settings',settings,*resume)
   if completed and completed.get('complete'):
    path=Path(completed['path'])
    if not path.is_file() or sha(path)!=completed['file_sha256']:raise RuntimeError('確定済みDBのSHA-256が不一致です')
    state('complete',result=completed,complete=True);return completed
   previous=read(output/'parent-job.json')['previous'] if (output/'parent-job.json').exists() else None
   if previous:copy_proof(Path(previous),output)
   pending,extra=proof_state(output);coverage_child=None
   if pending is None or pending>0:
    # 既に動いている取得確認はロックが解放されるまで待ち、二重起動しない。
    if not (output/'coverage-run-lock.sqlite3').exists():
     if not records.is_file():raise FileNotFoundError('初回の取得確認には元のdem-downloads.jsonlが必要です')
     coverage_child=launch(output,[sys.executable,'-X','utf8',ROOT/'tools/terrain/verify_natural_coverage_sources.py','--output',output,'--records',records,'--workers','4','--rate','8'],'coverage')
   native('lpf','--lpf')
   if coverage_child:
    p,err=coverage_child
    while p.poll() is None:
     state('waiting-for-source-coverage',child_pid=p.pid)
     if (root_output/'stop.request').exists():p.terminate();p.wait();raise RuntimeError('停止要求により取得確認を中断しました')
     time.sleep(5)
   wait_lock(output/'coverage-run-lock.sqlite3',lambda:state('waiting-for-source-coverage'))
   pending,extra=proof_state(output)
   if pending is None or pending>0:
    state('source-coverage');p,err=launch(output,[sys.executable,'-X','utf8',ROOT/'tools/terrain/verify_natural_coverage_sources.py','--output',output,'--records',records,'--workers','4','--rate','8'],'coverage-resume')
    while p.poll() is None:state('waiting-for-source-coverage');time.sleep(5)
    if p.returncode:raise RuntimeError(f'取得確認が失敗しました。ログ: {err}')
    pending,extra=proof_state(output)
   if pending!=0:raise RuntimeError('元DEMの未確認セルが残っています')
   if extra:
    new_source,new_output=expanded_source(output,source,extra);new_output.mkdir(parents=True,exist_ok=True);write(output/'next-job.json',dict(output=str(new_output),reason='追加で見つかった元DEMを含めて再生成',new_inputs=len(extra)));write(new_output/'parent-job.json',dict(previous=str(output)));output=new_output;source=new_source;continue
   if args.mode=='prepare':
    result=dict(stage='ready-to-generate',output=str(output),workers=workers,source_coverage_verified=True,all_lpf_qualities_audited=True,complete=False,distributable=False);write(output/'preparation.json',result);state('ready-to-generate',result=result);return result
   native('fit','--fit');native('pack','--pack-tiles');native('verify','--verify','--finalize');result=read(output/'verification.json');state('complete',result=result,complete=True);return result
  raise RuntimeError('入力範囲の再構成が8回を超えました。完了扱いにはしません')


def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--mode',choices=['prepare','generate'],default='prepare');p.add_argument('--source',type=Path,default=ROOT/'build/terrain-lod.sqlite3');p.add_argument('--cache',type=Path,default=ROOT/'build/terrain-cache');p.add_argument('--settings',type=Path,default=ROOT/'NATIONAL_NATURAL_SETTINGS.json');p.add_argument('--engine',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--records',type=Path,default=Path('M:/GDIData/yamamuki-terrain/records/dem-downloads.jsonl'));p.add_argument('--workers',type=int);p.add_argument('--resume',action='store_true');args=p.parse_args()
 try:print(json.dumps(run(args),ensure_ascii=False,indent=2))
 except Exception as error:
  if args.output.is_dir() and not isinstance(error,AlreadyRunning):
   report=dict(stage='failed',error=str(error),complete=False,updated_utc=datetime.now(timezone.utc).isoformat());write(args.output/'pipeline-error.json',report);write(args.output/'pipeline-progress.json',report)
  raise
if __name__=='__main__':main()

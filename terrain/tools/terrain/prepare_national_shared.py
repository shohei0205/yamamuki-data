"""現行共通曲面の全国生成準備。生成器は起動せず、入力と実装ゲートを記録する。"""
import argparse
from collections import Counter
from contextlib import closing
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
LEVELS = [(6,500),(7,400),(8,300),(9,250),(10,200),(11,125),(12,60),(13,25)]

def save_json(path, value):
    temporary = path.with_name(path.name + '.' + uuid.uuid4().hex + '.tmp')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2)+'\n', encoding='utf-8', newline='\n')
    try:
        for attempt in range(40):
            try:
                temporary.replace(path)
                return
            except PermissionError:
                if attempt == 39:
                    raise
                time.sleep(.05)
    finally:
        temporary.unlink(missing_ok=True)

def load_settings(path):
    raw = path.read_bytes()
    c = json.loads(raw)
    if c.get('profile') != 'sibson-shared-no-lpf-z6-z13' or [(v['z'],v['error_down_m']) for v in c['levels']] != LEVELS:
        raise ValueError('旧LPF方式・三地点用設定を全国準備へ混ぜられません')
    for v in c['levels']:
        if v['error_up_m'] != v['error_down_m']/4 or v['anchor'] != (v['z'] in (6,9,12)) or v['parent_z'] != (None if v['anchor'] else v['z']-1):
            raise ValueError('方向別誤差・アンカー・親段が不正です')
    if c['generation']['lpf'] or c['generation']['additional_vertex_pool'] or c['generation']['global_c1_solve']:
        raise ValueError('現行方式と異なる曲面設定です')
    return c, hashlib.sha256(raw).hexdigest()

def prepare(source, cache, output, settings, verify_png=True):
    source,cache,output,settings = [Path(v).resolve() for v in (source,cache,output,settings)]
    config, config_hash = load_settings(settings)
    if not source.is_file() or not cache.is_dir():
        raise ValueError('元DEM台帳とPNGキャッシュが必要です')
    if output == cache or cache in output.parents or output in cache.parents or output in source.parents:
        raise ValueError('出力を元データの中・親ディレクトリーに配置しないでください')
    output.mkdir(parents=True,exist_ok=True)
    identity = dict(source=str(source),cache=str(cache),settings_sha256=config_hash)
    if (output/'job.json').exists() and json.loads((output/'job.json').read_text(encoding='utf-8'))['identity'] != identity:
        raise ValueError('異なる入力・設定の作業先は再利用できません')
    with closing(sqlite3.connect(output/'engine-lock.sqlite3',timeout=0)) as guard:
        guard.execute('CREATE TABLE IF NOT EXISTS lock(id INTEGER)')
        guard.commit()
        guard.execute('BEGIN EXCLUSIVE')
        return inventory(source,cache,output,config,identity,verify_png)

def inventory(source,cache,output,config,identity,verify_png):
    start=time.monotonic()
    states=Counter(); issues=[]; tiles={z:{} for z,_ in LEVELS}; total_bytes=0; digest=hashlib.sha256()
    source_db=sqlite3.connect(source.as_uri()+'?mode=ro',uri=True)
    target=sqlite3.connect(output/'inputs.sqlite3')
    try:
        if source_db.execute('PRAGMA quick_check').fetchone()[0]!='ok':
            raise ValueError('入力台帳が破損しています')
        target.executescript('CREATE TABLE IF NOT EXISTS inputs(x INTEGER,y INTEGER,state TEXT,sha256 TEXT,bytes INTEGER,verified INTEGER,PRIMARY KEY(x,y)) WITHOUT ROWID; CREATE TABLE IF NOT EXISTS planned_tiles(z INTEGER,x INTEGER,y INTEGER,cached_inputs INTEGER,missing_inputs INTEGER,PRIMARY KEY(z,x,y)) WITHOUT ROWID;')
        target.execute('BEGIN IMMEDIATE')
        target.execute('DELETE FROM inputs');target.execute('DELETE FROM planned_tiles')
        count=source_db.execute('SELECT count(*) FROM cells').fetchone()[0]
        for done,(x,y,state,sha,size) in enumerate(source_db.execute('SELECT x,y,state,sha,bytes FROM cells ORDER BY y,x'),1):
            verified=False
            if not isinstance(x,int) or not isinstance(y,int) or not 0<=x<16384 or not 0<=y<16384:
                raise ValueError('z14タイル番号が不正です')
            if state not in ('cached','verified404'):
                issues.append(f'未確定入力: {x}/{y} {state}')
            if state=='cached':
                path=cache/'14'/str(x)/f'{y}.png'
                try:
                    if path.stat().st_size!=size or not isinstance(sha,str) or len(sha)!=64:
                        raise ValueError('サイズまたは台帳ハッシュが不正')
                    if verify_png:
                        raw=path.read_bytes()
                        if raw[:8]!=b'\x89PNG\r\n\x1a\n' or hashlib.sha256(raw).hexdigest()!=sha:
                            raise ValueError('PNG署名またはSHA256が不一致')
                        verified=True
                    total_bytes+=size
                except (OSError,ValueError) as exc:
                    issues.append(f'{x}/{y}: {exc}')
            states[state]+=1
            row=[x,y,state,sha,size]
            digest.update(json.dumps(row,separators=(',',':')).encode()+b'\n')
            target.execute('INSERT INTO inputs VALUES(?,?,?,?,?,?)',(*row,int(verified)))
            for z,_ in LEVELS:
                key=(x>>(14-z),y>>(14-z));counts=tiles[z].setdefault(key,[0,0]);counts[0 if state=='cached' else 1]+=1
            if done%4096==0:
                save_json(output/'progress.json',dict(stage='verify-inputs',done=done,total=count,issues=len(issues),bytes=total_bytes,pid=os.getpid(),elapsed_seconds=time.monotonic()-start))
        # 404だけの保存タイルは生成候補へ含めない。PNG内の有効画素は生成時に監査する。
        for z,rows in tiles.items():
            target.executemany('INSERT INTO planned_tiles VALUES(?,?,?,?,?)',((z,x,y,*counts) for (x,y),counts in sorted(rows.items()) if counts[0]>0))
        target.commit()
        levels=[]
        for v in config['levels']:
            rows=tiles[v['z']]
            levels.append(dict(**v,tile_side_projected_m=config['world_width_m']/2**v['z'],candidate_tiles=sum(c[0]>0 for c in rows.values()),verified404_only_tiles=sum(c[0]==0 for c in rows.values())))
        manifest_hash=digest.hexdigest()
        generation_id='NS1-'+hashlib.sha256((identity['settings_sha256']+manifest_hash).encode()).hexdigest()[:24]
        free=shutil.disk_usage(output).free
        blockers=[dict(gate=g,reason='この準備後の事前検証が必要です。qualify_national_shared.ps1で確認してください') for g in config['required_gates']]
        if not verify_png:blockers.append(dict(gate='png-integrity',reason='PNG全件SHA256照合を省略した'))
        if issues:blockers.append(dict(gate='input-integrity',reason=f'{len(issues)}件の入力不整合'))
        if free<config['resources']['minimum_free_disk_gib']*1024**3:blockers.append(dict(gate='free-disk',reason='最低空き容量未達'))
        job=dict(format='national-shared-prepared-v1',generation_id=generation_id,identity=identity,input_manifest_sha256=manifest_hash,prepared_utc=datetime.now(timezone.utc).isoformat(),prepared=True,inputs_verified=verify_png and not issues,ready_to_generate=False,complete=False,distributable=False,workers=max(1,(os.cpu_count() or 1)//2),levels=levels,states=dict(states),source_bytes=total_bytes,free_disk_bytes=free,blockers=blockers,issues=issues,settings=config)
        save_json(output/'job.json',job)
        save_json(output/'settings.json',config)
        save_json(output/'progress.json',dict(stage='prepared-not-started',done=count,total=count,ready_to_generate=False,blockers=blockers,elapsed_seconds=time.monotonic()-start))
        return job
    except BaseException as exc:
        target.rollback()
        save_json(output/'progress.json',dict(stage='prepare-failed',error=str(exc),ready_to_generate=False))
        raise
    finally:
        source_db.close();target.close()

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source',type=Path,default=ROOT/'build/terrain-national-natural-20261029/coverage-source.sqlite3')
    p.add_argument('--cache',type=Path,default=ROOT/'build/terrain-cache')
    p.add_argument('--output',type=Path,default=ROOT/'build/national-shared-z6-z13-v1')
    p.add_argument('--settings',type=Path,default=ROOT/'NATIONAL_SHARED_SETTINGS.json')
    p.add_argument('--skip-png-hash',action='store_true',help='開発用。全国開始の入力ゲートは通過しない')
    a=p.parse_args();job=prepare(a.source,a.cache,a.output,a.settings,not a.skip_png_hash)
    print(json.dumps({k:job[k] for k in ('generation_id','inputs_verified','ready_to_generate','states','source_bytes','workers','levels','blockers')},ensure_ascii=False,indent=2))
    if job['issues']:raise SystemExit(2)
if __name__=='__main__':main()

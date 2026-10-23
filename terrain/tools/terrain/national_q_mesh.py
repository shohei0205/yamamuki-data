"""新しいQ定義で全国基本TINを生成する。追加・境界接続前の作業DB。"""
import argparse,hashlib,json,os,shutil,sqlite3,struct,tempfile,time,zlib
from pathlib import Path
from concurrent.futures import ProcessPoolExecutor,wait,FIRST_COMPLETED
import numpy as np
from PIL import Image
import national_independent_mesh as national
from error_budget_mesh import base_mesh
from q_mesh_settings import definitions

ROOT=national.ROOT

def catalog_keys(parent_keys,width):
    if width<8000:
        factor=round(8000/width)
        return sorted({(x*factor+dx,y*factor+dy) for x,y in parent_keys for dx in range(factor) for dy in range(factor)})
    factor=round(width/8000)
    return sorted({(x//factor,y//factor) for x,y in parent_keys})

def initialize(source,binary):
    national.initialize(source,binary)

def sample_grid(x,y,width,spacing=31.25):
    n=round(width/spacing)+1;grid=np.empty((n,n),np.float32);valid=np.empty((n,n),bool)
    axis=np.arange(n,dtype=np.float64)*spacing
    # 512 kmタイルでも全国の座標配列を一度に展開しない。
    for start in range(0,n,16):
        xx,yy=np.meshgrid(x*width+axis,y*width+axis[start:start+16])
        grid[start:start+16],valid[start:start+16]=national.source_samples(xx,yy)
    return grid,valid

def available_memory_bytes():
    import ctypes
    memory=ctypes.create_string_buffer(64);struct.pack_into('<I',memory,0,64)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(memory):raise OSError('空きメモリーを取得できません')
    return struct.unpack_from('<Q',memory,16)[0]

def estimated_job_bytes(level):
    n=round(level['widthMeters']/level['sampleSpacingMeters'])+1
    return n*n*80+256*2**20

def admission_allowed(reservations,estimate,budget):
    # 最大サイズの1枚は既存のワーカー側検査へ渡す。同時には他の仕事を入れない。
    return not reservations or sum(reservations)+estimate<=budget

def generate(job):
    level,x,y=job;width=level['widthMeters'];spacing=level['sampleSpacingMeters']
    n=round(width/spacing)+1
    if n*n*80>available_memory_bytes()*.7:
        raise MemoryError(f'Q{level["level"]}の格子評価には空きメモリーが不足しています。条件を変えず停止します')
    grid,valid=sample_grid(x,y,width,spacing)
    if not valid.any():return None
    grid=np.rint(grid*4)/4;offset=min(0,float(grid.min()))
    with tempfile.TemporaryDirectory(prefix='national-q-basic-',dir=ROOT/'build') as temp:
        folder=Path(temp)
        Image.fromarray(np.rint((grid-offset)*4).astype(np.uint16)).save(folder/'height.png')
        region=dict(folder=folder,grid=grid,height_offset=offset,area=dict(id=f"{level['name']}/{x}/{y}"))
        points,faces=base_mesh(region,national.BINARY,folder,error=level['maxErrorMeters'])
    # 作業用の頂点もメートル座標で保存。DEMの傾き・法線は保存しない。
    points=np.asarray(points,dtype='<f4');points[:,:2]*=spacing;faces=np.asarray(faces,dtype='<u4')
    mesh=zlib.compress(struct.pack('<4sII',b'YQB1',len(points),len(faces))+points.tobytes()+faces.tobytes(),6)
    # 有効域は誤差検証用の別列。メッシュを切り抜く描画データではない。
    validity=zlib.compress(np.packbits(valid,bitorder='little').tobytes(),6)
    return len(points),len(faces),mesh,validity

def run(args):
    output=Path(args.output);output.parent.mkdir(parents=True,exist_ok=True)
    binary=Path(args.binary).resolve();source=Path(args.source).resolve();levels=definitions()
    digest=hashlib.sha256(binary.read_bytes()).hexdigest()
    pinned=output.parent/'terrain-q-binaries'/digest/binary.name;pinned.parent.mkdir(parents=True,exist_ok=True)
    if not pinned.exists():shutil.copy2(binary,pinned)
    config=dict(version=1,levels=levels,stage='basic-before-national-pool-and-boundary-connection',
        source=str(source),sourceSize=source.stat().st_size,sourceMtimeNs=source.stat().st_mtime_ns,
        binarySha256=digest,coastlineInMesh=False,demSlopeHints=False,demVertexNormals=False,
        pruningEnabled=False,additionalScope='national-global-pool',additionalFraction=.2,
        distributable=False,boundaryConnectionComplete=False,additionalComplete=False)
    fingerprint=hashlib.sha256(json.dumps(config,sort_keys=True).encode()).hexdigest();started=time.time()
    if args.workers<1:raise ValueError('並列数は1以上を指定してください')
    memory_budget=int(available_memory_bytes()*.55);pending={}
    with sqlite3.connect(output) as db:
        db.execute('PRAGMA journal_mode=WAL');db.execute('PRAGMA synchronous=NORMAL')
        db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS jobs(q INTEGER,x INTEGER,y INTEGER,state INTEGER DEFAULT 0,failure TEXT,PRIMARY KEY(q,x,y));CREATE TABLE IF NOT EXISTS basic(q INTEGER,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,mesh BLOB,source_validity BLOB,PRIMARY KEY(q,x,y));')
        previous=db.execute('SELECT value FROM metadata WHERE key="fingerprint"').fetchone()
        if previous and previous[0]!=fingerprint:raise ValueError('条件が違うため既存DBには追記しません')
        db.execute('INSERT OR REPLACE INTO metadata VALUES("manifest",?)',(json.dumps(config),))
        db.execute('INSERT OR REPLACE INTO metadata VALUES("fingerprint",?)',(fingerprint,))
        if not db.execute('SELECT count(*) FROM jobs').fetchone()[0]:
            with sqlite3.connect(Path(args.catalog).resolve().as_uri()+'?mode=ro',uri=True) as catalog:
                parents=catalog.execute('SELECT x,y FROM jobs WHERE error=800 AND state=1').fetchall()
            if not parents:raise ValueError('全国目録の標高ありタイルが空です')
            for level in levels:
                keys=catalog_keys(parents,level['widthMeters'])
                db.executemany('INSERT INTO jobs(q,x,y) VALUES(?,?,?)',[(level['level'],x,y) for x,y in keys])
        db.commit()
        def status(stage):
            info=dict(stage=stage,pid=os.getpid(),updated=time.time(),elapsedSeconds=time.time()-started,
                levels=levels,workerLimit=args.workers,activeWorkers=len(pending),reservedWorkerBytes=sum(v[1] for v in pending.values()),workerMemoryBudgetBytes=memory_budget,jobs=db.execute('SELECT q,state,count(*) FROM jobs GROUP BY q,state ORDER BY q,state').fetchall(),
                sizes=db.execute('SELECT q,count(*),sum(vertices),sum(triangles),sum(length(mesh)),sum(length(source_validity)) FROM basic GROUP BY q ORDER BY q').fetchall(),
                databaseBytes=output.stat().st_size,freeBytes=shutil.disk_usage(output.parent).free,
                complete=False,distributable=False,pendingStages=['national-global-pool-20-percent','cross-tile-and-cross-level-boundary-connection','final-verification'])
            progress=output.with_suffix('.progress.json');temporary=progress.with_suffix('.tmp')
            temporary.write_text(json.dumps(info,ensure_ascii=False,indent=2),encoding='utf-8');temporary.replace(progress)
            if info['freeBytes']<args.min_free_gib*2**30:raise RuntimeError('空き容量が停止条件を下回りました。保存済みタイルから再開できます')
        status('basic-generating')
        keys=db.execute('SELECT q,x,y FROM jobs WHERE state=0 ORDER BY q,y,x').fetchall()
        if args.limit:keys=keys[:args.limit]
        cursor=0;spec={l['level']:l for l in levels}
        with ProcessPoolExecutor(max_workers=args.workers,initializer=initialize,initargs=(str(source),str(pinned))) as pool:
            def enqueue():
                nonlocal cursor
                while cursor<len(keys) and len(pending)<args.workers:
                    key=keys[cursor];estimate=estimated_job_bytes(spec[key[0]])
                    if not admission_allowed([v[1] for v in pending.values()],estimate,memory_budget):break
                    pending[pool.submit(generate,(spec[key[0]],key[1],key[2]))]=(key,estimate);cursor+=1
            enqueue()
            while pending:
                done,_=wait(pending,timeout=10,return_when=FIRST_COMPLETED)
                for future in done:
                    key,_=pending.pop(future)
                    try:
                        result=future.result()
                        if result:db.execute('INSERT INTO basic VALUES(?,?,?,?,?,?,?)',(*key,*result))
                        db.execute('UPDATE jobs SET state=? WHERE q=? AND x=? AND y=?',(1 if result else 2,*key));db.commit()
                        print('基本タイル保存',key,'頂点',result[0] if result else 0,flush=True)
                    except Exception as exc:
                        db.execute('UPDATE jobs SET state=3,failure=? WHERE q=? AND x=? AND y=?',(str(exc),*key));db.commit()
                        status('basic-failed');raise
                enqueue();status('basic-generating')
        remaining=db.execute('SELECT count(*) FROM jobs WHERE state IN (0,3)').fetchone()[0]
        status('basic-partial' if remaining else 'basic-ready-for-national-pool')

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--source',default=str(ROOT/'build/terrain-lod.sqlite3'))
    parser.add_argument('--catalog',default=str(ROOT/'build/terrain-error-national-base.sqlite3'))
    parser.add_argument('--output',default=str(ROOT/'build/terrain-q4-q12-basic.sqlite3'))
    parser.add_argument('--binary',default=str(ROOT/'build/terrain-q-rust-target/release/terrain-adaptive-tin.exe'))
    parser.add_argument('--workers',type=int,default=4);parser.add_argument('--min-free-gib',type=float,default=8)
    parser.add_argument('--limit',type=int);args=parser.parse_args()
    # 同じ出力への二重起動を、プロセス終了時に解放されるOSロックで防ぐ。
    lock=Path(args.output).with_suffix('.lock');lock.parent.mkdir(parents=True,exist_ok=True)
    with lock.open('a+b') as handle:
        handle.seek(0);handle.write(b'0');handle.flush();handle.seek(0)
        import msvcrt
        msvcrt.locking(handle.fileno(),msvcrt.LK_NBLCK,1)
        try:run(args)
        except Exception as exc:
            Path(args.output).with_suffix('.failure.json').write_text(json.dumps(dict(error=str(exc),updated=time.time()),ensure_ascii=False),encoding='utf-8');raise

if __name__=='__main__':main()

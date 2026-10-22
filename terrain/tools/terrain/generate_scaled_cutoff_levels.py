"""全6品質を、誤差ごとの追加下限あり・なしで生成して比較する。"""
import hashlib,json,sqlite3,subprocess
from contextlib import closing
from national_independent_mesh import ROOT
from verify_shared_allocation import verify
from verify_error_detail20 import verify_mixed

ERRORS=[25,50,100,200,400,800]
REPORT=ROOT/'build/terrain-scaled-cutoff-levels.json'
PROGRESS=ROOT/'build/terrain-scaled-cutoff-levels.progress.json'


def mode(family,error):return 'error-detail20-'+family+('' if error==25 else '-e'+str(error))
def path(name):return ROOT/f'build/terrain-shared-allocation-{name}.sqlite3'


def manifest(name):
    p=path(name)
    if not p.exists():return None
    with closing(sqlite3.connect(p.resolve().as_uri()+'?mode=ro',uri=True)) as db:
        row=db.execute('select value from metadata where key="manifest"').fetchone()
        return json.loads(row[0]) if row else None


def complete(name):
    for suffix in ['', '-mixed']:
        m=manifest(name+suffix)
        if not m or m['sharedAllocation'].get('mode')!=name+suffix or not m['sharedAllocation'].get('generationComplete'):return False
    return True


def progress(stage,**values):
    row=dict(stage=stage,**values);PROGRESS.write_text(json.dumps(row,ensure_ascii=False),encoding='utf-8');print(stage,values,flush=True)


def reuse_25m():
    # 下限10 mの既存成果は同じ条件なので、BLOBを変更せず新しい品質系列へ登録する。
    name=mode('cutoff',25)
    if complete(name):return
    for suffix in ['', '-mixed']:
        target=path(name+suffix)
        if target.exists():raise ValueError('途中の出力を上書きしません: '+str(target))
        with closing(sqlite3.connect(path('error-detail20-cut10'+suffix).resolve().as_uri()+'?mode=ro',uri=True)) as src,closing(sqlite3.connect(target)) as db,db:
            src.backup(db);m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);a=m['sharedAllocation'];assert a['generationComplete'] and a['errorDetailPolicy']['minimumAdditionalHeightErrorMeters']==10
            a['mode']=name+suffix;a['errorDetailPolicy']['minimumAdditionalHeightErrorRule']='max(10,baseError/4)'
            db.execute('update metadata set value=? where key="manifest"',(json.dumps(m),))


def digest(name):
    h=hashlib.sha256();size=0
    with closing(sqlite3.connect(path(name).resolve().as_uri()+'?mode=ro',uri=True)) as db:
        for l,x,y,body in db.execute('select level,x,y,data from topology order by level,x,y'):
            h.update(f'{l}/{x}/{y}:'.encode());h.update(body);size+=len(body)
    return h.hexdigest(),size


def verify_quality(error):
    rows={}
    for family in ['perceptual','cutoff']:
        name=mode(family,error);m=manifest(name);assert complete(name)
        a=m['sharedAllocation'];policy=a['errorDetailPolicy'];assert policy['baseErrorMeters']==error and policy['additionalBudget']==policy['basicVertices']//5 and policy['additionalUsed']<=policy['additionalBudget']
        if family=='cutoff':
            threshold=max(10,error/4);assert policy['minimumAdditionalHeightErrorMeters']==threshold
            assert not policy['additionalUsed'] or policy['minimumInsertedHeightErrorMeters']>threshold
        for r in a['meshQuality']['regions'].values():assert r['maximum']<=error+1e-5 and r['minimumAngle']>=10.-1e-6
        uniform=verify(name);mixed=verify_mixed(name);fingerprint,blob_bytes=digest(name)
        rows[family]=dict(mode=name,sourceVertices=a['sourceVertices'],policy=policy,regions=a['meshQuality']['regions'],sqliteBytes=path(name).stat().st_size,mixedSqliteBytes=path(name+'-mixed').stat().st_size,topologyBytes=blob_bytes,topologySHA256=fingerprint,uniformTiles=uniform['tiles'],mixed=mixed)
    assert rows['perceptual']['policy']['basicVertices']==rows['cutoff']['policy']['basicVertices']
    return dict(errorMeters=error,minimumAdditionalErrorMeters=max(10,error/4),topologyEqual=rows['perceptual']['topologySHA256']==rows['cutoff']['topologySHA256'],**rows)


def main():
    reuse_25m();report=[]
    for error in reversed(ERRORS):
        for family in ['perceptual','cutoff']:
            name=mode(family,error)
            if complete(name):progress('既存の完成データを保持',mode=name);continue
            if path(name).exists():raise ValueError('途中の出力を上書きしません: '+str(path(name)))
            progress('品質を生成',mode=name,error=error,minimum=max(10,error/4) if family=='cutoff' else 0)
            args=['--perceptual'] if family=='perceptual' else ['--scaled-cutoff']
            with open(ROOT/f'build/terrain-{name}.log','w',encoding='utf-8') as log,open(ROOT/f'build/terrain-{name}.err','w',encoding='utf-8') as err:
                subprocess.run([str(ROOT/'build/terrain-venv/Scripts/python.exe'),'-X','utf8','-u',str(ROOT/'tools/terrain/generate_error_detail20.py'),'--error',str(error),*args],cwd=ROOT,stdout=log,stderr=err,check=True)
        row=verify_quality(error);report.append(row);REPORT.write_text(json.dumps(sorted(report,key=lambda r:r['errorMeters']),ensure_ascii=False,indent=2),encoding='utf-8')
        progress('品質の生成と検証完了',error=error,minimum=row['minimumAdditionalErrorMeters'],before=row['perceptual']['sourceVertices'],after=row['cutoff']['sourceVertices'],identical=row['topologyEqual'])
    report.sort(key=lambda r:r['errorMeters'])
    for family in ['perceptual','cutoff']:
        counts=[r[family]['sourceVertices'] for r in report];assert all(a>b for a,b in zip(counts,counts[1:]))
    progress('全品質の生成と検証完了',complete=True,errors=ERRORS,report=str(REPORT))

if __name__=='__main__':main()

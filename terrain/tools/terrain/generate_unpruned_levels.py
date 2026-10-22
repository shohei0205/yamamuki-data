"""不要点を削減せず、12.5～800 mの全品質を生成する。"""
import json,subprocess
from generate_scaled_cutoff_levels import ROOT,manifest,complete,path
from verify_shared_allocation import verify
from verify_error_detail20 import verify_mixed
ERRORS=[12.5,25,50,100,200,400,800]
REPORT=ROOT/'build/terrain-unpruned-levels.json'
PROGRESS=ROOT/'build/terrain-unpruned-levels.progress.json'
def mode(error):return 'error-detail20-direct'+('' if error==25 else '-e'+format(error,'g'))
def progress(stage,**values):
    row=dict(stage=stage,**values);PROGRESS.write_text(json.dumps(row,ensure_ascii=False),encoding='utf-8');print(stage,values,flush=True)
def main():
    report=[]
    for error in [800,400,200,100,50,25,12.5]:
        name=mode(error)
        if not complete(name):
            if path(name).exists():raise ValueError('途中の出力を上書きしません: '+str(path(name)))
            progress('品質を生成',mode=name,error=error)
            with open(ROOT/f'build/terrain-{name}.log','w',encoding='utf-8') as log,open(ROOT/f'build/terrain-{name}.err','w',encoding='utf-8') as err:
                subprocess.run([str(ROOT/'build/terrain-venv/Scripts/python.exe'),'-X','utf8','-u',str(ROOT/'tools/terrain/generate_error_detail20.py'),'--error',str(error),'--no-prune'],cwd=ROOT,stdout=log,stderr=err,check=True)
        m=manifest(name);a=m['sharedAllocation'];policy=a['errorDetailPolicy']
        assert not policy['pruningEnabled'] and policy['minimumAdditionalHeightErrorMeters']==0
        assert policy['baseErrorMeters']==error and policy['additionalBudget']==policy['basicVertices']//5
        assert policy['additionalUsed']<=policy['additionalBudget']
        for r in a['meshQuality']['regions'].values():
            assert r['pruning']['removed']==0 and r['pruning']['trials']==0
            assert r['sourceVertices']==r['basicVerticesBeforePruning']+r['addedBeforePruning']
            assert r['maximum']<=error+1e-5 and r['minimumAngle']>=10-1e-6
        uniform=verify(name);mixed=verify_mixed(name)
        report.append(dict(errorMeters=error,mode=name,sourceVertices=a['sourceVertices'],policy=policy,regions=a['meshQuality']['regions'],sqliteBytes=path(name).stat().st_size,mixedSqliteBytes=path(name+'-mixed').stat().st_size,uniform=uniform,mixed=mixed))
        REPORT.write_text(json.dumps(sorted(report,key=lambda r:r['errorMeters']),ensure_ascii=False,indent=2),encoding='utf-8')
        progress('品質の生成と検証完了',error=error,vertices=a['sourceVertices'])
    counts=[r['sourceVertices'] for r in sorted(report,key=lambda r:r['errorMeters'])];assert all(a>b for a,b in zip(counts,counts[1:]))
    progress('全品質の生成と検証完了',complete=True,errors=ERRORS,report=str(REPORT))
if __name__=='__main__':main()

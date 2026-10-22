"""最上位25 m版を保持し、50〜800 mの独立した品質レベルを生成する。"""
import json,sqlite3,subprocess,time
from pathlib import Path
from national_independent_mesh import ROOT
ERRORS=[25,50,100,200,400,800]
def mode(error):return 'error-detail20'+('' if error==25 else '-e'+str(error))
def collect():
    result=[]
    for quality,error in enumerate(ERRORS):
        path=ROOT/f'build/terrain-shared-allocation-{mode(error)}.sqlite3'
        if not path.exists():continue
        with sqlite3.connect(path) as db:
            row=db.execute('select value from metadata where key="manifest"').fetchone()
            if not row:continue
            m=json.loads(row[0]);a=m['sharedAllocation']
            if not a.get('generationComplete'):continue
            result.append(dict(quality=quality,errorMeters=error,mode=mode(error),sourceVertices=a['sourceVertices'],sqliteBytes=path.stat().st_size,policy=a['errorDetailPolicy'],regions=a['meshQuality']['regions']))
    (ROOT/'build/terrain-error-quality-levels.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8');return result

def main():
    for error in reversed(ERRORS[1:]):
        path=ROOT/f'build/terrain-shared-allocation-{mode(error)}.sqlite3'
        if path.exists():
            rows=collect()
            if any(r['errorMeters']==error for r in rows):print('既存品質を保持',error,flush=True);continue
            raise ValueError('途中の出力を上書きしません: '+str(path))
        print('品質レベルを生成',error,flush=True)
        with open(ROOT/f'build/terrain-{mode(error)}.log','w',encoding='utf-8') as log,open(ROOT/f'build/terrain-{mode(error)}.err','w',encoding='utf-8') as err:
            subprocess.run([str(ROOT/'build/terrain-venv/Scripts/python.exe'),'-X','utf8','-u',str(ROOT/'tools/terrain/generate_error_detail20.py'),'--error',str(error)],stdout=log,stderr=err,check=True,cwd=ROOT)
        rows=collect();print('品質レベル完了',error,[(r['errorMeters'],r['sourceVertices']) for r in rows],flush=True)
    print('全品質の生成完了',flush=True)
if __name__=='__main__':main()

"""追加候補の有無だけを変え、実タイルの接合品質と容量を比較する。"""
import json,subprocess,time
from pathlib import Path
import overlap_compare as overlap
from test_overlap_compare import check

def run():
 rows=[]
 configurations=[('dual',[50,100]),('dual',[12.5,800]),('quad',[25,50,100,200]),('quad',[800,12.5,800,12.5]),('tee',[200,50,100]),('tee',[800,12.5,25]),('partial',[200,50,400,100]),('partial',[800,12.5,25,400])]
 for region in ['fuji','yari','tsukuba']:
  for layout,errors in configurations:
   for width in [500,1000,2000,4000]:
    tiles=[overlap.prepare_tile(region,e,b,width)[0] for e,b in zip(errors,overlap.bounds(layout))]
    row=dict(region=region,layout=layout,errors=errors,width=width)
    for candidates in (True,False):
     selected=tiles if candidates else [dict(t,support=[]) for t in tiles]
     request=json.dumps(dict(tiles=selected,width=width,local=True,partial=False),separators=(',',':')).encode()
     result=subprocess.run([str(overlap.ROOT/'build/terrain-q-cache-target/release/terrain-overlap-compare.exe')],input=request,capture_output=True,creationflags=subprocess.CREATE_NO_WINDOW)
     key='withCandidates' if candidates else 'withoutCandidates'
     if result.returncode:
      row[key]=dict(error=result.stderr.decode(errors='replace'));continue
     mesh=json.loads(result.stdout)
     try:check(mesh,selected);topology=True
     except AssertionError as exc:topology=False;mesh['validationError']=repr(exc)
     row[key]=dict(mesh['stats'],topologyValid=topology,inputBytes=len(request),validationError=mesh.get('validationError'))
    rows.append(row)
   print(region,layout,errors,'完了',flush=True)
  (overlap.OUT/'support-comparison.json').write_text(json.dumps(rows,ensure_ascii=False,indent=2),encoding='utf-8')
 print('全',len(rows),'条件',flush=True)
 for key in ['withCandidates','withoutCandidates']:
  valid=[r[key] for r in rows if 'error' not in r[key]]
  print(key,'実行成功',len(valid),'接続成功',sum(r['topologyValid'] for r in valid),'角度合格',sum(r['qualitySatisfied'] for r in valid),'最小角',min(r['minimumAngle'] for r in valid),'入力合計',sum(r['inputBytes'] for r in valid),flush=True)
if __name__=='__main__':run()

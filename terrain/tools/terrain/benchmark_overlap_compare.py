"""全体分割と局所分割を同じ保存タイルで比較する。"""
import json,statistics
from overlap_compare import make,prepare_tile,bounds,OUT
from test_overlap_compare import check

def run(iterations=5):
 rows=[]
 for region in ['fuji','yari','tsukuba']:
  for label,errors in [('partial',[200,50,400,100]),('dense',[400,12.5,25,25])]:
   tiles=[prepare_tile(region,e,b,2000)[0] for e,b in zip(errors,bounds('partial'))]
   result={}
   for local in (False,True):
    samples=[]
    for i in range(iterations):
     m=make(region,errors,'partial',width=2000,local=local);samples.append(m['stats']['runtimeMs'])
     if i==0:check(m,tiles)
    result['local' if local else 'global']=dict(m['stats'],medianMs=statistics.median(samples))
   assert result['local']['coreFaces']==result['global']['coreFaces']
   row=dict(region=region,case=label,errors=errors,**result);rows.append(row);print(region,label,'分割頂点',result['global']['triangulationVertices'],'->',result['local']['triangulationVertices'],'計算中央値ms',round(result['global']['medianMs'],2),'->',round(result['local']['medianMs'],2),flush=True)
 (OUT/'local-benchmark.json').write_text(json.dumps(rows,indent=2),encoding='utf-8')
 return rows
if __name__=='__main__':run()

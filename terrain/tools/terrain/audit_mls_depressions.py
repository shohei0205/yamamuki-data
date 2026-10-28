"""槍ヶ岳の局所的な凹みを保存点・MLS・描画場で照合する。"""
import json
from pathlib import Path
import numpy as np
from scipy.interpolate import LinearNDInterpolator
from mls_quant_codec import decode_quant
from mls_smooth_radius import model_from_metadata
from free_mls import operator
from mls_lpf_dem import FilteredDem
out=[]
for root,kind in [('mls-six-point-v3','50-six-trained'),('mls-xy-placement-v1','50-six-xy'),('mls-xy-placement-v1','50-six-more')]:
 folder=Path('build')/root/'yari'/kind
 info=json.loads((folder/'info.json').read_text());p=decode_quant((folder/'model.mlq1').read_bytes());m=model_from_metadata(p,info)
 axis=np.linspace(-4000,4000,1025);x,y=np.meshgrid(axis,axis);q=np.column_stack((x.ravel(),y.ravel()))
 field=np.fromfile(Path('build/mls-generated-preview/dem-regions/yari')/(kind+'-8000.bin'),'<f4').reshape(-1,3)
 # 画面の全高さを復元値と照合し、凹み最大候補を特定する。
 actual=m(q);target=FilteredDem('yari',info['sigmaMeters']).sample(q)
 linear_field=LinearNDInterpolator(p[:,:2],p[:,2])(q)
 idx=int(np.argmin(actual-linear_field));point=q[idx:idx+1]
 a=operator(p[:,:2],point,m.support,curvature=1e-5,tree=m.support.tree)
 ids=a.indices;w=a.data;heights=p[ids,2];linear=float(LinearNDInterpolator(p[:,:2],p[:,2])(point)[0])
 neighbors=np.column_stack((p[ids],w)).tolist()
 check=q[np.random.default_rng(17).choice(len(q),2048,replace=False)]
 py=operator(p[:,:2],check,m.support,curvature=1e-5,tree=m.support.tree)@p[:,2]
 result=dict(kind=kind,xy=point[0].tolist(),height=float(actual[idx]),targetHeight=float(target[idx]),linearHeight=linear,deltaFromTarget=float(actual[idx]-target[idx]),supportRadius=float(m.support(point)[0]),neighborCount=len(ids),neighborMinHeight=float(heights.min()),neighborMaxHeight=float(heights.max()),weightSum=float(w.sum()),negativeWeightSum=float(w[w<0].sum()),minimumWeight=float(w.min()),maximumWeight=float(w.max()),displayMaxDifference=float(np.max(abs(field[:,0]-actual))),operatorMaxDifference=float(np.max(abs(py-m(check)))),neighbors=neighbors)
 out.append(result);print(json.dumps({k:v for k,v in result.items() if k!='neighbors'}),flush=True)
Path('build/mls-xy-placement-v1/depression-linear-audit.json').write_text(json.dumps(out,indent=2),encoding='utf-8')

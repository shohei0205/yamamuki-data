"""従来版と平坦部抑制版の、共通プール追加点の配分を比較する。"""
import json
from pathlib import Path
import numpy as np
from detail_priority import priority_gain
from national_independent_mesh import ROOT


def audit():
    result={}
    for name in ['fuji','tsukuba','yari']:
        with np.load(ROOT/f'build/terrain-quality-source-cache-v1/{name}.npz') as source:
            gain=priority_gain(source['grid'],source['terrain_weight'],source['valid'])
            gate=gain/(1.+.5*np.clip(source['terrain_weight'],0,1))
        rows={}
        for label,prefix in [('previous','terrain-error20'),('perceptual','terrain-error20-perceptual')]:
            folder=ROOT/f'build/{prefix}-{name}'
            with np.load(folder/'base-quality.npz') as data:base=len(data['p'])
            with np.load(folder/'pruned.npz') as data:
                p=data['p'][data['ids']>=base];xy=p[:,:2].astype(int);weights=gate[xy[:,1],xy[:,0]]
            rows[label]=dict(addedVertices=len(p),flatVertices=int((weights<.1).sum()),flatFraction=float(np.mean(weights<.1)),medianTerrainGate=float(np.median(weights)))
        result[name]=rows
    path=ROOT/'build/terrain-detail-priority-comparison.json'
    path.write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(result,ensure_ascii=False),flush=True)
    return result

if __name__=='__main__':audit()

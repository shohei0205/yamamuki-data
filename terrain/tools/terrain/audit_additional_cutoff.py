"""10 m下限の適用記録と、従来版からの形状の変化を検証する。"""
import json,sqlite3
import numpy as np
from national_independent_mesh import ROOT


def audit():
    with sqlite3.connect(ROOT/'build/terrain-shared-allocation-error-detail20-cut10.sqlite3') as db:
        m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0])
    policy=m['sharedAllocation']['errorDetailPolicy']
    assert policy['minimumAdditionalHeightErrorMeters']==10
    assert policy['minimumInsertedHeightErrorMeters']>10
    assert policy['additionalUsed']<=policy['additionalBudget']
    regions={}
    for name in ['fuji','tsukuba','yari']:
        folders=[ROOT/f'build/terrain-error20-{prefix}-{name}' for prefix in ['perceptual','cut10']]
        with np.load(folders[0]/'base-quality.npz') as a,np.load(folders[1]/'base-quality.npz') as b:
            assert np.array_equal(a['p'],b['p']) and np.array_equal(a['f'],b['f'])
        with np.load(folders[0]/'pruned.npz') as a,np.load(folders[1]/'pruned.npz') as b:
            regions[name]=dict(verticesEqual=bool(np.array_equal(a['p'],b['p'])),facesEqual=bool(np.array_equal(a['f'],b['f'])),vertices=len(b['p']))
    result=dict(minimumInsertedHeightErrorMeters=policy['minimumInsertedHeightErrorMeters'],additionalUsed=policy['additionalUsed'],additionalBudget=policy['additionalBudget'],regions=regions)
    (ROOT/'build/terrain-additional-cutoff-comparison.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(result,ensure_ascii=False),flush=True)
    return result

if __name__=='__main__':audit()

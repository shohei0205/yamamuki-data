import json,sqlite3
import numpy as np
from contextlib import closing
from pathlib import Path
from generate_shared_quality_budgets import cached_prepare
from error_budget_mesh import base_mesh
from refine_shared_quality import ROOT,compare
compare.initialize();binary=ROOT/'build/terrain-rust-local-budget-target/release/terrain-adaptive-tin.exe'
with closing(sqlite3.connect(ROOT/'build/terrain-shared-allocation-landform128.sqlite3')) as db,closing(sqlite3.connect(ROOT/'build/terrain-regional-priority-levels-v5.sqlite3')) as t:
 m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);templates=json.loads(t.execute('select value from metadata where key="manifest"').fetchone()[0])['reliefRegions']
 for a in m['reliefRegions']:
  folder=ROOT/f"build/terrain-error20-{a['id']}";checkpoint=folder/'base-quality.npz'
  if checkpoint.exists():continue
  r=cached_prepare(next(v for v in templates if v['id']==a['id']),db);p,f=base_mesh(r,binary,folder);np.savez(checkpoint,p=p,f=f);print('基本完了',a['id'],len(p),flush=True)

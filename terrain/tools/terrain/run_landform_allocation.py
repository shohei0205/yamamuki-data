"""従来の優先度に山体の盛り上がりを足した比較を生成・検証する。"""
import json,os,subprocess
from pathlib import Path
from compare_shared_allocation import main,ROOT
from verify_shared_allocation import verify
from run_shape_allocation import density
from audit_shared_mountain_quality import local_errors


if __name__=='__main__':
    binary=ROOT/'build/terrain-rust-incremental-target/release/terrain-adaptive-tin.exe'
    main(192000,2048,True,128000.,True,str(binary),landform_aware=True)
    verification=verify('landform128');assert verification['totalSource']==192000
    node=Path.home()/'.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node.exe'
    env=os.environ.copy();env['NODE_PATH']=str(node.parents[1]/'node_modules');env['ALLOCATION_MODES']='landform128'
    subprocess.run([str(node),'tools/terrain/test_shared_allocation_view.cjs'],cwd=ROOT,env=env,check=True)
    baseline=density('terrain128');candidate=density('landform128')
    errors=dict(baseline=local_errors('terrain128'),candidate=local_errors('landform128'))
    checks={name:dict(densityIncreased=candidate[name]['withinRadius']['5']>baseline[name]['withinRadius']['5'],centralErrorReduced=errors['candidate'][name]['2']['rmse']<errors['baseline'][name]['2']['rmse'],surroundingErrorLimited=errors['candidate'][name]['5']['rmse']<=errors['baseline'][name]['5']['rmse']*1.1) for name in ['fuji','tsukuba']}
    report=dict(complete=True,improved=all(all(v.values()) for v in checks.values()),checks=checks,baseline=baseline,candidate=candidate,errors=errors,verification=verification,sqliteBytes=(ROOT/'build/terrain-shared-allocation-landform128.sqlite3').stat().st_size)
    (ROOT/'build/terrain-landform-allocation-report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print('山体評価の比較と検証が完了',report,flush=True)

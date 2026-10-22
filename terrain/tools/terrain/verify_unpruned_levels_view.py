"""今回の生成完了を待ち、全品質のブラウザー表示を一度検証する。"""
import json,os,subprocess,time
from pathlib import Path
from national_independent_mesh import ROOT
progress=ROOT/'build/terrain-unpruned-levels.progress.json'
result=ROOT/'build/terrain-unpruned-levels-view.json'
def main():
    deadline=time.monotonic()+43200
    while time.monotonic()<deadline:
        failure=(ROOT/'build/terrain-unpruned-levels.err').read_text(encoding='utf-8').strip()
        if failure:raise RuntimeError('メッシュ生成が失敗しました: '+failure[-2000:])
        try:ready=json.loads(progress.read_text(encoding='utf-8')).get('complete',False)
        except (ValueError,FileNotFoundError):ready=False
        if ready:break
        time.sleep(15)
    else:raise TimeoutError('生成完了の待機が12時間を超えました')
    runtime=Path.home()/'.cache/codex-runtimes/codex-primary-runtime/dependencies/node'
    env=dict(os.environ,NODE_PATH=str(runtime/'node_modules'),QUALITY_VARIANT='direct',QUALITY_ERRORS='12.5,25,50,100,200,400,800')
    subprocess.run([str(runtime/'bin/node.exe'),str(ROOT/'tools/terrain/test_error_quality_levels_view.cjs')],cwd=ROOT,env=env,check=True)
    result.write_text(json.dumps(dict(complete=True,regions=['fuji','tsukuba','yari'],errors=[12.5,25,50,100,200,400,800],cases=21),ensure_ascii=False,indent=2),encoding='utf-8')
if __name__=='__main__':main()

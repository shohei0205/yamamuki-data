import json,sqlite3,time,html,os
from pathlib import Path
from datetime import datetime
import argparse
parser=argparse.ArgumentParser(description='全国地形生成の進捗HTMLを更新する')
parser.add_argument('--job-root',type=Path,required=True)
parser.add_argument('--prepared',default='run-unlimited')
parser.add_argument('--once',action='store_true')
args=parser.parse_args()
root=args.job_root.resolve()
previous=None
while True:
 try:
  state=json.loads((root/'job-status.json').read_text(encoding='utf-8-sig'))
  c=sqlite3.connect('file:'+(root/args.prepared/'work.sqlite3').as_posix()+'?mode=ro',uri=True,timeout=2)
  stages={n:json.loads(d) for n,d in c.execute('select name,details from stages')};c.close()
  rows=''
  for z in range(13,5,-1):
   status='待機';detail=''
   for prefix,label in [('fit','標高誤差調整 完了'),('prune-queue','制御点削減中'),('prune','制御点削減 完了'),('certify','誤差検証 完了'),('shape','形状検証 完了')]:
    if f'{prefix}-{z}' in stages:status=label
   fit=stages.get(f'fit-{z}',{})
   if 'max' in fit:detail=f"最大誤差 {fit['max']:.3f} m"
   rows+=f'<tr><th>z{z}</th><td>{status}</td><td>{detail}</td></tr>'
  now=datetime.now().astimezone().strftime('%Y-%m-%d %H:%M:%S %Z')
  stage='完了' if state.get('complete') else {'generate':'全国生成中','failed':'停止・エラー'}.get(state.get('stage'),state.get('stage','確認中'))
  doc='<!doctype html><html lang="ja"><meta charset="utf-8"><meta http-equiv="refresh" content="10"><title>全国地形データ 生成進捗</title><style>body{font:16px system-ui;background:#eef4f3;color:#183733;max-width:1000px;margin:40px auto;padding:0 24px}section{background:white;border-radius:14px;padding:24px;margin:20px 0}table{width:100%;border-collapse:collapse}td,th{text-align:left;padding:14px;border-bottom:1px solid #dde7e4}h1{font-size:28px}pre{white-space:pre-wrap;overflow-wrap:anywhere}small{color:#56716a}</style><h1>全国地形データ 生成進捗</h1>'
  doc+=f'<small>情報取得：{now} ／ 情報は10秒間隔、画面は10秒間隔で更新</small><section><h2>{html.escape(stage)}</h2><p>固定係数 1 ／ z6〜z13 ／ 頂点数の固定上限なし</p><p>三地域の事前検証：{('完了' if (root/args.prepared/'ready.json').exists() else '未確認')}</p><p>全国の平滑化済み標高SQLite：{('生成済み' if (root/'smoothed-dem/smoothed-dem.work.sqlite3').exists() else '未確認')}</p></section><section><table><tr><th>レベル</th><th>処理状況</th><th>記録済み情報</th></tr>{rows}</table><p><small>保存済みの処理段階を表示。削減中の残量・完了予定時刻は未計測です。</small></p></section>'
  doc+='<section><h2>配布用SQLite</h2><p>'+('生成済み' if (root/'gsi-terrain.sqlite3').exists() else '未生成。全国生成後に変換・検証します。')+'</p><pre>'+html.escape(state.get('error',''))+'</pre></section>'
  doc+='<section><details><summary>処理の詳細</summary><pre>'+html.escape(json.dumps(dict(state=state,stages=stages),ensure_ascii=False,indent=2))+'</pre></details></section></html>'
  progress_path=root/args.prepared/'progress.json'
  progress=json.loads(progress_path.read_text(encoding='utf-8-sig'))
  done=progress.get('done',0);total=progress.get('total',0);profile=progress.get('profile',{})
  elapsed=profile.get('elapsed_seconds',0);stamp=progress_path.stat().st_mtime
  rate=None
  if previous and stamp>previous[0] and progress.get('stage')==previous[2] and progress.get('z')==previous[3] and done>=previous[1]:rate=(done-previous[1])/(stamp-previous[0])
  if previous is None or stamp>previous[0]:previous=(stamp,done,progress.get('stage'),progress.get('z'))
  percent=100*done/total if total else 0
  def duration(seconds):
   h,r=divmod(int(seconds),3600);m,sec=divmod(r,60)
   return f'{h}時間{m:02}分{sec:02}秒'
  avg=done/elapsed if elapsed else 0
  remaining=max(0,total-done)
  eta=duration(remaining/(rate or avg)) if (rate or avg)>0 else '算出待ち'
  age=max(0,__import__('time').time()-stamp)
  names={'prune-parallel':'制御点の並列削減','fit':'標高誤差調整','audit':'誤差検証'}
  title=names.get(progress.get('stage'),progress.get('stage','確認中'))
  detail=f'<section><h2>現在の工程：z{progress.get("z","—")} {html.escape(title)}</h2><div style="font-size:32px;font-weight:bold;color:#167767">{percent:.2f}%</div><progress style="width:100%;height:24px" max="{total or 1}" value="{done}"></progress><table>'
  metrics=[('処理済み候補',f'{done:,} / {total:,} 点'),('残り候補',f'{remaining:,} 点'),('削減した制御点',f'{progress.get("accepted",0):,} 点'),('工程の経過時間',duration(elapsed)),('累計平均速度',f'{avg:,.1f} 候補/秒'),('直近の速度',f'{rate:,.1f} 候補/秒' if rate is not None else '更新待ち'),('この工程の残り目安',eta),('並列実行',f'{progress.get("workers","—")} スレッド / {progress.get("lanes","—")} レーン'),('生成器の最終進捗記録',datetime.fromtimestamp(stamp).astimezone().strftime('%Y-%m-%d %H:%M:%S')+f'（{age:.0f}秒前）')]
  for label,value in metrics:detail+=f'<tr><th>{label}</th><td>{value}</td></tr>'
  detail+='</table><p><small>割合と残り時間は現在の工程のみ。全国生成全体の進捗率ではありません。処理速度は地形により変動し、この後に監査と他レベルの生成が続きます。</small></p>'
  detail+='<details><summary>処理時間の内訳・メモリ</summary><table>'
  for key,label in [('prepare_seconds','準備'),('restore_seconds','復元'),('evaluate_seconds','誤差評価'),('save_seconds','保存'),('sync_seconds','同期')]:
   if key in profile:detail+=f'<tr><th>{label}</th><td>{duration(profile[key])}</td></tr>'
  for key,label in [('peak_private_bytes','最大プライベートメモリ'),('peak_working_set_bytes','最大物理メモリ'),('last_free_disk_bytes','最終確認時のディスク空き')]:
   value=progress.get('resources',{}).get(key)
   if value is not None:detail+=f'<tr><th>{label}</th><td>{value/1024**3:.2f} GiB</td></tr>'
  detail+='</table></details></section>'
  doc=doc.replace('<section><h2>配布用SQLite</h2>',detail+'<section><h2>配布用SQLite</h2>')
  tmp=root/'progress.tmp.html';tmp.write_text(doc,encoding='utf-8');os.replace(tmp,root/'progress.html')
 except Exception as e:
  (root/'progress-html-error.log').write_text(str(e),encoding='utf-8')
  if args.once:raise
 if args.once:break
 time.sleep(10)
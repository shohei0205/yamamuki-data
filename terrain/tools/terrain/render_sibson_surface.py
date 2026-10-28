"""自然近傍と既存 MLS を同じ視点で比較する。"""
import re
from pathlib import Path
root=Path('build/mls-generated-preview')
s=(root/'dem-xy/index.html').read_text(encoding='utf-8')
s=s.replace('狭いMLS・XY配置と点数の比較','自然近傍補間・MLSの比較').replace('二次MLS ／ XY配置・点数の比較','自然近傍補間 ／ 自由配置の制御点')
options='<option value="natural-xy" selected>自然近傍・8,192点（目標の高さ）</option><option value="natural-more">自然近傍・16,384点（目標の高さ）</option><option value="six-xy">MLS・8,192点（再適合した高さ）</option><option value="six-more">MLS・16,384点（再適合した高さ）</option>'
options=''.join('<option value="natural-error'+str(e)+'">自然近傍・最大誤差'+str(e)+' m（点数自動）</option>' for e in [12.5,25,50,100,200,400,800])+options
options=options.replace('<option value="six-xy">', ''.join('<option value="natural-'+str(n)+'">自然近傍・'+format(n,',')+'点（目標の高さ）</option>' for n in [4096,2048,1024,512,256,128])+'<option value="six-xy">')
s=re.sub(r'<select id="variantSelect">.*?</select>','<select id="variantSelect">'+options+'</select>',s)
s=s.replace("'six-xy','six-more'].includes", "'six-xy','six-more','natural-error12.5','natural-error25','natural-error50','natural-error100','natural-error200','natural-error400','natural-error800','natural-xy','natural-more','natural-4096','natural-2048','natural-1024','natural-512','natural-256','natural-128'].includes")
s=s.replace('選択した自由配置MLS','選択した自由配置曲面')
s=re.sub(r'<p>6点を下限に、.*?</p>','<p>自然近傍は Voronoi 領域の面積比による非負の重みで補間します。固定の格子・半径・近傍点数は使いません。8,192・16,384 点の XY は MLS と同じです。少点数版は同じ密度基準で点数ごとに再配置し、粗い版が細かい版の部分集合になる制約は設けていません。自然近傍の Z は LPF 目標から取得（0.125 m 刻み）、MLS の Z は別途再適合した値なので、補間式以外にも差があります。LPF σ は全方式で88.4 m。表示時の追加平滑化なし。</p>',s)
s=s.replace('元DEMに直接適合した独立MLS','自然近傍補間')
s=s.replace('MLS 曲面の分割線ではありません。','補間後の曲面の分割線ではありません。')
s=s.replace('<p>左ドラッグ：','<p>外周付近は制御点が不足しており、32 km 全域では大きな誤差が残ります。まず山頂周囲 8 km で比較してください。</p><p>左ドラッグ：')
# LPF の比較は点配置・点数を維持し、参照目標もモデル別に選ぶ。
s=s.replace('50 m・同じLPF目標','点数・LPF幅の比較')
s=s.replace('<label>左側 ', '<label>生成時LPF <select id="lpfSelect"><option value="fixed">固定 σ88.4 m</option><option value="scaled">点数に連動</option><option value="error" hidden>誤差に連動</option></select></label>　<label>左側 ')
s=s.replace("modelKey=variant==='original'?kind:error+'-'+variant,referenceKey=", "autoError=variant.startsWith('natural-error'),requestedError=autoError?Number(variant.slice('natural-error'.length)):Number(error),scaled=!autoError&&document.getElementById('lpfSelect').value==='scaled'&&variant.startsWith('natural'),controlCount=variant==='natural-xy'?8192:variant==='natural-more'?16384:Number(variant.split('-')[1]),modelKey=autoError?requestedError+'-'+variant:scaled?error+'-natural-scale-'+controlCount:variant==='original'?kind:error+'-'+variant,referenceKey=")
s=s.replace("?error+'-target':'dem'", "?(autoError?requestedError+'-target-error'+requestedError:scaled?error+'-target-scale-'+controlCount:error+'-target'):'dem'")
s=s.replace("+'&render='+document.getElementById('renderMode').value", "+'&render='+document.getElementById('renderMode').value+'&lpf='+document.getElementById('lpfSelect').value")
s=s.replace("document.getElementById('renderMode').value=params.get('render')", "document.getElementById('lpfSelect').value=params.get('lpf')==='scaled'?'scaled':'fixed';\ndocument.getElementById('lpfSelect').onchange=()=>loadRange().catch(e=>document.getElementById('status').textContent=e.message);\ndocument.getElementById('renderMode').value=params.get('render')")
s=s.replace('LPF σ は全方式で88.4 m。', '生成時 LPF は固定幅と点数連動（σ=32 km÷√点数÷4）を比較できます。連動版も固定版と同じ XY を使います。左の LPF 目標も選択した幅に合わせます。MLS は固定幅のみです。')
s=s.replace(" document.getElementById('status').textContent='元DEMと曲面を読み込み中…';", " document.getElementById('lpfSelect').disabled=autoError||!variant.startsWith('natural');\n document.getElementById('status').textContent='元DEMと曲面を読み込み中…';")
s=s.replace(' info=nextInfo;focus=', " info=nextInfo;if(autoError){document.getElementById('lpfSelect').value='error';document.querySelector('#lpfSelect option[value=error]').textContent='誤差に連動 σ'+info.sigmaMeters+' m';}else if(document.getElementById('lpfSelect').value==='error')document.getElementById('lpfSelect').value='fixed';focus=")
s=s.replace("window.activeVariant=variant;", "window.activeVariant=variant;window.activeLpf=autoError?'error':scaled?'scaled':'fixed';")

s=s.replace("(info.filteredTargetValidation?'LPF目標 ':'元DEM目標 ')","(info.pointCountMode?'元DEM目標 ':info.filteredTargetValidation?'LPF目標 ':'元DEM目標 ')")
s=s.replace('((info.filteredTargetValidation||info).converged', '((info.pointCountMode?info:(info.filteredTargetValidation||info)).converged')
s=s.replace('<p>外周付近は', '<p>「最大誤差指定（点数自動）」は元PNGの全画素中心で誤差を評価し、超過箇所に点を追加します。LPF σ は許容誤差の半分（m）から開始し、LPFだけの標高差を許容誤差の半分以下に抑えるため必要に応じて弱めます。</p><p>従来の固定点数版では外周付近は')
from sibson_compression_ui import inject as inject_compression
s=inject_compression(s)
# 親XY保持の曲面を同じ視点・誤差で比較する。
s=s.replace('<label>生成時LPF ', '<label>XY生成 <select id="xyMode"><option value="independent">品質ごとに独立</option><option value="nested">親XY保持・追加点保存</option></select></label>　<label>生成時LPF ')
s=s.replace("autoError=variant.startsWith('natural-error'),requestedError=", "autoError=variant.startsWith('natural-error'),xyNested=autoError&&document.getElementById('xyMode').value==='nested',requestedError=")
s=s.replace("modelKey=autoError?requestedError+'-'+variant", "modelKey=autoError?requestedError+'-natural-'+(xyNested?'nested':'error')+requestedError")
s=s.replace("autoError?requestedError+'-target-error'+requestedError", "autoError?requestedError+'-target-'+(xyNested?'nested':'error')+requestedError")
s=s.replace("document.getElementById('lpfSelect').disabled=", "document.getElementById('xyMode').disabled=!autoError;document.getElementById('lpfSelect').disabled=")
s=s.replace("+'&lpf='+document.getElementById('lpfSelect').value", "+'&lpf='+document.getElementById('lpfSelect').value+'&xy='+document.getElementById('xyMode').value")
s=s.replace("document.getElementById('lpfSelect').value=params.get('lpf')", "document.getElementById('xyMode').value=params.get('xy')==='nested'?'nested':'independent';\ndocument.getElementById('xyMode').onchange=()=>loadRange().catch(e=>document.getElementById('status').textContent=e.message);\ndocument.getElementById('lpfSelect').value=params.get('lpf')")
s=s.replace("showCompression(region,requestedError,ticket,autoError);", "showCompression(region,requestedError,ticket,autoError,xyNested);")
s=s.replace("window.activeVariant=variant;", "window.activeXYMode=xyNested?'nested':'independent';window.activeVariant=variant;")
s=s.replace("residual?'残差追加分 ':'独立圧縮 '", "residual?'残差追加分 ':info.nestedXY?'独立換算 ':'独立圧縮 '")
s=s.replace('<p>「最大誤差指定', '<p>「親XY保持」は粗い品質のXYを必ず含む別の生成系統です。表示時の補間は共通で、元DEMへの最大誤差も同じ基準で検証します。容量は点数の変化を含めて比較します。</p><p>「最大誤差指定')
s=s.replace('</header>',' ／ <a href="../dem-natural-seams/">タイル接合試験</a></header>')
dest=root/'dem-natural/index.html';dest.parent.mkdir(exist_ok=True);dest.write_text(s,encoding='utf-8',newline='\n')

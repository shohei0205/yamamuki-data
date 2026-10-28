"""従来・LPF・曲率抑制を、元DEMとLPF目標の両方で比較する画面。"""
import re
from pathlib import Path

ROOT = Path('build/mls-generated-preview')
html = (ROOT/'dem-regions/index.html').read_text(encoding='utf-8')
html = html.replace('3地域・元DEM基準のMLS品質比較', '3地域・LPF目標と波打ち抑制の比較')
html = html.replace('富士山・槍ヶ岳・筑波山 ／ 元DEM基準の曲面比較', '自由配置MLS ／ LPF目標・曲率抑制の比較')
html = html.replace('<label>表示範囲 ', '''<label>生成 <select id="variantSelect"><option value="original">従来・未処理DEM</option><option value="lpf">LPFのみ（50 m比較用）</option><option value="lpf-stable" selected>LPF＋曲率抑制</option></select></label>
<label>左側 <select id="referenceSelect"><option value="dem">元DEM</option><option value="target" selected>LPF後の目標</option></select></label>　<label>表示範囲 ''')
html = html.replace('<option value="lpf-stable" selected>LPF＋曲率抑制</option>', '<option value="lpf-balanced" selected>LPF＋曲率抑制（弱）</option><option value="lpf-stable">LPF＋曲率抑制（強・50 m比較用）</option>')
html = html.replace('<option value="lpf-balanced" selected>', '<option value="lpf-smooth">半径平滑化（非採用・比較用）</option><option value="lpf-balanced" selected>')
html = html.replace('<option value="lpf-balanced" selected>', '<option value="lpf-refit" selected>LPF＋滑らかな補間半径（生成時に再適合）</option><option value="lpf-balanced">')
html = html.replace("region=document.getElementById('regionSelect').value;", "region=document.getElementById('regionSelect').value,variant=document.getElementById('variantSelect').value,error=kind.split('-')[0],modelKey=variant==='original'?kind:error+'-'+variant,referenceKey=document.getElementById('referenceSelect').value==='target'&&Number(error)>=50?error+'-target':'dem';")
html = html.replace("fetch(region+'/'+kind+'.json')", "fetch('../dem-regions/'+region+'/'+modelKey+'.json')")
html = html.replace("...['dem',kind].map", "...[referenceKey,modelKey].map")
html = html.replace("fetch(region+'/'+name+'-'+range+'.bin')", "fetch('../dem-regions/'+region+'/'+name+'-'+range+'.bin')")
html = html.replace("info.targetError+' m目標 '+(info.converged?'達成':'未達')", "info.targetError+' m '+(info.filteredTargetValidation?'LPF目標 ':'元DEM目標 ')+((info.filteredTargetValidation||info).converged?'達成':'未達')+(info.sigmaMeters?' ／ LPF σ'+info.sigmaMeters.toFixed(1)+' m':'')")
html = html.replace("document.querySelector('.right').textContent=document.getElementById('modelSelect').selectedOptions[0].textContent;", "document.querySelector('.right').textContent=document.getElementById('modelSelect').selectedOptions[0].textContent+' ／ '+document.getElementById('variantSelect').selectedOptions[0].textContent;document.querySelector('.left').textContent=referenceKey==='dem'?'元PNG DEM':'LPF後の目標DEM';")
html = html.replace("window.levelInfo=info;", "if(info.filteredTargetValidation){const f=info.filteredTargetValidation;document.getElementById('status').textContent+=' LPF目標との差：最大 '+f.measuredMaxError.toFixed(2)+' m ／ RMSE '+f.measuredRmse.toFixed(2)+' m。';}window.levelInfo=info;window.activeVariant=variant;window.activeReference=referenceKey;")
html = html.replace("'?region='+region+'&quality='+kind", "'?region='+region+'&quality='+kind+'&variant='+variant+'&reference='+document.getElementById('referenceSelect').value")
html = html.replace("const params=new URLSearchParams(location.search),initial=params.get('quality'),initialRegion=params.get('region');", "const params=new URLSearchParams(location.search),initial=params.get('quality')||'50-independent',initialRegion=params.get('region');")
html = html.replace('loadRange().catch(e=>', '''if(['original','lpf','lpf-stable','lpf-balanced','lpf-smooth','lpf-refit'].includes(params.get('variant')))document.getElementById('variantSelect').value=params.get('variant');
if(['dem','target'].includes(params.get('reference')))document.getElementById('referenceSelect').value=params.get('reference');
function adjustVariant(){const error=Number(document.getElementById('modelSelect').value.split('-')[0]),variant=document.getElementById('variantSelect').value;if(variant!=='original'&&error<50)document.getElementById('modelSelect').value='50-independent';if(variant==='lpf'||variant==='lpf-stable')document.getElementById('modelSelect').value='50-independent';}
document.getElementById('variantSelect').onchange=()=>{adjustVariant();loadRange().catch(e=>document.getElementById('status').textContent=e.message);};
document.getElementById('referenceSelect').onchange=()=>loadRange().catch(e=>document.getElementById('status').textContent=e.message);
adjustVariant();
loadRange().catch(e=>''', 1)
html = html.replace("document.getElementById('modelSelect').onchange=()=>loadRange()", "document.getElementById('modelSelect').onchange=()=>{const e=Number(document.getElementById('modelSelect').value.split('-')[0]);if(e<50){document.getElementById('variantSelect').value='original';document.getElementById('referenceSelect').value='dem';}else if(['lpf','lpf-stable'].includes(document.getElementById('variantSelect').value)&&e!==50)document.getElementById('variantSelect').value='lpf-balanced';return loadRange()")
html = html.replace(".catch(e=>document.getElementById('status').textContent=e.message);\nlet drag", ".catch(e=>document.getElementById('status').textContent=e.message);};\nlet drag")
html = html.replace('左：元PNG標高タイル z14 ／ 右：元DEMに直接適合した自由配置MLS。', '左：元DEMまたはLPF後の目標 ／ 右：選択した自由配置MLS。')
html = html.replace('誤差は元DEMの画素標高との差であり、', 'LPF幅は制御点の平均間隔の1/4。元DEMとの差とLPF後の目標との差を分けて表示します。元DEMとの差は、')
html = html.replace("window.activeVariant=variant;", "if(info.sameSupportAtFitAndReconstruction)document.getElementById('status').textContent+=' 生成・復元は同じ半径計算。表示時の追加平滑化なし。';window.activeVariant=variant;")
html = html.replace('../yari-dem/', '../dem-regions/').replace('従来の槍ヶ岳3方式比較', '従来方式・全地域全段階の比較')
dest = ROOT/'dem-lpf/index.html'; dest.parent.mkdir(exist_ok=True)
dest.write_text(html, encoding='utf-8', newline='\n')

"""元DEMとの比較画面を作る。旧Bスプライン比較は基準を明示して残す。"""
from pathlib import Path

ROOT = Path('build/mls-generated-preview')
old = ROOT / 'yari-levels/index.html'
html = old.read_text(encoding='utf-8')
start = html.index('(async()=>')
end = html.index('let drag=null;', start)
html = html[:start] + r'''
const xyBuffers=[],fieldBuffers=[];let info,loadTicket=0;
async function loadRange(resetCamera=false){
 const ticket=++loadTicket,range=Number(document.getElementById('range').value),kind=document.getElementById('modelSelect').value;
 document.getElementById('status').textContent='元DEMと曲面を読み込み中…';
 const [nextInfo,...fields]=await Promise.all([fetch(kind+'.json').then(r=>{if(!r.ok)throw Error('この方式は生成中です');return r.json();}),...['dem',kind].map(name=>fetch(name+'-'+range+'.bin').then(r=>{if(!r.ok)throw Error('この方式は生成中です');return r.arrayBuffer();}))]);
 if(ticket!==loadTicket)return;
 info=nextInfo;
 const n=info.n,N=(n+1)**2,xy=new Float32Array(N*2);
 for(let y=0;y<=n;y++)for(let x=0;x<=n;x++){const i=y*(n+1)+x;xy[2*i]=(x/n-.5)*range;xy[2*i+1]=(y/n-.5)*range;}
 if(!models.length){
  const ids=new Uint32Array(n*n*6);for(let y=0;y<n;y++)for(let x=0;x<n;x++){const a=y*(n+1)+x,j=(y*n+x)*6;ids.set([a,a+1,a+n+1,a+1,a+n+2,a+n+1],j);}window.indexCount=ids.length;
  for(let i=0;i<2;i++){const vao=gl.createVertexArray();gl.bindVertexArray(vao);xyBuffers.push(attr('xy',xy,2));fieldBuffers.push(attr('field',new Float32Array(fields[i]),3));const b=gl.createBuffer();gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,b);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,ids,gl.STATIC_DRAW);models.push(vao);}
 }else for(let i=0;i<2;i++){gl.bindBuffer(gl.ARRAY_BUFFER,xyBuffers[i]);gl.bufferData(gl.ARRAY_BUFFER,xy,gl.STATIC_DRAW);gl.bindBuffer(gl.ARRAY_BUFFER,fieldBuffers[i]);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array(fields[i]),gl.STATIC_DRAW);}
 if(resetCamera||!ready){span=range*1.1;pan=[0,0];}ready=true;draw();
 const residual=info.reconstruction==='parent-plus-residual-MLS';
 document.querySelector('strong').textContent=info.controls.toLocaleString()+' 制御点 ／ '+(residual?'残差追加分 ':'独立圧縮 ')+(info.bytes/1024).toFixed(1)+' KiB'+(residual?' ／ 親込み '+(info.totalBytes/1024).toFixed(1)+' KiB':'')+' ／ '+info.targetError+' m目標 '+(info.converged?'達成':'未達');
 document.querySelector('.right').textContent=document.getElementById('modelSelect').selectedOptions[0].textContent;
 document.getElementById('status').textContent='元PNGの全 '+info.samples.toLocaleString()+' 画素で検証：最大 '+info.measuredMaxError.toFixed(2)+' m ／ RMSE '+info.measuredRmse.toFixed(2)+' m ／ 山頂周囲2 kmの最大 '+info.measuredPeakMaxError.toFixed(2)+' m。欠損 '+info.missing+' 画素。';
 window.levelInfo=info;
 history.replaceState(null,'','?quality='+kind);
}
const initial=new URLSearchParams(location.search).get('quality');if(['25-independent','12.5-independent','12.5-residual'].includes(initial))document.getElementById('modelSelect').value=initial;
loadRange().catch(e=>document.getElementById('status').textContent=e.message);
document.getElementById('range').onchange=()=>loadRange(true).catch(e=>document.getElementById('status').textContent=e.message);
document.getElementById('modelSelect').onchange=()=>loadRange().catch(e=>document.getElementById('status').textContent=e.message);
''' + html[end:]
html = html.replace('<title>槍ヶ岳・生成制御点MLSの改善版</title>', '<title>槍ヶ岳・元PNG DEMとの比較</title>')
start = html.index('<header>')
end = html.index('<script>', start)
html = html[:start] + '''<header><button id="reset">視点を戻す</button><h1>槍ヶ岳 ／ 元PNG DEM基準・独立と残差の比較</h1>
<label>方式 <select id="modelSelect"><option value="25-independent">25 m・独立</option><option value="12.5-independent" selected>12.5 m・独立</option><option value="12.5-residual">12.5 m・残差曲面（25 m独立が親）</option></select></label>
<label>表示範囲 <select id="range"><option value="8000">山頂周囲 8 km</option><option value="32000">全域 32 km</option></select></label>
<p><strong>読み込み中</strong></p><p>左：元PNG標高タイル z14 ／ 右：元DEMに直接適合した自由配置MLS。左右の視点・陰影・高さ倍率は共通です。</p>
<p>誤差は元DEMの画素標高との差であり、実測地形に対する絶対精度を保証するものではありません。保存形状は自由配置点。描画用格子は配布用データに含みません。</p>
<p>左ドラッグ：移動 ／ 右ドラッグ：回転・傾き ／ ホイール：拡大縮小。範囲の距離はWebメルカトル上の値。</p><p id="status">全元画素の検証結果を読み込み中…</p><a href="../yari-levels/">旧Bスプライン基準の比較</a></header>
<div id="line"></div><div class="label left">元PNG DEM</div><div class="label right">元DEMに直接適合した独立MLS</div>
''' + html[end:]
html = html.replace("document.getElementById('quality').value=quality;", '')
html = html.replace('span=14000;', "span=Number(document.getElementById('range').value)*1.1;")
html = html.replace('span=14000', 'span=8800').replace('field.x-1400.', 'field.x-2300.')
target = ROOT / 'yari-dem/index.html'
target.parent.mkdir(exist_ok=True)
target.write_text(html, encoding='utf-8', newline='\n')
warning = '<p id="dem-warning" style="color:#8a3200">この旧画面の生成・誤差評価は元DEMではなく保存済みBスプライン基準です。<a href="../yari-dem/">元PNG DEMとの比較へ</a></p>'
if 'id="dem-warning"' not in old.read_text(encoding='utf-8'):
 old.write_text(old.read_text(encoding='utf-8').replace('<header>', '<header>'+warning), encoding='utf-8', newline='\n')

"""地形の比較画面へ差分圧縮の実測値を追加する。"""


def inject(html):
    html = html.replace('<p id="status">', '<p id="compressionStats"></p><p id="status">')
    html = html.replace('</header>', ' ／ <a id="compressionLink" href="../dem-natural-compression/">差分圧縮の比較</a></header>')
    code = '''const compressionReports=new Map();
async function showCompression(region,error,ticket,enabled,nested=false){
 const label=document.getElementById('compressionStats');label.textContent='';window.compressionInfo=null;
 const route=nested?'dem-natural-shared':'dem-natural-compression',cacheKey=route+'/'+region;document.getElementById('compressionLink').href='../'+route+'/?region='+region;document.getElementById('compressionLink').textContent=nested?'共通XY圧縮の比較':'差分圧縮の比較';
 if(!enabled)return;
 try{
  if(!compressionReports.has(cacheKey))compressionReports.set(cacheKey,fetch('../'+route+'/'+region+'.json').then(r=>{if(!r.ok)throw Error('未測定');return r.json();}));
  const data=await compressionReports.get(cacheKey);if(ticket!==loadTicket)return;
  const row=data.rows.find(r=>r.error===error);if(!row)return;
  if(nested){label.textContent='共通XY試験：従来 '+row.originalControls.toLocaleString()+'点・'+(row.originalBytes/1024).toFixed(2)+' KiB ／ 親XY保持 '+row.nestedControls.toLocaleString()+'点 ／ 共有形式単体 '+(row.sharedBytes/1024).toFixed(2)+' KiB・親込み '+(row.timings.shared.requiredBytes/1024).toFixed(2)+' KiB。モデル未読込からの準備：従来 '+(row.timings.original.coldSeconds*1000).toFixed(1)+' ms ／ 共有形式 '+(row.timings.shared.coldSeconds*1000).toFixed(1)+' ms。共有保存の追加誤差0。測定はPython＋C、描画は事前計算データ。';}else label.textContent='差分圧縮試験：独立 '+(row.independentBytes/1024).toFixed(2)+' KiB ／ 差分単体 '+(row.deltaBytes/1024).toFixed(2)+' KiB（'+((1-row.deltaBytes/row.independentBytes)*100).toFixed(1)+'%削減）／ 親込み '+(row.timings.chain.requiredBytes/1024).toFixed(2)+' KiB。モデル未読込からの準備：独立 '+(row.timings.independent.coldSeconds*1000).toFixed(1)+' ms ／ 全段差分 '+(row.timings.chain.coldSeconds*1000).toFixed(1)+' ms。保存点完全一致・追加誤差0。測定はPython＋C、描画は既存の事前計算データ。';
  window.compressionInfo=row;
 }catch(e){if(ticket===loadTicket)label.textContent='差分圧縮の測定結果は未取得です。';}
}
'''
    html = html.replace('const params=new URLSearchParams', code+'const params=new URLSearchParams')
    return html.replace('window.activeReference=referenceKey;', 'window.activeReference=referenceKey;showCompression(region,requestedError,ticket,autoError);')

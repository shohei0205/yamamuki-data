// GPU候補表は高さ画像ではなく、制御点とVoronoi半平面の照会索引。
const pixelAssets=new Map(),pixelLoading=new Set(),pixelFailed=new Map();let pixelInflight=0,pixelBytes=0;
const pixelBudget=160*1024*1024;
function pixelRequested(){return $('shade').value.startsWith('pixel');}
function pixelShadingActive(){return pixelRequested()&&(!drag)&&performance.now()-lastInteraction>500&&pixelInflight===0&&!building&&!dirty&&!inflight;}
function pumpPixel(){
 if(!pixelRequested()||drag||performance.now()-lastInteraction<180||inflight>0||building||dirty||buildTimer)return;
 const order=meshes.filter(m=>m.tile).slice().sort((a,b)=>Math.hypot(a.tile.info.bounds[0]+4000-pan[0],a.tile.info.bounds[1]+4000-pan[1])-Math.hypot(b.tile.info.bounds[0]+4000-pan[0],b.tile.info.bounds[1]+4000-pan[1]));
 for(const m of order){const key=m.pixelKey;if(pixelInflight>=2)break;if(pixelAssets.has(key)||pixelLoading.has(key)||Date.now()-(pixelFailed.get(key)||0)<15000)continue;
  pixelLoading.add(key);pixelInflight++;const info=m.tile.info,parts=key.split('/');
  fetch('/gpu?'+new URLSearchParams({region:parts[0],x:info.x,y:info.y,error:info.error})).then(async response=>{if(!response.ok)throw Error(await response.text());const buffer=await response.arrayBuffer();
   while(pixelBytes+buffer.byteLength>pixelBudget){const old=[...pixelAssets.keys()].find(k=>!meshes.some(m=>m.pixelKey===k));if(!old)throw Error('画素評価のGPU常駐予算を超えました');const value=pixelAssets.get(old);pixelBytes-=value.bytes;value.destroy();pixelAssets.delete(old);}
   const value=sibsonPixelUpload(gl,buffer);pixelAssets.set(key,value);pixelBytes+=value.bytes;renderDirty=true;
  }).catch(e=>{pixelFailed.set(key,Date.now());console.warn('画素評価',key,e.message);}).finally(()=>{pixelLoading.delete(key);pixelInflight--;pumpPixel();});
 }
}
function pixelStatus(){
 const selected=meshes.map(m=>pixelAssets.get(m.pixelKey)).filter(Boolean),ready=selected.length,cells=selected.reduce((s,a)=>s+a.info.totalCells,0),unsupported=selected.reduce((s,a)=>s+a.info.unsupportedCells,0);
 window.pixelStats={requested:pixelRequested(),active:pixelShadingActive(),ready,tiles:meshes.length,cells,unsupportedCells:unsupported,bytes:pixelBytes,inflight:pixelInflight};
 $('pixelStatus').hidden=!pixelRequested();if(pixelRequested())$('pixelStatus').textContent=(pixelShadingActive()?($('shade').value==='pixel-live'||$('shade').value==='pixel-debug'?'画素評価（常時計算） ':'画素評価（静止画を再利用） '):pixelRequested()?'操作・準備中は軽量表示 ':'画素評価 ')+ready+'/'+meshes.length+' タイル準備済み ／ 候補表の未対応 '+unsupported+'/'+cells+' セル ／ GPU用データ '+(pixelBytes/1048576).toFixed(1)+' MiB。接合帯・未準備・計算上限に達した画素は従来の陰影。';
}

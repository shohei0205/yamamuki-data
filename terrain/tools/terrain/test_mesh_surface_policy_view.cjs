const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const b=await chromium.launch({channel:'msedge',headless:true});try{
 const p=await b.newPage({viewport:{width:1440,height:1000}}),requests=[],errors=[];p.on('request',r=>requests.push(r.url()));p.on('pageerror',e=>errors.push(e.message));
 await p.addInitScript(()=>{window.__meshMasks=[];const Original=Worker;window.Worker=class extends Original{constructor(...a){super(...a);this.addEventListener('message',e=>{if(e.data.mask)window.__meshMasks.push({n:e.data.n,length:e.data.mask.length,valid:[...e.data.mask].every(v=>v===255)})})}}});
 for(const region of ['fuji','tsukuba','yari']){
  await p.goto(`http://127.0.0.1:61452/?dataset=allocation-error-detail20-direct-e800-mixed&region=${region}&level=3&texture=surface&landColorSize=1024&priorityVertices=0&poolVertices=0`);
  await p.waitForFunction(()=>window.nationalMeshStatus?.tiles&&!dirty&&!active&&!queue.length&&!selectionDirty&&!landColorLoading,null,{timeout:120000});
  const r=await p.evaluate(()=>({policy:manifest.coastlineInMesh,codec:$('landColorCodec').value,masks:__meshMasks,error:gl.getError(),failure:landColorFailure}));assert.equal(r.policy,false);assert.equal(r.codec,'astc8');assert.equal(r.failure,'');assert.equal(r.error,0);assert.ok(r.masks.length>0);assert.ok(r.masks.every(m=>m.n===1&&m.length===1&&m.valid));
  if(region==='fuji')await p.screenshot({path:'build/terrain-landcover-astc/fuji-no-coast-mask.png'});
 }
 assert.equal(requests.filter(u=>/\/tile\/.*\/mask(?:\?|$)/.test(u)).length,0);assert.deepEqual(errors,[]);console.log('3地域でASTC標準・欠損境界で切らない描画・海岸マスク取得なしを確認');
}finally{await b.close()}})().catch(e=>{console.error(e);process.exit(1)});

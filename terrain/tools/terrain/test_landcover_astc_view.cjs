const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const b=await chromium.launch({channel:'msedge',headless:true});try{
 const p=await b.newPage({viewport:{width:1440,height:1000}}),errors=[];p.on('pageerror',e=>errors.push(e.message));
 const ready=()=>p.waitForFunction(()=>window.nationalMeshStatus?.tiles&&!dirty&&!selectionDirty&&!active&&!queue.length&&!landColorLoading&&!reliefLoading&&!lodTransition,null,{timeout:120000});
 const state=()=>p.evaluate(()=>({epoch,serial,camera:[...center,distance,bearing,tilt],smooth:$('smooth').checked,shade:$('meshShading').checked,contrast:$('shadeContrast').value,vertices:nationalMeshStatus.sourceVertices}));
 const results=[];
 for(const region of ['tsukuba','fuji','yari']){
  await p.goto(`http://127.0.0.1:61452/?dataset=allocation-error-detail20-direct-e800-mixed&region=${region}&level=3&curve=loop&divisions=2&smooth=1&texture=surface&meshShade=1&landColorSize=512&landColorCodec=png&priorityVertices=0&poolVertices=0`);await ready();const before=await state();
  for(const size of ['128','256','512','1024','2048','4096']){
   await p.selectOption('#landColorSize',size);await ready();await p.selectOption('#landColorCodec','astc8');await ready();
   const r=await p.evaluate(()=>({...nationalMeshStatus,glError:gl.getError(),info:$('landColorInfo').textContent}));assert.equal(r.landColorFailure,'');assert.equal(r.glError,0);assert.equal(r.landColorCodec,'astc8');assert.equal(r.landColorMips,Math.log2(+size)+1);assert.equal(r.landColorAstcDirect,r.landColorAstcSupported);assert.deepEqual(await state(),before);assert.match(r.info,r.landColorAstcDirect?/GPU直接/:/復号PNG・GPU非対応/);
   let expected=0;for(let n=+size;n>=1;n=Math.floor(n/2))expected+=r.landColorAstcDirect?Math.ceil(n/8)**2*16:n*n*4;assert.equal(r.landColorGpuBytes,expected);
   const native=await p.evaluate(async({region,size})=>{const response=await fetch(`/landcolor-astc/${region}/${size}/astc8`);if(!response.ok)throw Error('ASTC本体を取得できません');const buffer=await response.arrayBuffer(),levels=LandcoverAstc.parse(buffer,+size);return {levels:levels.length,bytes:levels.reduce((n,m)=>n+m.data.byteLength,0)}},{region,size});assert.equal(native.levels,Math.log2(+size)+1);let compressed=0;for(let n=+size;n>=1;n=Math.floor(n/2))compressed+=Math.ceil(n/8)**2*16;assert.equal(native.bytes,compressed);
   results.push({region,size,...r});await p.selectOption('#landColorCodec','png');await ready();assert.deepEqual(await state(),before);
  }
  await p.selectOption('#landColorSize','1024');await p.selectOption('#landColorCodec','astc8');await ready();await p.evaluate(()=>{distance=160000;tilt=70;$('tilt').value=tilt;selectionDirty=dirty=true;});await ready();assert.equal(await p.evaluate(()=>gl.getError()),0);
  await p.locator('details').evaluate(e=>e.open=false);await p.screenshot({path:`build/terrain-landcover-astc/${region}-oblique.png`});await p.evaluate(()=>{distance=14000;tilt=45;$('tilt').value=tilt;selectionDirty=dirty=true;});await ready();assert.equal(await p.evaluate(()=>gl.getError()),0);await p.screenshot({path:`build/terrain-landcover-astc/${region}-near.png`});
 }
 // 読み込み途中でPNGへ戻したとき、遅れて到着したASTCで上書きしない。
 await p.locator('details').evaluate(e=>e.open=true);await p.selectOption('#landColorCodec','png');await ready();
 await p.route('**/landcolor-astc/**',async route=>{await new Promise(r=>setTimeout(r,400));await route.continue()});
 await p.selectOption('#landColorCodec','astc8');await p.selectOption('#landColorCodec','png');await ready();await p.waitForTimeout(700);await ready();assert.equal(await p.evaluate(()=>landColorImage.codec),'png');assert.equal(await p.selectOption('#landColorCodec','astc8').then(()=>p.inputValue('#landColorCodec')),'astc8');await ready();
 assert.deepEqual(errors,[]);require('fs').writeFileSync('build/terrain-landcover-astc/browser-report.json',JSON.stringify(results,null,2));console.log('3地域×6解像度の全ミップ・PNG切替・カメラと陰影設定維持・斜め表示・読み込み競合を確認。GPU直接対応:',results[0].landColorAstcSupported);
}finally{await b.close()}})().catch(e=>{console.error(e);process.exit(1)});

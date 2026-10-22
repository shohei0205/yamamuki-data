const {chromium}=require('playwright'),assert=require('node:assert/strict');
const mode=process.env.DETAIL_MODE||'error-detail20-perceptual';
(async()=>{const browser=await chromium.launch({channel:'msedge',headless:true});try{
for(const region of ['tsukuba','fuji','yari']){
 const page=await browser.newPage({viewport:{width:1200,height:850}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto(`http://127.0.0.1:61452/?dataset=allocation-${mode}-mixed&region=${region}&level=3&overview=1&divisions=1&smooth=0&poolVertices=0&priorityVertices=1`);
 try{await page.waitForFunction(()=>window.nationalMeshStatus?.tiles>0&&!selectionDirty&&!dirty&&!active&&!queue.length,{},{timeout:60000});}catch(e){console.log(await page.evaluate(()=>window.nationalMeshStatus));throw e;}
 const result=await page.evaluate(()=>{draw();return {mode:manifest.sharedAllocation.mode,tiles:cache.size,failed:failures.size,error:gl.getError(),priorityVisible:!$('priorityVerticesLabel').hidden,drawCount:priorityDrawCount,policy:manifest.sharedAllocation.errorDetailPolicy};});
 assert.equal(result.mode,mode+'-mixed');assert.equal(result.failed,0);assert.equal(result.error,0);assert.ok(result.priorityVisible&&result.drawCount>0);assert.equal(result.policy.densityReferenceVertices,4096);assert.deepEqual(errors,[]);
 if(mode.endsWith('-shape')||mode.endsWith('-cut10')){if(mode.endsWith('-shape'))assert.ok(result.policy.shapePriority);else {assert.equal(result.policy.minimumAdditionalHeightErrorMeters,10);assert.ok(result.policy.minimumInsertedHeightErrorMeters>10);assert.equal(result.policy.shapePriority,false);}if(region==='tsukuba'){await page.locator('#priorityVertices').uncheck();await page.locator('#hokyo').click();await page.waitForFunction(()=>window.nationalMeshStatus?.tiles>0&&!selectionDirty&&!dirty&&!active&&!queue.length);const c=await page.evaluate(()=>geographic(...center));assert.ok(Math.abs(c[0]-140.13)<1e-6&&Math.abs(c[1]-36.1675)<1e-6);await page.screenshot({path:`build/detail-${mode}-hokyo.png`});}}await page.screenshot({path:`build/detail-priority-${mode}-${region}.png`});console.log(region,result);await page.close();
}
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exit(1)});

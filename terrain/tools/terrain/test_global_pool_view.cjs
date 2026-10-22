const {chromium}=require('playwright'),assert=require('node:assert/strict');
const unlimited=process.env.GLOBAL_POOL_UNLIMITED==='1',mode='global-pool128'+(unlimited?'-unlimited':'');
(async()=>{const browser=await chromium.launch({channel:'msedge',headless:true});try{const p=await browser.newPage({viewport:{width:1500,height:1100}}),errors=[];p.on('pageerror',e=>errors.push(e.message));
for(const region of ['fuji','yari','tsukuba']){
 await p.goto(`http://127.0.0.1:61452/?dataset=allocation-${mode}&region=${region}&level=3&curve=loop&divisions=1&smooth=0&overview=1&priorityVertices=0&wire=off&budget=shade`);
 await p.waitForFunction(()=>window.nationalMeshStatus?.tiles===256&&!selectionDirty&&!dirty&&!nationalMeshStatus.active&&!nationalMeshStatus.queued,{timeout:60000});
 const state=await p.evaluate(()=>({policy:manifest.sharedAllocation.localAllocation.policy,limit:manifest.sharedAllocation.localAllocation.windowLimit,info:$('jointInfo').textContent,gl:gl.getError(),mode:nationalMeshStatus.budgetOverlay,scale:budgetOverlayData.colorScale,legend:$('budgetPositive').textContent}));assert.equal(state.policy,'global-pool');assert.equal(state.limit,unlimited?0:864);assert.equal(state.gl,0);assert.equal(state.mode,'shade');assert.ok(state.info.includes(unlimited?'3×3上限 なし':'3×3上限 864点'));assert.ok(state.legend.includes(String(Math.round(state.scale*100))));
 await p.screenshot({path:`build/${mode}-${region}.png`});console.log(region,state);
}
await p.goto('http://127.0.0.1:61452/neighbor_budget.html');await p.waitForFunction(()=>document.querySelectorAll('svg rect').length===768);assert.equal(await p.locator('#error').textContent(),'');assert.ok((await p.locator('#after').getAttribute('href')).includes('global-pool128'));assert.ok((await p.locator('#summary').textContent()).includes('49,152'));await p.screenshot({path:'build/global-pool-dashboard.png'});assert.deepEqual(errors,[]);
}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});

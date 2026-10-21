const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const browser=await chromium.launch({channel:'msedge',headless:true});try{
 const p=await browser.newPage({viewport:{width:1400,height:1000}}),errors=[];p.on('pageerror',e=>errors.push(e.message));
 for(const region of ['yari','fuji','tsukuba'])for(const [divisions,smooth] of [[1,1],[2,0]]){
  await p.goto(`http://127.0.0.1:61452/?dataset=allocation-terrain128&region=${region}&level=3&curve=loop&divisions=${divisions}&smooth=${smooth}&priorityVertices=0&overview=1`);
  await p.waitForFunction(()=>window.nationalMeshStatus?.tiles===256&&!nationalMeshStatus.active&&!nationalMeshStatus.queued,null,{timeout:180000});
  const status=await p.evaluate(()=>({gl:gl.getError(),...nationalMeshStatus,topology:manifest.sharedAllocation.topology}));
  assert.equal(status.topology,'uncut-halo-v3');assert.equal(status.gl,0);assert.equal(status.failures,0);
  if(divisions===1)assert.notEqual(status.shading,'face');else assert.equal(status.shading,'face');assert.ok(status.triangles>0);
  console.log(region,divisions,smooth,{tiles:status.tiles,gpuMiB:status.gpuBytes/2**20});
  await p.screenshot({path:`build/uncut-${region}-${divisions}-${smooth}.png`});
 }
 assert.deepEqual(errors,[]);
}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});

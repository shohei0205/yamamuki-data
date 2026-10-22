const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const browser=await chromium.launch({channel:'msedge',headless:true});try{
 const p=await browser.newPage({viewport:{width:1400,height:1000}}),errors=[];p.on('pageerror',e=>errors.push(e.message));
 for(const mode of (process.env.ALLOCATION_MODES||'relative,balanced').split(','))for(const region of ['fuji','tsukuba','yari']){
  await p.goto(`http://127.0.0.1:61452/?dataset=allocation-${mode}&region=${region}&level=3&curve=loop&engine=gpu&divisions=${mode.includes('128')?2:4}&smooth=1&priorityVertices=1`);
  await p.waitForFunction(()=>window.nationalMeshStatus?.tiles&&!selectionDirty&&!dirty&&!nationalMeshStatus.active&&!nationalMeshStatus.queued,null,{timeout:180000});
  await p.click('#national');await p.waitForFunction(()=>!selectionDirty&&!dirty&&!nationalMeshStatus.active&&!nationalMeshStatus.queued,null,{timeout:180000});
  const result=await p.evaluate(()=>{
   const groups=new Map();let shared=0,maxPosition=0,maxNormal=0;
   for(const t of cache.values())if(t.loopGpu){
    const width=1000*2**t.row[0],array=new Float32Array(t.vertices*8);gl.bindBuffer(gl.ARRAY_BUFFER,t.loopGpu.buffer);gl.getBufferSubData(gl.ARRAY_BUFFER,0,array);
    for(let k=0;k<t.seamIndices.length;k++){
     const id=t.seamIndices[k],key=(t.seamReference[k*2]+t.row[1]*width).toFixed(4)+'/'+(t.seamReference[k*2+1]+t.row[2]*width).toFixed(4);
     const value=[array[id*8]+t.row[1]*width,array[id*8+1]+t.row[2]*width,array[id*8+2],...array.slice(id*8+4,id*8+7)];
     if(groups.has(key)){shared++;const before=groups.get(key);for(let j=0;j<6;j++)if(j<3)maxPosition=Math.max(maxPosition,Math.abs(before[j]-value[j]));else maxNormal=Math.max(maxNormal,Math.abs(before[j]-value[j]));}else groups.set(key,value);
    }
   }
   return {shared,maxPosition,maxNormal,tiles:nationalMeshStatus.tiles,failures:nationalMeshStatus.failures,points:priorityDrawCount,gl:gl.getError(),allocation:manifest.sharedAllocation};
  });
  assert.equal(result.tiles,(128000/(result.allocation.storageWidthMeters||8000))**2);assert.equal(result.gl,0);assert.equal(result.failures,0);assert.ok(result.points>0);if((result.allocation.storageWidthMeters||8000)<128000)assert.ok(result.shared>0);assert.ok(result.maxPosition<.03,JSON.stringify(result));assert.ok(result.maxNormal<.003,JSON.stringify(result));assert.ok(result.allocation.sourceVertices<=result.allocation.totalBudget);
  console.log(mode,region,{tiles:result.tiles,shared:result.shared,position:result.maxPosition,normal:result.maxNormal});
  await p.uncheck('#priorityVertices');await p.screenshot({path:`build/shared-allocation-${mode}-${region}.png`});
 }
 assert.deepEqual(errors,[]);
}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});

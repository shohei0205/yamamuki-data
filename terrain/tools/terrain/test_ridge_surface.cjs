'use strict';
const{chromium}=require('playwright'),fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
(async()=>{
 const output=path.resolve(process.env.RIDGE_OUTPUT||'build/ridge-surface-prototype-v4/browser-check');fs.mkdirSync(output,{recursive:true});
 const port=process.env.RIDGE_PORT||'61469',half=process.env.RIDGE_HALF==='1',finalQuality=half?'half':'fine';
 const browser=await chromium.launch({channel:'chrome',headless:true,args:['--ignore-gpu-blocklist']}),page=await browser.newPage({viewport:{width:1300,height:850}}),errors=[],rows=[];
 page.on('pageerror',e=>errors.push(e.message));
 async function ready(region,quality,division=4){await page.waitForFunction(([r,q,d])=>window.lpfCurveStatus?.ridge?.region===r&&lpfCurveStatus.ridge.quality===q&&lpfCurveStatus.ridgeMesh.requestedDivision===d,[region,quality,division],{timeout:180000});const s=await page.evaluate(()=>lpfCurveStatus);assert.equal(s.error,0);assert.equal(s.renderer,'ridge');assert(s.surfaceTriangles>0);return s;}
 try{
  for(const region of (process.env.RIDGE_REGIONS?process.env.RIDGE_REGIONS.split(','):['yari','fuji','tsukuba'])){
   await page.goto(`http://127.0.0.1:${port}/?region=${region}&quality=${half?'half':'coarse'}&sharpness=1`);rows.push(await ready(region,half?'half':'coarse'));
   for(const quality of (half?[]:['medium','fine'])){await page.selectOption('#ridgeQuality',quality);rows.push(await ready(region,quality));}
   await page.locator('#ridgeFocus').click();await page.screenshot({path:path.join(output,`${region}-summit.png`)});
   if(half){const v=rows.at(-1).ridge;assert(v.storedBytes/v.referenceMeshBytes>.4&&v.storedBytes/v.referenceMeshBytes<.6);}
   const check=await page.evaluate(async ([region,quality])=>{
    const m=await(await fetch(`/api/ridge?region=${region}&quality=${quality}&division=4`)).json(),g=ridgeEngine.readGeometry(),decode=(v,T)=>new T(Uint8Array.from(atob(v),x=>x.charCodeAt(0)).buffer),w=decode(m.mesh.weights,Float32Array),lookup=decode(m.mesh.lookup,Uint32Array),controls=decode(m.mesh.controls,Float32Array),peaks=decode(m.mesh.peaks,Uint32Array),indices=decode(m.mesh.indices,Uint32Array);let maxHeightError=0,maxGradientError=0,maxPeakError=0,maxSharedGap=0,nonpositive=0;
    for(let i=0;i<g.length/6;i+=Math.max(1,Math.floor(g.length/6/100))){const start=lookup[i*2],count=lookup[i*2+1];let origin=0;for(let j=0;j<count;j++)if(w[(start+j)*4+1]!==0){origin=controls[w[(start+j)*4]];break;}let h=origin,dx=0,dy=0;for(let j=0;j<count;j++){const at=(start+j)*4,z=controls[w[at]]-origin;h+=z*w[at+1];dx+=z*w[at+2];dy+=z*w[at+3];}maxHeightError=Math.max(maxHeightError,Math.abs(g[i*6+1]-h));maxGradientError=Math.max(maxGradientError,Math.abs(g[i*6+3]+dx),Math.abs(g[i*6+5]+dy));}
    for(let i=0;i<peaks.length;i++)maxPeakError=Math.max(maxPeakError,Math.abs(g[peaks[i]*6+1]-m.vertices[m.corners[i]][2]));
    const shared=new Map();for(let i=0;i<g.length;i+=6){const key=`${g[i]},${g[i+2]}`;if(shared.has(key))maxSharedGap=Math.max(maxSharedGap,Math.abs(g[i+1]-shared.get(key)));else shared.set(key,g[i+1]);}
    for(let i=0;i<indices.length;i+=3){const a=indices[i]*6,b=indices[i+1]*6,c=indices[i+2]*6,area=(g[b]-g[a])*(g[c+2]-g[a+2])-(g[b+2]-g[a+2])*(g[c]-g[a]);if(area<=0)nonpositive++;}
    let generations=0;const original=gl.beginTransformFeedback.bind(gl);gl.beginTransformFeedback=(...args)=>{generations++;return original(...args);};for(const pitch of [.1,.5,1.1]){camera.pitch=pitch;camera.yaw+=.1;draw();}gl.beginTransformFeedback=original;return {maxHeightError,maxGradientError,maxPeakError,maxSharedGap,nonpositive,generations,error:gl.getError()};
   },[region,finalQuality]);
   console.log(region,JSON.stringify(check));assert.equal(check.error,0);assert.equal(check.generations,0);assert.equal(check.nonpositive,0);assert(check.maxPeakError<.005);assert(check.maxHeightError<.02);assert(check.maxGradientError<.001);assert.equal(check.maxSharedGap,0);
   fs.writeFileSync(path.join(output,`${region}-geometry.json`),JSON.stringify(check,null,2));
   await page.locator('#ridgeLines').check();await page.screenshot({path:path.join(output,`${region}-skeleton.png`)});await page.locator('#ridgeLines').uncheck();
   for(const division of [1,2]){await page.selectOption('#divisions',String(division));await ready(region,finalQuality,division);}
   await page.selectOption('#divisions','4');await ready(region,finalQuality);await page.locator('#normals').uncheck();assert.equal((await ready(region,finalQuality)).error,0);await page.locator('#normals').check();await page.locator('#wire').check();assert.equal((await ready(region,finalQuality)).error,0);await page.locator('#wire').uncheck();
   await page.selectOption('#renderer','features');assert.equal((await page.evaluate(()=>lpfCurveStatus)).renderer,'features');await page.selectOption('#renderer','bspline');assert.equal((await page.evaluate(()=>lpfCurveStatus)).renderer,'bspline');await page.selectOption('#renderer','ridge');await ready(region,finalQuality);
  }
  assert.deepEqual(errors,[]);fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({passed:true,rows,errors},null,2));console.log(JSON.stringify({passed:true,cases:rows.length,errors}));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1);});

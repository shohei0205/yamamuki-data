'use strict';
const {chromium}=require('playwright'),assert=require('node:assert/strict'),fs=require('node:fs');
(async()=>{const browser=await chromium.launch({channel:'chrome',headless:true,args:['--ignore-gpu-blocklist']});const report=[];try{
 const page=await browser.newPage({viewport:{width:1200,height:800}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 for(const region of ['yari','fuji','tsukuba'])for(const budget of [512,2048]){
  await page.goto(`http://127.0.0.1:61473/?region=${region}&renderer=adaptive&points=${budget}&layout=legacy&lowSurface=thinPlate&fairing=0`);
  await page.waitForFunction(([r,b])=>window.lpfCurveStatus?.adaptive?.region===r&&lpfCurveStatus.adaptive.budget===b,[region,budget],{timeout:120000});
  const row=await page.evaluate(async({region,budget})=>{
   const model=await(await fetch(`/api/adaptive?region=${region}&budget=${budget}&layout=legacy&lowSurface=thinPlate`)).json(),p=new Float32Array(Uint8Array.from(atob(model.features),c=>c.charCodeAt(0)).buffer),base=model.base,span=model.span,grid=new Float32Array(Uint8Array.from(atob(model.fieldGrid),c=>c.charCodeAt(0)).buffer),g=adaptiveEngine.readGeometry();let maxHeightError=0,maxGradientError=0;
   function field(x,y){let h=base[0]+(base[1]*x+base[2]*y)/span,gx=base[1]/span,gy=base[2]/span;for(let k=0;k<p.length;k+=4){const dx=(x-p[k])/span,dy=(y-p[k+1])/span,r2=Math.max(dx*dx+dy*dy,1e-30),l=Math.log(r2),w=p[k+3];h+=w*.5*r2*l;gx+=w*(l+1)*dx/span;gy+=w*(l+1)*dy/span;}return[h,gx,gy];}
   function cached(x,y){const n=model.fieldN,qx=(x/span+.5)*n,qy=(y/span+.5)*n,ix=Math.min(n-1,Math.max(0,Math.floor(qx))),iy=Math.min(n-1,Math.max(0,Math.floor(qy))),fx=qx-ix,fy=qy-iy;let out=[0,0,0];for(const [dx,dy,w] of [[0,0,(1-fx)*(1-fy)],[1,0,fx*(1-fy)],[0,1,(1-fx)*fy],[1,1,fx*fy]])for(let j=0;j<3;j++)out[j]+=w*grid[((iy+dy)*(n+1)+ix+dx)*4+j];return out;}
   for(let i=0;i<g.length/6;i+=Math.floor(g.length/6/100)){const v=field(g[i*6],g[i*6+2]);maxHeightError=Math.max(maxHeightError,Math.abs(g[i*6+1]-v[0]));maxGradientError=Math.max(maxGradientError,Math.abs(g[i*6+3]+v[1]),Math.abs(g[i*6+5]+v[2]));}
   const profile=adaptiveEngine.profile([-10000,0,10000]);let profileError=0;for(let i=0;i<3;i++)profileError=Math.max(profileError,Math.abs(profile[i]-field([-10000,0,10000][i],0)[0]));
   adaptiveEngine.setFairing(40);const filtered=adaptiveEngine.readGeometry();let filteredError=0;
   for(let i=0;i<g.length/6;i+=Math.floor(g.length/6/10)){const x=g[i*6],y=g[i*6+2];if(Math.max(Math.abs(x),Math.abs(y))>15000)continue;let h=.2*cached(x,y)[0];for(const [dx,dy,w] of [[40,0,.1],[-40,0,.1],[0,40,.1],[0,-40,.1],[80,0,.05],[-80,0,.05],[0,80,.05],[0,-80,.05],[40,40,.05],[-40,40,.05],[-40,-40,.05],[40,-40,.05]])h+=w*cached(x+dx,y+dy)[0];filteredError=Math.max(filteredError,Math.abs(filtered[i*6+1]-h));}
   return{region,budget,kernel:adaptiveEngine.info.kernel,maxHeightError,maxGradientError,profileError,filteredError,error:gl.getError(),triangles:model.triangles.length};
  },{region,budget});
  console.log(JSON.stringify(row));assert.equal(row.kernel,'Thin-plate-global');assert.equal(row.error,0);assert(row.maxHeightError<.002);assert(row.maxGradientError<.002);assert(row.profileError<.0001);assert(row.filteredError<.002);report.push(row);
  if(budget===512){await page.locator('#splineControlPoints').uncheck();await page.locator('#ridgeFocus').click();await page.screenshot({path:`build/adaptive-free-surface-v6/${region}512-thin-after.png`});}
  await page.selectOption('#adaptiveLowSurface','cubic');await page.waitForFunction(()=>lpfCurveStatus.adaptive?.kernel==='Clough-Tocher-C1-cubic');assert.equal(await page.evaluate(()=>gl.getError()),0);
 }
 assert.deepEqual(errors,[]);fs.writeFileSync('build/adaptive-free-surface-v6/thin-plate-browser-verification.json',JSON.stringify({passed:true,report,errors},null,2));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exit(1);});

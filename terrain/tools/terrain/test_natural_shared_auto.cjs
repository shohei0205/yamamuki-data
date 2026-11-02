const {chromium}=require('playwright');const assert=require('node:assert/strict');
(async()=>{const b=await chromium.launch({channel:'chrome',headless:true});try{const p=await b.newPage({viewport:{width:1200,height:850}}),errors=[],reports=[];p.on('pageerror',e=>errors.push(e.message));p.on('console',m=>{if(m.type()==='error')errors.push(m.text())});
await p.goto('http://127.0.0.1:61493/?region=fuji&quality=auto&source=sqlite&span=450000&tilt=.1&join=1&c1Amount=.35');
async function ready(){await p.waitForFunction(()=>window.gpu128Error||(window.gpu128Ready&&gpu128Stats.pending===0&&!gpu128Stats.morphing&&gpu128Stats.quality==='auto'),null,{timeout:90000});assert(!(await p.evaluate(()=>window.gpu128Error)));assert.equal((await p.evaluate(()=>window.gpu128Failures||[])).length,0);return p.evaluate(()=>gpu128Stats);}
async function view(span,tilt,settle=false){await p.evaluate(v=>gpu128Navigate({...v,pan:[64000,64000]}),{span,tilt});await p.waitForTimeout(settle?3300:200);return ready();}
for(const region of ['fuji','yari','tsukuba']){
 await p.selectOption('#region',region);
 const coarse=await view(450000,.1,true);assert(Math.max(...Object.keys(coarse.levels).map(Number))>=250);console.log('広域',region,coarse.levels,coarse.cacheBytes);
 const fine=await view(6000,.5,true);assert.equal(Math.min(...Object.keys(fine.levels).map(Number)),25);console.log('近景',region,fine.levels,fine.cacheBytes);
 const mixed=await view(70000,1.05,true);assert(Object.keys(mixed.levels).length>=2);const seam=await p.evaluate(()=>checkGPU128Seams({patches:true}));assert(seam.mixedSamples>0);assert(seam.maximumHeightDifference<.05,JSON.stringify(seam));assert(seam.maximumGradientDifference<.005,JSON.stringify(seam));console.log('混合',region,mixed.levels,JSON.stringify(seam));
 const targets=mixed.targets.filter(t=>Number.isFinite(t.depth));const finest=Math.min(...targets.map(t=>t.error)),roughest=Math.max(...targets.map(t=>t.error));const avg=a=>a.reduce((s,t)=>s+t.depth,0)/a.length;assert(avg(targets.filter(t=>t.error===finest))<avg(targets.filter(t=>t.error===roughest)));
 reports.push({region,coarse:coarse.levels,fine:fine.levels,mixed:mixed.levels,seam});
}
for(const span of [25000,80000,5000,250000,18000]){await p.evaluate(span=>gpu128Navigate({span,tilt:.9}),span);await p.waitForTimeout(80);}
await p.waitForTimeout(3300);const final=await ready();assert(final.cacheBytes<=final.cacheBudget);const seam=await p.evaluate(()=>checkGPU128Seams({patches:true}));assert(seam.maximumHeightDifference<.05,JSON.stringify(seam));console.log('ズーム往復',final.levels,JSON.stringify(seam));
require('fs').writeFileSync('build/natural-shared-packed-v1/auto-lod-report.json',JSON.stringify({reports,final:{levels:final.levels,cacheBytes:final.cacheBytes,seam}},null,2));assert.deepEqual(errors,[]);console.log('自動LOD・三地点・混合境界・ズーム往復：成功');}finally{await b.close()}})().catch(e=>{console.error(e);process.exitCode=1});

const {chromium}=require('playwright');const assert=require('node:assert/strict');
(async()=>{const b=await chromium.launch({channel:'chrome',headless:true});try{const p=await b.newPage({viewport:{width:1200,height:850}}),errors=[];p.on('pageerror',e=>errors.push(e.message));p.on('console',m=>{if(m.type()==='error')errors.push(m.text())});
await p.goto('http://127.0.0.1:61492/?region=fuji&quality=300&source=partition&span=70000&tilt=0.7&c1Amount=0.35');
async function ready(){await p.waitForFunction(()=>window.gpu128Error||(window.gpu128Ready&&gpu128Stats.pending===0&&!gpu128Stats.morphing&&gpu128Stats.drawn.every(t=>t.error===Number(document.getElementById('quality').value))),null,{timeout:60000});assert(!(await p.evaluate(()=>window.gpu128Error)));assert.equal((await p.evaluate(()=>window.gpu128Failures||[])).length,0);return p.evaluate(()=>gpu128Stats);}
const qualities=['500','400','300','250','200','125','60','25'];
for(const region of ['fuji','yari','tsukuba']){
 await p.selectOption('#region',region);assert.deepEqual(await p.locator('#quality option').evaluateAll(a=>a.map(v=>v.value)),qualities);
 for(const quality of qualities){await p.evaluate(q=>gpu128Navigate({span:q>=250?80000:q>=125?40000:q>=60?20000:10000,tilt:.8,pan:[64000,64000]}),+quality);await p.selectOption('#quality',quality);
  for(const mode of ['global','local','partition']){await p.selectOption('#source',mode);await p.waitForTimeout(150);const s=await ready();assert(s.drawn.length>0);assert(s.drawn.every(t=>t.error===+quality),JSON.stringify(s.levels));assert(s.cacheBytes<=s.cacheBudget);console.log(region,quality,mode,s.drawn.length,s.cacheBytes);}
  if(quality==='25'||quality==='60'){const r=await p.evaluate(()=>checkGPU128Seams({patches:true}));assert(r.samples>0);assert(r.maximumHeightDifference<.02,JSON.stringify(r));assert(r.maximumGradientDifference<.002,JSON.stringify(r));console.log('境界',region,quality,JSON.stringify(r));}
 }
}
// 細かい品質を広域表示してもキャッシュ不足で取得失敗にならない。
await p.selectOption('#region','fuji');await p.selectOption('#quality','25');await p.evaluate(()=>gpu128Navigate({span:180000,tilt:.2}));await p.waitForTimeout(200);const overview=await ready();console.log('25m広域',overview.drawn.length,overview.cacheBytes,overview.triangles);assert(overview.drawn.length>=100);assert(overview.cacheBytes<=overview.cacheBudget);
assert.deepEqual(errors,[]);console.log('全8品質・三地点・三方式・広域表示：成功');}finally{await b.close()}})().catch(e=>{console.error(e);process.exitCode=1});

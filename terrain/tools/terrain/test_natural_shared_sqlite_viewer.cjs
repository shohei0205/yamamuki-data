const {chromium}=require('playwright');const assert=require('node:assert/strict');
(async()=>{const b=await chromium.launch({channel:'chrome',headless:true});try{const p=await b.newPage({viewport:{width:1100,height:800}}),errors=[],results=[];p.on('pageerror',e=>errors.push(e.message));p.on('console',m=>{if(m.type()==='error')errors.push(m.text())});
await p.goto('http://127.0.0.1:61493/?region=fuji&quality=300&source=sqlite&span=70000&c1Amount=0.35');
async function ready(){await p.waitForFunction(()=>window.gpu128Error||(window.gpu128Ready&&gpu128Stats.pending===0&&!gpu128Stats.morphing&&gpu128Stats.source===document.getElementById('source').value&&gpu128Stats.drawn.every(t=>t.error===Number(document.getElementById('quality').value))),null,{timeout:90000});assert(!(await p.evaluate(()=>window.gpu128Error)));assert.equal((await p.evaluate(()=>window.gpu128Failures||[])).length,0);return p.evaluate(()=>gpu128Stats);}
for(const region of ['fuji','yari','tsukuba']){await p.selectOption('#region',region);
 for(const error of ['500','400','300','250','200','125','60','25']){
  await p.evaluate(q=>gpu128Navigate({span:q>=250?80000:q>=125?40000:q>=60?20000:10000,pan:[64000,64000],tilt:.8}),+error);await p.selectOption('#quality',error);
  const buffers=[];let seconds=[];
  for(const source of ['partition','sqlite']){const start=Date.now();await p.selectOption('#source',source);await p.waitForTimeout(150);await ready();seconds.push((Date.now()-start)/1000);buffers.push(await p.evaluate(()=>readGPU128Samples([64000,64000])));}
  const key=v=>v.tile.error+'/'+v.tile.x+'/'+v.tile.y;const old=new Map(buffers[0].map(v=>[key(v),v]));let height=0,gradient=0,compared=0;
  for(const v of buffers[1]){const a=old.get(key(v));if(!a)continue;assert.equal(v.tile.n,a.tile.n);for(let i=0;i<v.values.length;i++)for(const j of [0,1,2,4,5,6]){const delta=Math.abs(v.values[i][j]-a.values[i][j]);if(j===0||j===4)height=Math.max(height,delta);else gradient=Math.max(gradient,delta);compared++;}}
  assert(compared>0);const seams=await p.evaluate(()=>checkGPU128Seams({patches:true}));assert(seams.samples>0);assert(seams.maximumHeightDifference<.02,JSON.stringify(seams));assert(seams.maximumGradientDifference<.002,JSON.stringify(seams));
  const row={region,error,heightDifference:height,gradientDifference:gradient,compared,seconds,seams};results.push(row);console.log(JSON.stringify(row));
  if(error==='200'&&region==='tsukuba')await p.screenshot({path:'build/sqlite-tsukuba-200-comparison.png'});
 }
}
require('fs').writeFileSync('build/natural-shared-packed-v1/viewer-comparison.json',JSON.stringify(results,null,2));assert.deepEqual(errors,[]);assert(results.every(r=>r.heightDifference<.1),JSON.stringify(results.filter(r=>r.heightDifference>=.1)));assert(results.every(r=>r.gradientDifference<.005));console.log('SQLite三地点全8品質の表示・境界・比較：成功');}finally{await b.close()}})().catch(e=>{console.error(e);process.exitCode=1});

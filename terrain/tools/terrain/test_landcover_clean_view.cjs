const {chromium}=require('playwright'),assert=require('assert/strict');
(async()=>{
 const b=await chromium.launch({channel:'msedge',headless:true});
 try{
 const p=await b.newPage({viewport:{width:1550,height:1050}}),errors=[];p.on('pageerror',e=>errors.push(e.message));
 await p.goto('http://127.0.0.1:61495/?filter=majority-clean&curve=fit&mode=cleanup-split');
 await p.waitForFunction(()=>window.landcoverPrototype?.currentData||document.querySelector('#error').textContent,null,{timeout:120000});
 assert.equal(await p.locator('#error').textContent(),'');const report=[];
 for(const l of ['0','2','4']){
  await p.selectOption('#lod',l);await p.waitForFunction(l=>landcoverPrototype.currentLevel===+l&&document.querySelector('#status').textContent.includes('読込完了'),l,{timeout:120000});
  assert.equal(await p.locator('#error').textContent(),'');
  report.push(await p.evaluate(()=>({level:landcoverPrototype.currentLevel,area:landcoverPrototype.currentData.areaRelativeError,key:landcoverPrototype.currentData.key,geometricFit:landcoverPrototype.currentData.curveInfo.geometricFit,fairBoundary:landcoverPrototype.currentData.curveInfo.fairBoundary,progressiveLod:landcoverPrototype.currentData.curveInfo.progressiveLod,sourceGrid:landcoverPrototype.manifest.sourceGrid,filterBeforeVectorization:landcoverPrototype.manifest.filterBeforeVectorization})));
  if(l==='2'){await p.click('#summit');await p.waitForTimeout(400);await p.screenshot({path:'build/landcover-native-filter-bezier-fuji-v1/fuji-native-before-after.png'});}
 }
 assert.deepEqual(errors,[]);assert.ok(report.every(r=>r.area<1e-7&&r.geometricFit===true&&r.fairBoundary===true&&r.progressiveLod===true&&r.sourceGrid===16384&&r.filterBeforeVectorization===true));console.log(JSON.stringify(report,null,2));
 }finally{await b.close()}
})().catch(e=>{console.error(e);process.exit(1)});

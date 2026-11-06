const {chromium}=require('playwright'),assert=require('assert/strict');
(async()=>{
 const b=await chromium.launch({channel:'msedge',headless:true});
 try{
  const p=await b.newPage({viewport:{width:1550,height:1050}}),errors=[];
  p.on('pageerror',e=>errors.push(e.message));
  await p.goto('http://127.0.0.1:61495/?filter=majority&curve=fit&mode=curve-split');
  await p.waitForFunction(()=>window.landcoverPrototype?.currentData||document.querySelector('#error').textContent,null,{timeout:120000});
  assert.equal(await p.locator('#error').textContent(),'');
  const reports=[];
  for(const filter of ['majority','regions']){
   if(await p.locator('#filter').inputValue()!==filter)await p.selectOption('#filter',filter);
   await p.waitForFunction(f=>window.landcoverPrototype?.filter===f&&document.querySelector('#status').textContent.includes('読込完了'),filter,{timeout:120000});
   for(const level of ['0','2','4']){
    await p.selectOption('#lod',level);
    await p.waitForFunction(l=>landcoverPrototype.currentLevel===+l&&document.querySelector('#status').textContent.includes('読込完了'),level,{timeout:120000});
    assert.equal(await p.locator('#error').textContent(),'');
    const r=await p.evaluate(()=>({filter:landcoverPrototype.filter,level:landcoverPrototype.currentLevel,key:landcoverPrototype.currentData.key,areaError:landcoverPrototype.currentData.areaRelativeError,info:landcoverPrototype.currentData.curveInfo}));
    assert.equal(r.info.method,'shared-cubic-bezier-fit');assert.ok(r.areaError<1e-7);reports.push(r);
    if(level==='2'){await p.click('#summit');await p.waitForTimeout(500);await p.screenshot({path:'build/landcover-bezier-fit-fuji-v5/fuji-'+filter+'-fit-comparison.png'});}
   }
  }
  const capabilities=await p.request.get('http://127.0.0.1:61495/wide/capabilities.json');
  if(capabilities.ok()&&(await capabilities.json()).fineOverlayLevels?.includes(3)){
   await p.goto('http://127.0.0.1:61495/wide?lod=3&mode=cleanup-split&filter=majority-clean&curve=fit');
   await p.waitForFunction(()=>window.landcoverPrototype?.currentLevel===3||document.querySelector('#error')?.textContent,null,{timeout:120000});
   assert.equal(await p.locator('#error').textContent(),'');
   const comparison=await p.evaluate(()=>[...landcoverPrototype.cache.values()]
    .map(data=>({key:data.key,areaError:data.areaRelativeError})));
   assert.ok(comparison.some(data=>data.key.includes('fit-overlay-prior')&&data.areaError<1e-7));
   assert.ok(comparison.some(data=>data.key.includes('fit-overlay/')&&data.areaError<1e-7));
  }
  assert.deepEqual(errors,[]);console.log(JSON.stringify(reports,null,2));
 }finally{await b.close();}
})().catch(e=>{console.error(e);process.exit(1)});

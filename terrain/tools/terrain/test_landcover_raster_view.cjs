const {chromium}=require('playwright');
(async()=>{
 const b=await chromium.launch({channel:'msedge',headless:true});
 const p=await b.newPage({viewport:{width:1500,height:1050}});let errors=[];
 p.on('pageerror',e=>errors.push(e.message));
 await p.goto('http://127.0.0.1:61495/?filter=majority');
 await p.waitForFunction(()=>window.landcoverPrototype?.currentData||document.querySelector('#error').textContent,null,{timeout:180000});
 if(await p.locator('#error').textContent())throw Error(await p.locator('#error').textContent());
 const records=[];
 for(const filter of ['majority','regions','none']){
  if(await p.locator('#filter').inputValue()!==filter)await p.selectOption('#filter',filter);
  await p.waitForFunction(f=>window.landcoverPrototype?.filter===f&&document.querySelector('#status').textContent.includes('読込完了')&&window.landcoverPrototype.currentData?.key.startsWith(f+'/'),filter,{timeout:180000});
  await p.selectOption('#lod','2');
  await p.waitForFunction(()=>window.landcoverPrototype?.currentLevel===2&&document.querySelector('#status').textContent.includes('読込完了'),null,{timeout:180000});
  if(await p.locator('#error').textContent())throw Error(await p.locator('#error').textContent());
  records.push(await p.evaluate(()=>({filter:landcoverPrototype.filter,level:landcoverPrototype.currentLevel,areaError:landcoverPrototype.currentData.areaRelativeError,filterInfo:landcoverPrototype.manifest.filter})));
  await p.click('#summit');await p.waitForTimeout(600);
  await p.screenshot({path:'build/landcover-raster-fuji-v4/fuji-'+filter+'-comparison.png'});
 }
 await p.selectOption('#filter','majority');
 await p.waitForFunction(()=>landcoverPrototype.filter==='majority'&&document.querySelector('#status').textContent.includes('読込完了'),null,{timeout:180000});
 await p.selectOption('#mode','vector');await p.waitForTimeout(300);
 await p.selectOption('#mode','filter-split');await p.waitForTimeout(1000);
 if(errors.length)throw Error(errors.join('\n'));
 console.log(JSON.stringify(records,null,2));await b.close();
})().catch(e=>{console.error(e);process.exit(1)});

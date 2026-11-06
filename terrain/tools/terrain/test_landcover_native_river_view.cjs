const {chromium}=require('playwright'),assert=require('assert/strict');
(async()=>{
 const b=await chromium.launch({channel:'msedge',headless:true});
 try{
  const p=await b.newPage({viewport:{width:1550,height:1050}}),errors=[];
  p.on('pageerror',e=>errors.push(e.message));
  await p.goto('http://127.0.0.1:61495/?filter=majority-clean&curve=fit&mode=cleanup-split&lod=4&px=32500&py=69000&span=8000');
  await p.waitForFunction(()=>window.landcoverPrototype?.currentData||document.querySelector('#error').textContent,null,{timeout:120000});
  assert.equal(await p.locator('#error').textContent(),'');
  assert.equal(await p.evaluate(()=>landcoverPrototype.currentLevel),4);
  assert.equal(await p.evaluate(()=>landcoverPrototype.manifest.filterSpanProjectedM),62.5);
  assert.equal(await p.evaluate(()=>landcoverPrototype.manifest.waterHoleAreaProjectedM2[4]),40000);
  await p.screenshot({path:'build/landcover-native-water-holes-bezier-fuji-v1/river-native-L4-before-after.png'});
  await p.selectOption('#mode','vector');await p.waitForFunction(()=>document.querySelector('#status').textContent.includes('読込完了'),null,{timeout:120000});await p.waitForTimeout(200);
  await p.screenshot({path:'build/landcover-native-water-holes-bezier-fuji-v1/river-native-L4-vector.png'});
  assert.deepEqual(errors,[]);console.log('L4 river view, native stage order and no browser errors: passed');
 }finally{await b.close()}
})().catch(e=>{console.error(e);process.exit(1)});

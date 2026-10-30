const assert=require('node:assert/strict');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
(async()=>{const browser=await chromium.launch({channel:'chrome',headless:true});try{
 async function run(baseline){const page=await browser.newPage({viewport:{width:1200,height:800}});await page.route('**/control-gpu128-packet?*',async route=>{await new Promise(r=>setTimeout(r,80));await route.continue();});
 if(baseline)await page.route('**/control-gpu128.js',async route=>{const r=await route.fetch();const text=(await r.text()).replace('changed&&initialCoverageReady&&drawn.length','changed&&drawn.length').replace('return new Map([...all].filter(([key,v])=>v.tile.error===rootError()||committedTiles.has(key)));','return all;');await route.fulfill({status:200,contentType:'text/javascript',body:text});});
 await page.addInitScript(()=>{window.coldMorphFrames=0;function tick(){if(window.gpu128Stats?.pending>0&&gpu128Stats.morphing)coldMorphFrames++;requestAnimationFrame(tick);}requestAnimationFrame(tick);});
 await page.goto('http://127.0.0.1:61489/?region=fuji&quality=auto&source=errorfit-full&detail=2&span=23969&px=64000&py=64000&yaw=-.9328&tilt=1.096&join=1&c1Amount=.5&meshResolution=512');
 await page.waitForFunction(()=>window.gpu128Ready&&gpu128Stats.initialCoverageReady&&gpu128Stats.pending===0&&!gpu128Stats.morphing,{},{timeout:120000});
 const first=await page.evaluate(()=>({frames:coldMorphFrames,snapshots:window.gpu128MorphSnapshots||0,error:window.gpu128Error,tiles:gpu128Stats.drawn.length}));
 if(!baseline){assert.equal(first.frames,0);assert.equal(first.snapshots,0);assert.equal(first.error,undefined);
 await page.evaluate(()=>coldMorphFrames=0);await page.selectOption('#source','pruned');await page.waitForFunction(()=>window.gpu128Ready&&gpu128Stats.source==='pruned'&&gpu128Stats.pending===0&&gpu128Stats.initialCoverageReady,{},{timeout:120000});assert.equal(await page.evaluate(()=>coldMorphFrames),0);assert.equal(await page.evaluate(()=>window.gpu128MorphSnapshots||0),0);}
 await page.close();return first;}
 const before=await run(true),after=await run(false);assert(before.frames>0);console.log(JSON.stringify({before,after}));console.log('初回・表示元変更後の取得中は高さ移行なしを確認');
 }finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1});

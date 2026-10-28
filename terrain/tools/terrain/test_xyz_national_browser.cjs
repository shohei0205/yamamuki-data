'use strict';
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const {chromium}=require('playwright');
(async()=>{
 const output=path.resolve(process.argv[2]);fs.mkdirSync(output,{recursive:true});
 const browser=await chromium.launch({channel:'chrome',headless:true,args:['--use-angle=swiftshader','--enable-unsafe-swiftshader']});
 const page=await browser.newPage({viewport:{width:1440,height:1000},deviceScaleFactor:1});const errors=[];page.on('pageerror',e=>errors.push(e.message));const results=[];
 async function ready(){await page.waitForFunction(()=>{const t=document.getElementById('status').textContent;return t.startsWith('エラー')||t.includes('WASM復元')},{timeout:120000});const status=await page.locator('#status').innerText();assert(!status.startsWith('エラー'),status);assert(!/／ 0 タイル ／/.test(status),status);await page.waitForTimeout(100);const state=await page.evaluate(()=>{const gl=document.getElementById('canvas').getContext('webgl2');return {lost:gl.isContextLost(),error:gl.getError()}});assert.deepEqual(state,{lost:false,error:0});return status}
 try{
  await page.goto('http://127.0.0.1:61463/?region=national&z=6&span=4000000');results.push({region:'national',z:6,status:await ready()});await page.screenshot({path:path.join(output,'national-z6.png')});
  await page.selectOption('#level','10');results.push({region:'national',z:10,status:await ready()});await page.screenshot({path:path.join(output,'national-z10.png')});
  for(const region of ['fuji','yari','tsukuba']){
   await page.goto(`http://127.0.0.1:61463/?region=${region}&z=14&span=8000`);results.push({region,z:14,status:await ready()});await page.screenshot({path:path.join(output,`${region}-z14.png`)});
   for(let z=13;z>=6;z--){await page.selectOption('#level',String(z));results.push({region,z,status:await ready()})}
  }
  await page.goto('http://127.0.0.1:61463/?region=fuji&z=14&span=8000');await ready();
  async function matrix(){await page.waitForTimeout(100);return page.evaluate(()=>{const gl=document.getElementById('canvas').getContext('webgl2'),p=gl.getParameter(gl.CURRENT_PROGRAM);return Array.from(gl.getUniform(p,gl.getUniformLocation(p,'mvp')))})}
  let before=await matrix();await page.mouse.move(650,600);await page.mouse.down({button:'left'});await page.mouse.move(740,640,{steps:8});await page.mouse.up({button:'left'});let after=await matrix();assert.deepEqual(after.slice(0,12),before.slice(0,12),'左ドラッグが回転してしまう');assert.notDeepEqual(after.slice(12),before.slice(12),'左ドラッグで移動しない');assert(after[12]/after[15]>before[12]/before[15],'右へドラッグしたのに地形が左へ動く');assert(-after[13]/after[15]>-before[13]/before[15],'下へドラッグしたのに地形が上へ動く');
  before=after;await page.mouse.move(650,600);await page.mouse.down({button:'right'});await page.mouse.move(730,630,{steps:8});await page.mouse.up({button:'right'});after=await matrix();assert.notDeepEqual(after.slice(0,12),before.slice(0,12),'右ドラッグで回転しない');before=after;await page.mouse.move(650,600);await page.mouse.down({button:'left'});await page.mouse.move(710,630,{steps:8});await page.mouse.up({button:'left'});after=await matrix();assert(after[12]/after[15]>before[12]/before[15],'回転後の左右移動が逆');assert(-after[13]/after[15]>-before[13]/before[15],'回転後の上下移動が逆');
  before=after;await page.mouse.wheel(0,-200);after=await matrix();assert.notDeepEqual(after,before,'ホイールで拡大縮小しない');
  await page.uncheck('#smooth');await page.check('#wire');await page.waitForTimeout(100);await page.screenshot({path:path.join(output,'fuji-z14-flat-wire.png')});
  assert.deepEqual(errors,[]);fs.writeFileSync(path.join(output,'browser-verification.json'),JSON.stringify({passed:true,scenes:results.length,results,controls:{leftDrag:'pan',rightDrag:'rotate',wheel:'zoom'},errors},null,2)+'\n');console.log(JSON.stringify({passed:true,scenes:results.length,controls:true,output}));
 }finally{await browser.close()}
})().catch(e=>{console.error(e);process.exitCode=1});

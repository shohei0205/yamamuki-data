// 全国表示で陰影方式だけを切り替え、画角・地形・光の設定が保持されることを確かめる。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});try{
 const page=await browser.newPage({viewport:{width:1300,height:1050}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61440/national-shading');await page.waitForFunction(()=>window.nationalShadingStatus);
 const before=await page.evaluate(()=>window.nationalShadingStatus);assert.equal(before.geometryLevel,4);assert.equal(before.glError,0);
 await page.evaluate(()=>{draw();window.beforePixels=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,window.beforePixels);});
 await page.screenshot({path:'build/terrain-national-shading/normal-preview.png'});
 await page.click('#shadeMode');
 const after=await page.evaluate(()=>window.nationalShadingStatus);
 for(const key of ['center','scale','bearing','tilt','strength','contrast','triangles'])assert.deepEqual(after[key],before[key]);assert.equal(after.mode,'shade');assert.equal(after.glError,0);
 const diff=await page.evaluate(()=>{draw();const p=new Uint8Array(beforePixels.length);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,p);let delta=0;for(let i=0;i<p.length;i++)if(i%4!==3)delta+=Math.abs(p[i]-beforePixels[i]);return delta/(p.length/4*3);});assert.ok(diff>.01,'切替前後の陰影が違うこと');
 await page.screenshot({path:'build/terrain-national-shading/shade-preview.png'});
 await page.selectOption('#normalResolution','equal');await page.click('#normalMode');assert.equal(await page.evaluate(()=>window.nationalShadingStatus.resolution),'equal');
 await page.locator('#tilt').fill('45');await page.locator('#strength').fill('0.6');await page.mouse.move(650,500);await page.mouse.wheel(0,-300);const view=await page.evaluate(()=>window.nationalShadingStatus);
 await page.click('#shadeMode');for(const key of ['center','scale','bearing','tilt','strength'])assert.deepEqual(await page.evaluate(k=>window.nationalShadingStatus[k],key),view[key]);
 await page.click('#reset');assert.equal(await page.evaluate(()=>window.nationalShadingStatus.tilt),0);assert.deepEqual(errors,[]);
 console.log(JSON.stringify({sourceLevel:before.sourceLevel,geometryLevel:before.geometryLevel,triangles:before.triangles,normalBytes:before.bytes,shadeBytes:after.bytes,pixelDifference:diff,errors}));
}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});

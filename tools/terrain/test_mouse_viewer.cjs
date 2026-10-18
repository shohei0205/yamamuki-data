// 実際のマウス入力で視点移動、拡大縮小、回転と傾き、復帰を確認する。
const assert=require('node:assert/strict');
const {chromium}=require('playwright');
async function main(){
  const browser=await chromium.launch({headless:true,channel:'msedge'});
  try{
    const page=await browser.newPage({viewport:{width:768,height:1024}});
    const errors=[];page.on('pageerror',e=>errors.push(e.message));
    await page.goto(process.argv[2]||'http://127.0.0.1:61434/terrain?lat=35.40&lon=138.73&size=30&fullscreen=1');
    await page.waitForFunction(()=>document.querySelector('#stats').textContent.includes('三角形'),{timeout:60000});
    await page.waitForFunction(()=>!dynamicLOD||(!lodBusy&&(!data.sqliteRelay||!!data.overlays)&&!data.coverage.overviewFallback&&!data.coverage.progressiveTarget&&boundsContain(loadedBounds,viewFootprint().bounds)),null,{timeout:120000});
    const state=()=>page.evaluate(()=>({pan:[...viewPan],center:[...viewCenter],dynamic:dynamicLOD,zoom:Number(document.querySelector('#zoom').value),minimum:Number(document.querySelector('#zoom').min),maximum:Number(document.querySelector('#zoom').max),bearing:Number(document.querySelector('#bearing').value),tilt:Number(document.querySelector('#tilt').value),maximumTilt:Number(document.querySelector('#tilt').max),shadow:shadowRevision,error:gl.getError(),image:canvas.toDataURL()}));
    const settle=()=>page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
    const initial=await state();
    await page.mouse.move(350,400);await page.mouse.down();await page.mouse.move(450,460,{steps:8});await page.mouse.up();await settle();
    const pan=await state();if(pan.dynamic)assert.notDeepEqual(pan.center.slice(0,2),initial.center.slice(0,2));else assert.ok(pan.pan[0]>0&&pan.pan[1]<0);assert.notEqual(pan.image,initial.image);assert.equal(pan.shadow,initial.shadow);
    await page.mouse.wheel(0,-300);await settle();const zoom=await state();assert.ok(zoom.zoom>pan.zoom);assert.notEqual(zoom.image,pan.image);
    await page.mouse.down({button:'right'});await page.mouse.move(550,520,{steps:8});await page.mouse.up({button:'right'});await settle();
    const rotate=await state();assert.notEqual(rotate.bearing,zoom.bearing);assert.ok(rotate.tilt>zoom.tilt);assert.deepEqual(rotate.pan,zoom.pan);assert.deepEqual(rotate.center.slice(0,2),zoom.center.slice(0,2));assert.notEqual(rotate.image,zoom.image);
    await page.keyboard.down('Shift');await page.mouse.down();await page.mouse.move(520,500,{steps:4});await page.mouse.up();await page.keyboard.up('Shift');await settle();
    const shift=await state();assert.notEqual(shift.bearing,rotate.bearing);assert.deepEqual(shift.pan,rotate.pan);
    await page.mouse.wheel(0,-100000);await settle();assert.ok(Math.abs((await state()).zoom-initial.maximum)<1e-9);
    await page.mouse.wheel(0,100000);await settle();assert.equal((await state()).zoom,initial.minimum);
    await page.mouse.dblclick(400,400);await settle();
    const reset=await state();assert.deepEqual(reset.pan,[0,0]);assert.deepEqual(reset.center.slice(0,2),initial.center.slice(0,2));assert.equal(reset.zoom,initial.zoom);assert.equal(reset.bearing,initial.bearing);assert.equal(reset.tilt,initial.tilt);
    await page.mouse.down({button:'right'});await page.mouse.move(450,1000);await page.mouse.up({button:'right'});await settle();assert.equal((await state()).tilt,initial.maximumTilt);
    await page.locator('#resetView').click();await settle();assert.equal((await state()).tilt,initial.tilt);
    assert.deepEqual(errors,[]);for(const s of [initial,pan,zoom,rotate,shift,reset])assert.equal(s.error,0);
    console.log('マウスの移動・拡大縮小・右ドラッグ・Shift操作・範囲制限・視点復帰・描画変化を確認しました。');
  }finally{await browser.close();}
}
main().catch(e=>{console.error(e);process.exitCode=1;});

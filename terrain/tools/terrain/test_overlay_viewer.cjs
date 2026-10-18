// 保存済みの実データでレイヤー独立性、遮蔽、全画面と外部通信なしを確認する。
const assert = require('node:assert/strict');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { chromium } = require('playwright');
async function main() {
  const browser = await chromium.launch({headless:true,channel:'msedge'});
  try {
    const page = await browser.newPage({viewport:{width:1024,height:1024}});
    const errors=[],requests=[];
    page.on('pageerror',e=>errors.push(e.message));
    await page.route(/^https?:/,route=>{requests.push(route.request().url());return route.abort();});
    await page.goto(pathToFileURL(path.resolve(process.argv[2]||'build/terrain-national/fuji-layers.html')).href);
    await page.waitForFunction(()=>document.querySelector('#stats').textContent.includes('三角形'));
    await page.evaluate(()=>{
      document.querySelector('#method').value='shaded';
      document.querySelector('#zoom').value='1.7';
      document.querySelector('#tilt').value='35';
      document.querySelector('#bearing').value='180';
      document.querySelector('#sunHeight').value='35';
      document.querySelector('#fullscreenLayer').checked=true;
      document.querySelector('#fullscreenLayer').dispatchEvent(new Event('change'));
    });
    async function modes(visible,occlude=true) {
      return page.evaluate(({visible,occlude})=>{
        for(const id of ['terrain','water','trail','boundary'])document.querySelector('#'+id+'Layer').checked=visible.includes(id);
        document.querySelector('#occludeLayers').checked=occlude;draw();
        const pixels=new Uint8Array(canvas.width*canvas.height*4);
        gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,pixels);
        let blue=0,orange=0,purple=0,colored=0;
        for(let i=0;i<pixels.length;i+=4){const [r,g,b]=pixels.slice(i,i+3);
          if(b>r+40&&b>g+30)blue++;
          if(r>180&&r>g*2&&g>b)orange++;
          if(r>80&&b>100&&b>g*2&&r>g*2)purple++;
          if(r<235||g<235||b<230)colored++;
        }
        return {blue,orange,purple,colored,error:gl.getError(),width:canvas.width,height:canvas.height};
      },{visible,occlude});
    }
    const all=await modes(['terrain','water','trail','boundary']);
    assert.ok(all.blue>100&&all.orange>20&&all.purple>20);
    await page.screenshot({path:'build/terrain-national/fuji-all-layers.png'});
    const water=await modes(['water']);
    assert.ok(water.blue>100);assert.equal(water.orange,0);assert.equal(water.purple,0);
    await page.screenshot({path:'build/terrain-national/fuji-water-only.png'});
    const through=await modes(['water'],false);assert.ok(through.blue>=water.blue);
    const terrain=await modes(['terrain']);assert.equal(terrain.blue,0);assert.equal(terrain.orange,0);assert.equal(terrain.purple,0);
    const none=await modes([],false);assert.equal(none.colored,0);
    for(const r of [all,water,through,terrain,none])assert.equal(r.error,0);
    assert.equal(all.width,1024);assert.equal(all.height,1024);
    assert.deepEqual(errors,[]);assert.deepEqual(requests,[]);
    console.log(JSON.stringify({all,water,through,terrain,none},null,2));
  } finally {await browser.close();}
}
main().catch(e=>{console.error(e);process.exitCode=1;});

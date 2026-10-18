// 全レベルの復元誤差と表示、GPU 用タイル参照経路を確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{
 const page=await browser.newPage({viewport:{width:1000,height:700}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61440/heightmap?level=14');await page.waitForFunction(()=>window.heightmapStatus?.level===14);
 const results=await page.evaluate(async()=>{
  const report=await(await fetch('/height-astc/report.json')).json();if(report.state!=='complete')throw Error('標高 ASTC の生成が未完了です');
  const result=[];
  for(let level=14;level>=4;level--){
   const img=await createImageBitmap(await(await fetch('/levels/'+level+'/height.png')).blob(),{colorSpaceConversion:'none'}),c=new OffscreenCanvas(img.width,img.height),ctx=c.getContext('2d');ctx.drawImage(img,0,0);const p=ctx.getImageData(0,0,img.width,img.height).data;img.close();
   const info=levelInfos.find(m=>m.level===level);
   for(const codec of ['height-astc8','height-astc12']){
    const decoded=await TerrainCodecs.loadHeight(level,codec);let max=0,sum=0,count=0,missing=0,boundary=0;
    const errorAt=i=>decoded.values[i]-(p[i*4]+p[i*4+1]*256+info.heightOffset);
    for(let y=0;y<c.height;y++)for(let x=0;x<c.width;x++){const i=y*c.width+x,valid=p[i*4+2]>0;if(valid!==Number.isFinite(decoded.values[i]))missing++;if(!valid)continue;const e=errorAt(i);max=Math.max(max,Math.abs(e));sum+=e*e;count++;
     if(x>0&&x%256===0&&p[(i-1)*4+2]>0)boundary=Math.max(boundary,Math.abs(e-errorAt(i-1)));
     if(y>0&&y%256===0&&p[(i-c.width)*4+2]>0)boundary=Math.max(boundary,Math.abs(e-errorAt(i-c.width)));
    }
    const row=report.results.find(r=>r.level===level&&r.codec===codec);result.push({level,codec,max,rmse:Math.sqrt(sum/count),missing,boundary,expected:row});
   }
  }
  return result;
 });
 for(const r of results){assert.equal(r.missing,0);assert.ok(Math.abs(r.max-r.expected.maxErrorMeters)<.005);assert.ok(Math.abs(r.rmse-r.expected.rmseMeters)<.002);assert.ok(Math.abs(r.boundary-r.expected.boundaryResidualJumpMaxMeters)<.005);}
 for(const level of [14,13,12,11,10,9,8,7,6,5,4]){
  await page.selectOption('#dataLevel',String(level));await page.waitForFunction(l=>heightmapStatus.level===l&&!document.getElementById('recommended').disabled,level);
  for(const codec of ['height-astc8','height-astc12']){await page.selectOption('#heightCodec',codec);await page.waitForFunction(c=>heightmapStatus.heightCodec===c&&!document.getElementById('recommended').disabled,codec);assert.equal((await page.evaluate(()=>heightmapStatus)).glError,0);}
 }
 await page.goto('http://127.0.0.1:61440/codec-report');await page.waitForFunction(()=>document.querySelectorAll('#heightASTC tr').length===2);assert.deepEqual(errors,[]);
 // ASTC 非対応 GPU でも、復号済みテクセルを使って実際のタイル配列シェーダーを検証する。
 // ASTC ブロックのハードウェア復号そのものを確認するテストではない。
 const context=await browser.newContext({viewport:{width:800,height:600}});
 await context.addInitScript(()=>{
  const getContext=HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext=function(type,options){const g=getContext.call(this,type,options);if(type==='webgl2'&&g&&!g.astcTestWrapped){g.astcTestWrapped=true;const ext=g.getExtension.bind(g),image3D=g.texImage3D.bind(g);
   g.getExtension=name=>name==='WEBGL_compressed_texture_astc'?{COMPRESSED_RGBA_ASTC_8x8_KHR:1,COMPRESSED_RGBA_ASTC_12x12_KHR:2}:ext(name);
   g.compressedTexImage3D=(target,level,format,w,h,l,border)=>{if(!window.astcTestPixels)throw Error('GPU 比較用テクセルがありません');image3D(target,level,g.RGBA8,w,h,l,0,g.RGBA,g.UNSIGNED_BYTE,astcTestPixels);window.astcArrayPathTested=true;};
  }return g;};
 });
 const gpu=await context.newPage();await gpu.goto('http://127.0.0.1:61440/heightmap?level=11');await gpu.waitForFunction(()=>window.heightmapStatus?.level===11);
 await gpu.evaluate(async()=>{
  const d=await TerrainCodecs.loadHeightASTC(11,'height-astc12'),h=d.header,img=await createImageBitmap(await(await fetch(d.row.previewUrl)).blob(),{colorSpaceConversion:'none'}),c=new OffscreenCanvas(h.width,h.height),ctx=c.getContext('2d');ctx.drawImage(img,0,0);const p=ctx.getImageData(0,0,h.width,h.height).data;img.close();
  window.astcTestPixels=new Uint8Array(h.layers*h.side*h.side*4);
  for(let layer=0;layer<h.layers;layer++){const tile=h.tiles[layer];for(let y=0;y<h.side;y++)for(let x=0;x<h.side;x++){const sx=tile.x*h.core+Math.max(0,Math.min(h.core-1,x-h.padding)),sy=tile.y*h.core+Math.max(0,Math.min(h.core-1,y-h.padding)),v=p[(sy*h.width+sx)*4],i=(layer*h.side*h.side+y*h.side+x)*4;astcTestPixels.set([v,v,v,255],i);}}
  // 欠損領域をタイル境界にまたがって置き、ビットマスク参照も確認する。
  for(let y=248;y<265;y++)for(let x=248;x<265;x++)d.mask[y*h.width+x]=0;
 });
 await gpu.selectOption('#normal','gpu');await gpu.selectOption('#heightCodec','height-astc12');await gpu.waitForFunction(()=>heightmapStatus.heightCodec==='height-astc12'&&!document.getElementById('recommended').disabled);
 const arrayError=await gpu.evaluate(async()=>{
  if(!window.astcArrayPathTested||!heightASTC)throw Error('タイル配列の参照経路が使われていません');
  const capture=()=>{draw();const p=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,p);return p;};
  const a=capture(),decoded=await TerrainCodecs.loadHeight(11,'height-astc12');uploadFloatHeight(decoded.values);heightASTC=false;const b=capture();let sum=0;for(let i=0;i<a.length;i++)if(i%4!==3)sum+=Math.abs(a[i]-b[i]);return {mae:sum/(a.length*.75),glError:gl.getError()};
 });
 assert.equal(arrayError.glError,0);assert.ok(arrayError.mae<.1,JSON.stringify(arrayError));
 console.log(JSON.stringify({allLevelsAndErrorsVerified:true,gpuArrayShaderEmulated:arrayError,hardwareASTCDecoderVerified:false,results:results.filter(r=>r.level===14).map(({expected,...rest})=>rest)}));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});

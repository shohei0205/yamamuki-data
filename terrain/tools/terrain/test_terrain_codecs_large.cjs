// 最大解像度の標高を実際に復元し、PNG の全画素と照合する。
const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{
 const page=await browser.newPage();await page.goto('http://127.0.0.1:61440/heightmap?level=14');
 await page.waitForFunction(()=>window.heightmapStatus?.level===14);
 const result=await page.evaluate(async()=>{
  const image=await createImageBitmap(await(await fetch('/levels/14/height.png')).blob(),{colorSpaceConversion:'none'});
  const canvas=new OffscreenCanvas(image.width,image.height),context=canvas.getContext('2d');
  context.drawImage(image,0,0);const source=context.getImageData(0,0,image.width,image.height).data;image.close();
  const rows=[];
  for(const codec of ['ymd-xz-1','lerc-0','residual-zstd']){
   const decoded=await TerrainCodecs.loadHeight(14,codec);let max=0,missing=0;
   for(let i=0;i<decoded.values.length;i++){
    const valid=source[4*i+2]>0;if(valid!==Number.isFinite(decoded.values[i]))missing++;
    if(valid)max=Math.max(max,Math.abs(decoded.values[i]-source[4*i]-256*source[4*i+1]-heightmapMetadata.heightOffset));
   }
   rows.push({codec,max,missing,decodeMs:decoded.decodeMs});
  }
  return rows;
 });
 for(const row of result){assert.equal(row.max,0);assert.equal(row.missing,0);}
 console.log(JSON.stringify(result));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});

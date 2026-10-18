// 地形に隠れた山頂を除外し、標高差の許容とデータ切替を確認する。
const assert=require('node:assert/strict');
const {chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{const page=await browser.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
await page.goto('http://127.0.0.1:61434/terrain?lat=36.3419&lon=137.6476&size=12&fullscreen=1',{timeout:60000});
const result=await page.evaluate(()=>{const mesh=data.mesh,indices=data.indices;
function ridge(h){data.mesh=[[-10,0,0],[10,0,0],[-10,5,h],[10,5,h],[-10,10,0],[10,10,0]];data.indices=[0,1,2,1,3,2,2,3,4,3,5,4];}
try{ridge(200);const hidden=terrainOccludesPeak([0,0,100],[0,10,100],5);
const topVisible=!terrainOccludesPeak([0,5,300],[0,5,200],5);
const outsideHidden=terrainOccludesPeak([0,-20,100],[0,10,100],5);
ridge(50);const lowVisible=!terrainOccludesPeak([0,0,100],[0,10,100],5);
ridge(110);const toleranceVisible=!terrainOccludesPeak([0,0,100],[0,10,100],20);
return {hidden,topVisible,outsideHidden,lowVisible,toleranceVisible};
}finally{data.mesh=mesh;data.indices=indices;}});
for(const [name,value] of Object.entries(result))assert.equal(value,true,name);
assert.deepEqual(errors,[]);console.log(result);
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});

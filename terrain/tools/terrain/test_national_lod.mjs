import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
import test from 'node:test';

const source=readFileSync(new URL('./national_mesh.js',import.meta.url),'utf8');
const advance=source.slice(source.indexOf('function advanceLod(){'),source.indexOf('let priorityProgram='));
function state(){
 const context=vm.createContext({selectedLevel:4,presentedLevel:3,lodTransition:null,
  wantedKeys:new Set(['new']),cache:new Map([['new',{}]]),coverageDirty:false,dirty:false,
  performance:{now:()=>100},$:()=>({checked:true}),lodFloatSupport:true,
  prepareLodMorph:()=>true,updateLodClock:t=>{},lodBlendValue:()=>0,capturePreviousLod:()=>{throw Error('山頂を再標本化してはいけない');}});
 return {context,run:()=>vm.runInContext(advance+';advanceLod();',context)};
}
test('切替では旧地形を画像化せず新旧の形状を保持する',()=>{
 const {context:c,run}=state();run();assert.equal(c.presentedLevel,3);
 assert.equal(c.lodTransition.from,3);assert.equal(c.lodTransition.atlas,undefined);
 assert.equal(c.coverageDirty,true);
});
test('読み込み中は旧LODを保つ',()=>{
 const {context:c,run}=state();c.cache.clear();run();assert.equal(c.presentedLevel,3);assert.equal(c.lodTransition,null);
});
test('連続操作では現在の切替を完了してから次に移る',()=>{
 const {context:c,run}=state();run();const first=c.lodTransition;c.selectedLevel=5;run();
 assert.equal(c.lodTransition,first);assert.equal(c.presentedLevel,3);
 c.updateLodClock=t=>{t.blend=1;};run();assert.notEqual(c.lodTransition,null);run();assert.equal(c.lodTransition,null);
 c.lodBlendValue=()=>0;run();assert.equal(c.presentedLevel,4);assert.equal(c.lodTransition.from,4);
});

test('粗い面の共有山頂と面上の高さを画像化せず復元する',()=>{
 const c=vm.createContext({});const start=source.indexOf('function createLodSurface('),end=source.indexOf("\n'use strict';const $",start);
 vm.runInContext(source.slice(start,end),c);
 const mesh={points:new Float32Array([0,0,0,100,0,0,0,100,100]),indices:new Uint32Array([0,1,2]),normals:new Float32Array([0,0,1,0,0,1,0,0,1]),origin:[5000,-2000]};
 const sample=c.createLodSurface([mesh]);assert.equal(sample(5000,-1900)[0],100);
 assert.equal(sample(5025,-1975)[0],25);assert.equal(sample(5200,-1800),null);
});
test('詳細化は面から展開し、粗化は詳細メッシュのまま面へ戻る',()=>{
 const {context:c,run}=state();c.presentedLevel=4;c.selectedLevel=3;run();assert.equal(c.presentedLevel,3);assert.equal(c.lodTransition.coarsening,false);
 c.updateLodClock=t=>{t.blend=1;};run();assert.equal(c.presentedLevel,3);assert.notEqual(c.lodTransition,null);run();assert.equal(c.lodTransition,null);
});

test('一フレーム内は一定の補間率を使い、時計が戻っても逆行しない',()=>{
 const start=source.indexOf('function lodBlendValue(){'),end=source.indexOf('function resetLodTransition()',start);
 const c=vm.createContext({lodTransition:{start:100,blend:0},performance:{now:()=>99999}});vm.runInContext(source.slice(start,end),c);
 c.updateLodClock(c.lodTransition,200);const blend=c.lodBlendValue();assert.ok(blend>0&&blend<1);
 assert.equal(c.lodBlendValue(),blend);c.updateLodClock(c.lodTransition,150);assert.equal(c.lodBlendValue(),blend);
 c.updateLodClock(c.lodTransition,450);assert.equal(c.lodBlendValue(),1);
});

test('開始フレームは準備時間によらず旧形状を使い、端点まで一方向に進む',()=>{
 const {context:c,run}=state();let calls=0;c.updateLodClock=t=>{calls++;t.blend=1;};
 run();assert.equal(calls,0);assert.equal(c.lodTransition.blend,0);assert.equal(c.lodTransition.coarsening,true);
 run();assert.equal(c.lodTransition.blend,1);assert.equal(c.presentedLevel,3);
 run();assert.equal(c.lodTransition,null);assert.equal(c.presentedLevel,4);
});

test('対角線が異なるLODでも共通分割の全ての面が両端に一致する',()=>{
 const start=source.indexOf('function createLodSurface('),end=source.indexOf("\n'use strict';const $",start),c=vm.createContext({});vm.runInContext(source.slice(start,end),c);
 const normals=new Float32Array([0,0,1,0,0,1,0,0,1,0,0,1]);
 const coarse={points:new Float32Array([0,0,0,10,0,0,10,10,100,0,10,0]),indices:new Uint32Array([0,1,2,0,2,3]),normals,origin:[0,0]};
 const fine={points:new Float32Array([0,0,0,10,0,10,10,10,110,0,10,10]),indices:new Uint32Array([0,1,3,1,2,3]),normals,origin:[0,0]};
 const a=c.createLodSurface([coarse]),b=c.createLodSurface([fine]),overlay=c.createLodOverlay(fine,a);assert.equal(overlay.length/30,4);let area=0;
 for(let i=0;i<overlay.length;i+=30){let x=0,y=0,z0=0,z1=0;for(let j=0;j<3;j++){const k=i+j*10;x+=overlay[k]/3;y+=overlay[k+1]/3;z0+=overlay[k+6]/3;z1+=overlay[k+2]/3;}
  assert.ok(Math.abs(z0-a(x,y)[0])<1e-4);assert.ok(Math.abs(z1-b(x,y)[0])<1e-4);
  area+=Math.abs((overlay[i+10]-overlay[i])*(overlay[i+21]-overlay[i+1])-(overlay[i+11]-overlay[i+1])*(overlay[i+20]-overlay[i]))/2;
 }assert.equal(area,100);
});

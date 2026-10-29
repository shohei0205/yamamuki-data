import assert from 'node:assert/strict';
import {visibleDepth} from './natural_c1_128_lod.mjs';
const project=(x,y,h)=>{const z=10000+y;return {x:x/z,y:0,z};};
const near=visibleDepth([-1000,-9000,1000,10000],project),far=visibleDepth([-1000,10000,1000,20000],project);
assert.equal(near,1000);assert.equal(far,20000);assert(near<far);
// 中心が遠くても手前の部分を含む親は、子への分割を止めない。
assert(visibleDepth([-1000,-9000,1000,20000],project)<=near);
// 画面外の手前の隅で遠景全体を最詳細にしない。
const clipped=visibleDepth([10000,-9990,20000,20000],project);assert(clipped>8000);
console.log('傾斜視点の近接部分・親子・画面外除外によるLOD深度を確認');

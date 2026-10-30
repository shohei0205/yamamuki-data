import assert from 'node:assert/strict';
import {morphWeight,coverageKey,MORPH_MS} from './natural_c1_128_morph.mjs';
assert.equal(morphWeight(-Infinity,0),1);assert.equal(morphWeight(100,100),0);assert.equal(morphWeight(100,100+MORPH_MS),1);
let last=0;for(let t=0;t<=MORPH_MS;t++){const w=morphWeight(0,t);assert(w>=last&&w<=1);last=w;}
const mix=(a,b,w)=>a+(b-a)*w,current=mix(1200,3000,morphWeight(0,170));assert.equal(mix(current,2200,morphWeight(170,170)),current);assert.equal(mix(current,2200,morphWeight(170,620)),2200);
const a={offset:1,tile:{bounds:[0,0,10,10]}},b={offset:2,tile:{bounds:[10,0,20,10]}};assert.equal(coverageKey([a,b]),coverageKey([b,a]));assert.notEqual(coverageKey([a]),coverageKey([{...a,drawBounds:[0,0,5,5]}]));
console.log('450msの連続移行・途中変更時の連続性・表示範囲の変更検出を確認');

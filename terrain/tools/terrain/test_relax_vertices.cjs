const assert=require('node:assert/strict'),{relaxTerrainVertices}=require('./hct-terrain.js');
const p=[],idx=[],g=[];for(let y=0;y<5;y++)for(let x=0;x<5;x++){p.push(x*100,y*100,.2*x*100+.1*y*100);g.push(.2,.1);}for(let y=0;y<4;y++)for(let x=0;x<4;x++){const a=y*5+x;idx.push(a,a+1,a+5,a+1,a+6,a+5);}const points=Float32Array.from(p),indices=Uint32Array.from(idx),slopes=Float32Array.from(g),copy=points.slice(),gc=slopes.slice();const flat=relaxTerrainVertices(points,indices,slopes,[0,400,0,400],5);assert.ok(flat.stats.maxShift<1e-5);assert.deepEqual(points,copy);assert.deepEqual(slopes,gc);
points[12*3+2]=200;slopes[11*2]=1.5;points[6*3+2]+=8;const r=relaxTerrainVertices(points,indices,slopes,[0,400,0,400],2);
assert.ok(r.stats.moved>0);assert.ok(r.stats.maxShift<=2.00001);assert.equal(r.points[12*3+2],200);assert.ok(r.stats.maxSlopeChange>0);
for(let i=0;i<25;i++){assert.equal(r.points[i*3],points[i*3]);assert.equal(r.points[i*3+1],points[i*3+1]);if(i%5===0||i%5===4||i<5||i>=20){assert.equal(r.points[i*3+2],points[i*3+2]);assert.equal(r.slopes[i*2],slopes[i*2]);assert.equal(r.slopes[i*2+1],slopes[i*2+1]);}}
console.log('平面・外周・山頂高さ・XY固定、移動量制限、元配列不変を確認',r.stats);

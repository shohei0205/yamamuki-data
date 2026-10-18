const assert=require('node:assert/strict'),m=require('./height-mesh.js');
assert.equal(m.diagonal(0,10,10,0,0,0,0),true);assert.equal(m.diagonal(0,10,10,0,10,0,0),false);
assert.equal(m.diagonal(0,1,2,3,1.5,0,0),false);assert.equal(m.diagonal(1,1,1,1,1,0,0),true);assert.equal(m.diagonal(1,1,1,1,1,1,0),false);
const n=4,values=new Float32Array(16);for(let y=0;y<4;y++)for(let x=0;x<4;x++)values[y*4+x]=x*x-y*y;const indices=m.indices(values,[0,0,1,1],n,null,null,4);assert.equal(indices.length,n*n*6);
for(let y=0;y<n;y++)for(let x=0;x<n;x++){const a=y*(n+1)+x,allowed=new Set([a,a+1,a+n+1,a+n+2]),t=[...indices.slice((y*n+x)*6,(y*n+x+1)*6)];assert.ok(t.every(i=>allowed.has(i)));assert.equal(new Set(t).size,4);}
const previous=indices.slice(),edges=new Float32Array((n+1)*4*2).fill(1000);m.indices(values,[0,0,1,1],n,edges,indices,4);for(let y=1;y<n-1;y++)for(let x=1;x<n-1;x++)assert.deepEqual(indices.slice((y*n+x)*6,(y*n+x+1)*6),previous.slice((y*n+x)*6,(y*n+x+1)*6));
console.log('対角線：標高中点への誤差・勾配・同点時の方向分散・三角形接続・境界のみ更新 OK');

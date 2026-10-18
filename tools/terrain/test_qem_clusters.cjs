// 配信タイルの索引より広いメッシュを、重複せず正しい位置に投影する。
const assert=require('node:assert/strict'),core=require('./terrain-client-core.js');
const map=Object.fromEntries(Array.from({length:11},(_,i)=>[i+4,Math.max(2,i+2)]));
const meta={levels:Array.from({length:11},(_,i)=>i+4),geometryZoomByLevel:map,pixelOrigin:'highest-grid-corners',meshErrorBounds:Object.fromEntries(Array.from({length:11},(_,i)=>[i+4,600]))};
const x=3614,y=1603,z=12,lon=(x+.5)/2**z*360-180,lat=Math.atan(Math.sinh(Math.PI*(1-2*(y+.5)/2**z)))*180/Math.PI;
const plan=core.selection(lat,lon,2,14,null,true,meta);assert.deepEqual(plan.tiles,[[14,x,y]]);
const rows=new Map([[`14,${x},${y}`,{points:[[0,0,0],[256,0,0],[0,256,0],[256,256,0]],indices:[0,1,2,1,3,2],bytes:100}]]);
const result=core.assembleTerrain(plan,rows,meta,lat,lon,2,null,true);
assert.equal(result.indices.length,6);assert.equal(result.mesh.length,4);assert.ok(result.span>7000&&result.span<9000);
assert.equal(result.coverage.pixelOrigin,'highest-grid-corners');assert.equal(result.coverage.geometryZoomByLevel[14],12);
assert.equal(core.geometryZoom(14,{}),14);
for(const level of [4,5,6,8,11,14]){
 const plan=core.selection(lat,lon,30,level,[75,180,25000],false,meta);
 assert.equal(new Set(plan.tiles.map(t=>t.join(','))).size,plan.tiles.length);
 for(const [lod,x,y] of plan.tiles){const zoom=core.geometryZoom(lod,meta);assert.ok(x>=0&&x<2**zoom&&y>=0&&y<2**zoom);}
}
console.log('QEM cluster projection, unique selection and tile-crossing geometry: OK');

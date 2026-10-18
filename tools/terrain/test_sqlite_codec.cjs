// Python の既存デコーダーと各レベル・混在レベルの形状を比較する。
const assert=require('node:assert/strict'),fs=require('node:fs'),crypto=require('node:crypto');
global.self=global;global['xz-decompress']=require('./vendor/xz-decompress-0.2.3.min.js');
global.earcut=require('./vendor/earcut-3.2.4.min.js');global.polygonClipping=require('./vendor/polygon-clipping-0.15.7.min.js');
const core=require('./terrain-client-core.js'),sha=bytes=>crypto.createHash('sha256').update(Buffer.from(bytes.buffer,bytes.byteOffset,bytes.byteLength)).digest('hex');
(async()=>{
 const fixture=JSON.parse(fs.readFileSync('build/terrain-national/client-codec-fixture.json','utf8'));
 for(const tile of fixture.tiles){const value=await core.decodeMesh(new Uint8Array(Buffer.from(tile.body,'base64')));assert.equal(sha(new Float32Array(value.points.flat())),tile.pointsSha,'頂点 '+tile.z);assert.equal(sha(new Uint32Array(value.indices)),tile.indicesSha,'三角形 '+tile.z);}
 for(const {args,records,expected} of fixture.regions){const [lat,lon,size,z,view]=args,fixed=z===4,plan=core.selection(lat,lon,size,z,view,fixed,fixture.metadata),rows=new Map();for(const tile of records){const body=new Uint8Array(Buffer.from(tile.body,'base64'));rows.set([tile.z,tile.x,tile.y].join(','),{...await core.decodeMesh(body),bytes:body.length});}
  const result=core.assembleTerrain(plan,rows,fixture.metadata,lat,lon,size,view,fixed);
  assert.equal(result.mesh.length,expected.vertices);assert.equal(sha(result.indices),expected.indicesSha);assert.deepEqual(result.coverage.levels,expected.coverage.levels);
  result.projectionCenter.forEach((v,i)=>assert.ok(Math.abs(v-expected.projectionCenter[i])<1e-7));
  for(let i=0;i<expected.sample.length;i++)result.mesh[i*expected.stride].forEach((v,k)=>assert.ok(Math.abs(v-expected.sample[i][k])<1e-5,`${z} ${i} ${k}`));
 }
 const rawLines=await new Response(new Blob([Buffer.from(fixture.lines.body,'base64')]).stream().pipeThrough(new DecompressionStream('gzip'))).arrayBuffer();
 assert.equal(sha(new Float64Array(core.decodeLines(new Uint8Array(rawLines)).flat(2))),fixture.lines.sha);
 const mesh=[[0,0,0],[10,0,10],[0,10,20],[10,10,30]],indices=[0,1,2,1,3,2],surface=new core.TerrainSurface(mesh,indices);
 assert.equal(surface.height(2,3),8);assert.ok(Number.isNaN(surface.height(-1,0)));
 assert.deepEqual(core.clipSegment([-1,5],[11,5],[0,0,10,10]),[[0,5],[10,5]]);
 assert.equal(core.clipSegment([-1,-1],[-2,-2],[0,0,10,10]),null);
 assert.throws(()=>core.selection(36,138,4000,14,null,true,fixture.metadata),/拡大/);
 assert.throws(()=>core.selection(NaN,138,30,12,null,false,fixture.metadata),/範囲/);
 await assert.rejects(core.decodeMesh(new Uint8Array(2)),/ヘッダー/);
 // 湖の穴（島）を残し、水面と地形を同じ境界で切り分ける。
 const radius=6378137,scale=Math.PI/180*radius;
 const polygon=[[[1,1],[9,1],[9,9],[1,9],[1,1]],[[4,4],[4,6],[6,6],[6,4],[4,4]]];
 const waterPayload={mesh:[[0,0,0],[10,0,0],[0,10,0],[10,10,0]],indices:new Uint32Array([0,1,2,1,3,2]),projectionCenter:[0,0,0],coverage:{latitude:0,longitude:0,sizeKm:2},overlays:{layers:{water:{triangles:[],lines:[]}},counts:{water:0},estimatedLakeLevels:0}};
 core.appendWater(waterPayload,[{kind:'water',item:{geometry:{type:'Polygon',coordinates:polygon.map(ring=>ring.map(([x,y])=>[x/scale,y/scale]))},properties:{vt_code:5230}}}],new core.TerrainSurface(waterPayload.mesh,waterPayload.indices));
 function area(vertices){let sum=0;for(let i=0;i<vertices.length;i+=3){const [a,b,c]=vertices.slice(i,i+3);sum+=Math.abs((b[0]-a[0])*(c[1]-a[1])-(c[0]-a[0])*(b[1]-a[1]))/2;}return sum;}
 const water=waterPayload.overlays.layers.water.triangles,waterVertices=Array.from({length:water.length/3},(_,i)=>water.slice(i*3,i*3+3)),landVertices=Array.from(waterPayload.landIndices,i=>waterPayload.mesh[i]);
 assert.ok(Math.abs(area(waterVertices)-60)<1e-6);assert.ok(Math.abs(area(landVertices)-40)<1e-6);assert.equal(waterPayload.overlays.estimatedLakeLevels,1);
 console.log('全 11 レベルの頂点・三角形、混在レベルの接続、海岸形状、標高補間、範囲外・不正データを確認しました。');
})().catch(error=>{console.error(error);process.exitCode=1;});

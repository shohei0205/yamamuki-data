const assert=require('node:assert/strict');const fs=require('node:fs');
const {decodeYMV}=require('./xyz_viewer.js');
(async()=>{let count=0;for(const fixture of JSON.parse(fs.readFileSync(process.argv[2],'utf8'))){for(const target of fixture.targets){
const z=target.key[0],response=await fetch(`http://127.0.0.1:61461/api/bundle?region=${fixture.region}&z=${z}&span=8000`);assert(response.ok);const buffer=await response.arrayBuffer(),length=new DataView(buffer).getUint32(0,true),manifest=JSON.parse(new TextDecoder().decode(new Uint8Array(buffer,4,length))),rows=new Map();let offset=4+length;
for(const [z,x,y,vb,tb]of manifest.entries){rows.set([z,x,y].join(','),{data:new Uint8Array(buffer,offset,vb),tb});offset+=vb+tb}assert.equal(offset,buffer.byteLength);
const displayed=new Set(manifest.rows.map(r=>[z,r[0],r[1]].join(',')));for(const [key,value]of rows)if(!displayed.has(key))assert.equal(value.tb,0);
const cache=new Map();async function resolve(z,x,y){const key=[z,x,y].join(',');if(cache.has(key))return cache.get(key);assert(rows.has(key),'dependency present '+key);const result=decodeYMV(rows.get(key).data,z,x,y,resolve).then(points=>({points}));cache.set(key,result);return result}
assert.deepEqual(Array.from((await resolve(...target.key)).points),target.points.flat());count++}}
console.log(count+' bundled levels decoded with all dependencies; parent triangles omitted');})().catch(e=>{console.error(e);process.exitCode=1});

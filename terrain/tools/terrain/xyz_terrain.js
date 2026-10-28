'use strict';
// 異なるXYZタイルを境界で切り、共有辺を同じ頂点列にしてから結合する。
const XYZGeometry=(()=>{
 const WORLD=40075016.68557849,UNIT=WORLD/268435456,EPS=1e-4;
 const key=p=>Math.round(p[0]*1024)+','+Math.round(p[1]*1024);
 const area=(a,b,c)=>(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0]);
 const bounds=t=>{const s=16384*2**(14-t.z);return [t.x*s,t.y*s,(t.x+1)*s,(t.y+1)*s]};
 function clip(poly,box){if(poly.every(p=>p[0]>=box[0]&&p[0]<=box[2]&&p[1]>=box[1]&&p[1]<=box[3]))return poly;for(const [axis,value,sign] of [[0,box[0],1],[0,box[2],-1],[1,box[1],1],[1,box[3],-1]]){const out=[];for(let i=0;i<poly.length;i++){const a=poly[i],b=poly[(i+1)%poly.length],aa=sign*(a[axis]-value)>=-EPS,bb=sign*(b[axis]-value)>=-EPS;if(aa)out.push(a);if(aa!==bb){const r=(value-a[axis])/(b[axis]-a[axis]),p=a.map((v,k)=>v+(b[k]-v)*r);p[axis]=value;out.push(p)}}poly=out;if(!poly.length)break}const result=[];for(const p of poly)if(!result.length||key(p)!==key(result.at(-1)))result.push(p);if(result.length>1&&key(result[0])===key(result.at(-1)))result.pop();return result}
 function triangulate(poly,records){const ids=[...poly],out=[];if(ids.length===3)return area(...ids.map(i=>records[i].p))<-EPS?[...ids]:[];while(ids.length>3){let found=false;for(let i=0;i<ids.length;i++){const a=ids[(i+ids.length-1)%ids.length],b=ids[i],c=ids[(i+1)%ids.length],aa=records[a].p,bb=records[b].p,cc=records[c].p;if(area(aa,bb,cc)>=-EPS)continue;let blocked=false;for(const d of ids){if(d===a||d===b||d===c)continue;const p=records[d].p;if(area(aa,bb,p)<=EPS&&area(bb,cc,p)<=EPS&&area(cc,aa,p)<=EPS){blocked=true;break}}if(blocked)continue;out.push(a,b,c);ids.splice(i,1);found=true;break}if(!found)throw Error('接合境界の三角形分割に失敗しました')}if(ids.length===3&&area(...ids.map(i=>records[i].p))<-EPS)out.push(...ids);return out}
 function merge(tiles,join=true){
  const mixed=new Set(tiles.map(t=>t.z)).size>1,lines=new Map(),records=[],lookup=new Map(),polys=[];let originalFaces=0,clippedFaces=0,splitFaces=0;
  const lineKey=(axis,value)=>axis+':'+value;
  if(mixed&&join)for(let i=0;i<tiles.length;i++)for(let j=i+1;j<tiles.length;j++){const a=tiles[i],b=tiles[j];if(a.z===b.z)continue;const aa=bounds(a),bb=bounds(b);for(let axis=0;axis<2;axis++){let value=null;if(aa[axis+2]===bb[axis])value=aa[axis+2];else if(bb[axis+2]===aa[axis])value=aa[axis];if(value===null)continue;const other=1-axis,lo=Math.max(aa[other],bb[other]),hi=Math.min(aa[other+2],bb[other+2]);if(hi<=lo)continue;const name=lineKey(axis,value);if(!lines.has(name))lines.set(name,{axis,value,ranges:[],nodes:new Map(),segments:[]});lines.get(name).ranges.push([lo,hi])}}
  function add(p,z){const k=key(p);let id=lookup.get(k);if(id===undefined){id=records.length;lookup.set(k,id);records.push({p:[Math.round(p[0]*1024)/1024,Math.round(p[1]*1024)/1024,p[2]],z})}else if(z>records[id].z){records[id].p[2]=p[2];records[id].z=z}return id}
  for(const tile of tiles){const box=bounds(tile),points=tile.points,faces=tile.faces;
   for(let i=0;i<tile.bodyCount*3;i+=3){originalFaces++;const source=Array.from(faces.subarray(i,i+3),id=>{const at=id*3,h=points[at+2];return [points[at],points[at+1],(h-24000)*(h<24000?.5:.25)]}),poly=mixed?clip(source,box):source;if(poly.length<3)continue;if(poly.length!==3||poly.some((p,j)=>key(p)!==key(source[j])))clippedFaces++;const ids=poly.map(p=>add(p,tile.z));polys.push({ids,z:tile.z});
    if(!lines.size)continue;
    for(let j=0;j<poly.length;j++){const a=poly[j],b=poly[(j+1)%poly.length];for(let axis=0;axis<2;axis++){if(Math.abs(a[axis]-b[axis])>EPS)continue;const value=Math.round(a[axis]);if(Math.abs(a[axis]-value)>EPS)continue;const line=lines.get(lineKey(axis,value));if(!line)continue;const other=1-axis,lo=Math.min(a[other],b[other]),hi=Math.max(a[other],b[other]);if(hi-lo<EPS||!line.ranges.some(([l,h])=>hi>=l-EPS&&lo<=h+EPS))continue;line.segments.push({a:[...a],b:[...b],z:tile.z});line.nodes.set(key(a),ids[j]);line.nodes.set(key(b),ids[(j+1)%ids.length]);for(const range of line.ranges)for(const t of range){if(t<=lo+EPS||t>=hi-EPS)continue;const r=(t-a[other])/(b[other]-a[other]),p=a.map((v,k)=>v+(b[k]-v)*r);p[axis]=value;line.nodes.set(key(p),add(p,tile.z))}}}
   }
  }
  let seamVertices=0;
  for(const line of lines.values()){
   const other=1-line.axis;line.sorted=[...new Set(line.nodes.values())].sort((a,b)=>records[a].p[other]-records[b].p[other]);seamVertices+=line.sorted.length;
   // 詳細側の境界面を基準にする。粗い面から作った切断点も同じ高さへ合わせる。
   for(const s of line.segments.sort((a,b)=>a.z-b.z)){const lo=Math.min(s.a[other],s.b[other]),hi=Math.max(s.a[other],s.b[other]);for(const id of line.sorted){const rec=records[id],v=rec.p[other];if(v<lo-EPS)continue;if(v>hi+EPS)break;if(s.z<rec.z)continue;const r=(v-s.a[other])/(s.b[other]-s.a[other]);rec.p[2]=s.a[2]+(s.b[2]-s.a[2])*r;rec.z=s.z}}
  }
  const indices=[],unique=new Set();
  for(const {ids} of polys){const expanded=[];for(let i=0;i<ids.length;i++){const a=records[ids[i]].p,b=records[ids[(i+1)%ids.length]].p;expanded.push(ids[i]);for(let axis=0;axis<2;axis++){if(Math.abs(a[axis]-b[axis])>EPS)continue;const value=Math.round(a[axis]),line=lines.get(lineKey(axis,value));if(!line||Math.abs(a[axis]-value)>EPS)continue;const other=1-axis,lo=Math.min(a[other],b[other]),hi=Math.max(a[other],b[other]),middle=[];for(const id of line.sorted){const v=records[id].p[other];if(v<=lo+EPS)continue;if(v>=hi-EPS)break;middle.push(id)}if(b[other]<a[other])middle.reverse();expanded.push(...middle)}}if(expanded.length>ids.length)splitFaces++;const ff=triangulate(expanded,records);for(let i=0;i<ff.length;i+=3){const face=ff.slice(i,i+3),k=[...face].sort((a,b)=>a-b).join(',');if(unique.has(k))continue;unique.add(k);indices.push(...face)}}
  const used=new Uint8Array(records.length);for(const i of indices)used[i]=1;const remap=new Int32Array(records.length),points=[];for(let i=0;i<records.length;i++)if(used[i]){remap[i]=points.length/3;points.push(...records[i].p)}
  return {points:Float64Array.from(points),indices:Uint32Array.from(indices,i=>remap[i]),stats:{mixed,joined:!!(join&&mixed),mixedSegments:[...lines.values()].reduce((s,l)=>s+l.ranges.length,0),seamVertices,originalFaces,clippedFaces,splitFaces,triangles:indices.length/3}};
 }
 function normals(points,indices){const out=new Float32Array(points.length);for(let i=0;i<indices.length;i+=3){const a=indices[i]*3,b=indices[i+1]*3,c=indices[i+2]*3,ux=points[b]-points[a],uy=points[b+1]-points[a+1],uz=points[b+2]-points[a+2],vx=points[c]-points[a],vy=points[c+1]-points[a+1],vz=points[c+2]-points[a+2],n=[uy*vz-uz*vy,uz*vx-ux*vz,ux*vy-uy*vx];for(const at of [a,b,c])for(let k=0;k<3;k++)out[at+k]+=n[k]}for(let i=0;i<out.length;i+=3){const n=Math.hypot(out[i],out[i+1],out[i+2])||1;out[i]/=n;out[i+1]/=n;out[i+2]/=n}return out}
 return {merge,clip,triangulate,normals,UNIT};
})();
if(typeof module!=='undefined')module.exports=XYZGeometry;

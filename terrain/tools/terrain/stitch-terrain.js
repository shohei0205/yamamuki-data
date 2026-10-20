/* タイル境界の制約辺を局所的に張り替え、隣接タイルをまたぐ三角形を作る。 */
'use strict';
function stitchTerrain(meshes,width,targetX,targetY,tolerance){
 const points=[],slopes=[],lookup=new Map(),triangles=[],edges=new Map();
 const globalKey=p=>`${Math.round((p[0]+targetX*width)*128)},${Math.round((p[1]+targetY*width)*128)},${Math.round(p[2]*4)}`;
 const pointKey=p=>`${Math.round(p[0]*128)},${Math.round(p[1]*128)},${Math.round(p[2]*4)}`;
 const edgeKey=(a,b)=>a<b?`${a}/${b}`:`${b}/${a}`;
 const orient=(a,b,c)=>(points[b][0]-points[a][0])*(points[c][1]-points[a][1])-(points[b][1]-points[a][1])*(points[c][0]-points[a][0]);
 function addEdges(id){const f=triangles[id];for(let j=0;j<3;j++){const k=edgeKey(f[j],f[(j+1)%3]);let s=edges.get(k);if(!s){s=new Set();edges.set(k,s);}s.add(id);}}
 function removeEdges(id){const f=triangles[id];for(let j=0;j<3;j++){const k=edgeKey(f[j],f[(j+1)%3]),s=edges.get(k);s.delete(id);if(!s.size)edges.delete(k);}}
 for(const mesh of meshes){const remap=[];for(let i=0;i<mesh.points.length;i+=3){const p=[mesh.points[i]+(mesh.x-targetX)*width,mesh.points[i+1]+(mesh.y-targetY)*width,mesh.points[i+2]],k=pointKey(p);let id=lookup.get(k);if(id===undefined){id=points.length;lookup.set(k,id);points.push(p);slopes.push(mesh.slopes?[mesh.slopes[i/3*2],mesh.slopes[i/3*2+1]]:[0,0]);}remap.push(id);}for(let i=0;i<mesh.indices.length;i+=3){const f=[remap[mesh.indices[i]],remap[mesh.indices[i+1]],remap[mesh.indices[i+2]]];if(orient(...f)<0)[f[1],f[2]]=[f[2],f[1]];triangles.push(f);addEdges(triangles.length-1);}}
 const isLine=(a,b)=>[0,1].some(axis=>Math.abs(points[a][axis]-points[b][axis])<.001&&Math.abs(points[a][axis]/width-Math.round(points[a][axis]/width))<1e-7);
 const candidates=[...edges].filter(([k,s])=>{const [a,b]=k.split('/').map(Number);return s.size===2&&isLine(a,b);}).map(([k])=>{const [a,b]=k.split('/').map(Number);return {k,sort:[globalKey(points[a]),globalKey(points[b])].sort().join('|')};}).sort((a,b)=>a.sort.localeCompare(b.sort,'en'));
 function normal(f){const a=points[f[0]],b=points[f[1]],c=points[f[2]],u=b.map((v,i)=>v-a[i]),v=c.map((w,i)=>w-a[i]),n=[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]],l=Math.hypot(...n);return n.map(v=>v/l);}
 const similarity=(f,g)=>{const a=normal(f),b=normal(g);return a.reduce((s,v,i)=>s+v*b[i],0);};
 let flips=0,maxDeviation=0;
 for(const candidate of candidates){const set=edges.get(candidate.k);if(!set||set.size!==2)continue;const ids=[...set],[a,b]=candidate.k.split('/').map(Number),c=triangles[ids[0]].find(i=>i!==a&&i!==b),d=triangles[ids[1]].find(i=>i!==a&&i!==b);if(c===d||orient(c,d,a)*orient(c,d,b)>=-1e-5||edges.has(edgeKey(c,d)))continue;
 const ab=[points[b][0]-points[a][0],points[b][1]-points[a][1]],cd=[points[d][0]-points[c][0],points[d][1]-points[c][1]],ca=[points[c][0]-points[a][0],points[c][1]-points[a][1]],cross=(u,v)=>u[0]*v[1]-u[1]*v[0],den=cross(ab,cd),t=cross(ca,cd)/den,u=cross(ca,ab)/den,deviation=Math.abs(points[a][2]+t*(points[b][2]-points[a][2])-points[c][2]-u*(points[d][2]-points[c][2]));if(!Number.isFinite(deviation)||deviation>tolerance)continue;
 const next=[[c,d,a],[d,c,b]];for(const f of next)if(orient(...f)<0)[f[1],f[2]]=[f[2],f[1]];
 if(similarity(...next)<=similarity(triangles[ids[0]],triangles[ids[1]])+1e-6)continue;
 for(const id of ids)removeEdges(id);ids.forEach((id,j)=>{triangles[id]=next[j];addEdges(id);});flips++;maxDeviation=Math.max(maxDeviation,deviation);
 }
 const owned=[],used=new Map(),out=[],hints=[];
 for(const f of triangles){const x=f.reduce((s,i)=>s+points[i][0],0)/3,y=f.reduce((s,i)=>s+points[i][1],0)/3;if(Math.floor(x/width)!==0||Math.floor(y/width)!==0)continue;for(const id of f){let mapped=used.get(id);if(mapped===undefined){mapped=used.size;used.set(id,mapped);out.push(...points[id]);hints.push(...slopes[id]);}owned.push(mapped);}}
 // 所有区画をまたぐ共有頂点も、陰影の接合対象として返す。
 const shared=new Set();for(const f of triangles)for(const id of f)if(used.has(id)){const x=f.reduce((s,j)=>s+points[j][0],0)/3,y=f.reduce((s,j)=>s+points[j][1],0)/3;if(Math.floor(x/width)!==0||Math.floor(y/width)!==0)shared.add(used.get(id));}
 return {points:new Float32Array(out),indices:new Uint32Array(owned),slopes:new Float32Array(hints),sharedVertexIds:new Uint32Array([...shared]),flips,maxDeviation};
}
if(typeof module!=='undefined')module.exports={stitchTerrain};

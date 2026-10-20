/* 曲面の曲がりが強い辺を二分する。共有辺は両側同時に分割する。 */
'use strict';
function refineCurvePattern(data,divisions,{rounds=2,maxTriangles=500000,angleDegrees=12,heightError=.5,width=null}={}){
 const vertices=[],faces=[],maps=[],faceCount=data.length/112;
 const sample=(patch,u,v)=>{const b=patch*112,c=data,w=1-u-v,k=b+(v<=u&&v<=w?0:w<=u&&w<=v?1:2)*10;
  const z=c[k]+c[k+1]*u+c[k+2]*v+c[k+3]*u*u+c[k+4]*u*v+c[k+5]*v*v+c[k+6]*u*u*u+c[k+7]*u*u*v+c[k+8]*u*v*v+c[k+9]*v*v*v;
  const du=c[k+1]+2*c[k+3]*u+c[k+4]*v+3*c[k+6]*u*u+2*c[k+7]*u*v+c[k+8]*v*v,dv=c[k+2]+c[k+4]*u+2*c[k+5]*v+c[k+7]*u*u+2*c[k+8]*u*v+3*c[k+9]*v*v;
  const gx=du*c[b+32]+dv*c[b+33],gy=du*c[b+34]+dv*c[b+35],l=Math.hypot(gx,gy,1);
  return {x:c[b+30]*w+c[b+46]*u+c[b+79]*v,y:c[b+31]*w+c[b+47]*u+c[b+110]*v,z:c[b+39]+z,n:[-gx/l,-gy/l,1/l]};};
 const vertex=(patch,u,v)=>{const key=u.toFixed(12)+'/'+v.toFixed(12),map=maps[patch];if(map.has(key))return map.get(key);const id=vertices.length;vertices.push({patch,u,v,...sample(patch,u,v)});map.set(key,id);return id;};
 for(let p=0;p<faceCount;p++){maps.push(new Map());const at=(i,j)=>vertex(p,i/divisions,j/divisions);for(let i=0;i<divisions;i++)for(let j=0;j<divisions-i;j++){faces.push([at(i,j),at(i+1,j),at(i,j+1)]);if(i+j<divisions-1)faces.push([at(i+1,j),at(i+1,j+1),at(i,j+1)]);}}
 let triangles=faces,completed=0,budgetLimited=false;
 const positionKey=v=>Math.round(v.x*10000)+'/'+Math.round(v.y*10000);
 const edgeKey=(a,b)=>{const x=positionKey(vertices[a]),y=positionKey(vertices[b]);return x<y?x+'|'+y:y+'|'+x;};
 const boundary=(a,b)=>{if(width===null)return false;const x=vertices[a],y=vertices[b];return [0,width].some(v=>Math.abs(x.x-v)<.01&&Math.abs(y.x-v)<.01||Math.abs(x.y-v)<.01&&Math.abs(y.y-v)<.01);};
 const cos=Math.cos(angleDegrees*Math.PI/180);
 for(let round=0;round<rounds;round++){
  const marked=new Set();
  for(const ids of triangles)for(let e=0;e<3;e++){const a=vertices[ids[e]],b=vertices[ids[(e+1)%3]],mid=sample(a.patch,(a.u+b.u)/2,(a.v+b.v)/2);if(boundary(ids[e],ids[(e+1)%3])||a.n.reduce((s,v,k)=>s+v*b.n[k],0)<cos||Math.abs(mid.z-(a.z+b.z)/2)>heightError)marked.add(edgeKey(ids[e],ids[(e+1)%3]));}
  if(!marked.size)break;
  // 各辺の分割で面数が1増える。先に予算を調べ、不完全な共有辺分割を作らない。
  let estimate=triangles.length;for(const ids of triangles)for(let e=0;e<3;e++)if(marked.has(edgeKey(ids[e],ids[(e+1)%3])))estimate++;
  if(estimate>maxTriangles){budgetLimited=true;for(const k of [...marked])marked.delete(k);for(const ids of triangles)for(let e=0;e<3;e++)if(boundary(ids[e],ids[(e+1)%3]))marked.add(edgeKey(ids[e],ids[(e+1)%3]));if(!marked.size)break;}
  const next=[];
  for(const ids of triangles){let a=ids[0],b=ids[1],c=ids[2],flags=[0,1,2].map(e=>marked.has(edgeKey(ids[e],ids[(e+1)%3]))),count=flags.filter(Boolean).length;
   if(!count){next.push(ids);continue;}
   // 1辺ならAB、2辺ならAB・BCが分割されるよう並べ替える。
   if(count<3){let rotation=0;while(count===1?!flags[rotation]:!(flags[rotation]&&flags[(rotation+1)%3]))rotation++;a=ids[rotation];b=ids[(rotation+1)%3];c=ids[(rotation+2)%3];}
   const midpoint=(i,j)=>{const x=vertices[i],y=vertices[j];return vertex(x.patch,(x.u+y.u)/2,(x.v+y.v)/2);},ab=midpoint(a,b);
   if(count===1)next.push([a,ab,c],[ab,b,c]);
   else {const bc=midpoint(b,c);if(count===2){
     const score=faces=>Math.max(...faces.map(ids=>{const [a,b,c]=ids.map(i=>vertices[i]),x=[b.x-a.x,b.y-a.y,b.z-a.z],y=[c.x-a.x,c.y-a.y,c.z-a.z],n=[x[1]*y[2]-x[2]*y[1],x[2]*y[0]-x[0]*y[2],x[0]*y[1]-x[1]*y[0]],len=Math.hypot(...n),g=sample(a.patch,(a.u+b.u+c.u)/3,(a.v+b.v+c.v)/3).n;return 1-n.reduce((sum,v,k)=>sum+v*g[k],0)/len;}));
     const first=[[a,ab,c],[ab,bc,c]],second=[[a,ab,bc],[a,bc,c]];next.push([ab,b,bc],...(score(first)<=score(second)?first:second));
    }else {const ca=midpoint(c,a);next.push([a,ab,ca],[ab,b,bc],[ca,bc,c],[ab,bc,ca]);}}
  }
  triangles=next;completed++;
 }
 const uv=new Float32Array(vertices.length*2),ids=new Float32Array(vertices.length);for(let i=0;i<vertices.length;i++){uv.set([vertices[i].u,vertices[i].v],i*2);ids[i]=vertices[i].patch;}
 return {uv,ids,indices:Uint32Array.from(triangles.flat()),rounds:completed,budgetLimited,baseTriangles:faces.length};
}
if(typeof module!=='undefined')module.exports={refineCurvePattern};

/* 粗いタイルの辺へ細かいタイルの境界頂点を合わせる。 */
'use strict';
const HeightLODEdges={
 create(items,parameters,sample){
  const map=new Map(),base=parameters.base,target=parameters.target,epsilon=1e-11;
  for(const item of items)map.set(item.displayRow.slice(0,3).join('/'),item);
  const at=(u,v)=>{for(let z=target;z>=base;z--){const item=map.get(`${z}/${Math.floor(u*2**z)}/${Math.floor(v*2**z)}`);if(item)return item;}return null;};
  const around=(u,v)=>[at(u-epsilon,v-epsilon),at(u+epsilon,v-epsilon),at(u-epsilon,v+epsilon),at(u+epsilon,v+epsilon)].filter(Boolean);
  const memo=new Map();
  const point=(u,v,level)=>{
   for(const other of around(u,v))level=Math.min(level,other.row[0]);
   const key=`${u},${v},${level}`;if(memo.has(key))return memo.get(key);
   const value=sample(u,v,level),step=1/(2**level*256),meters=TerrainLOD.world*step;
   const gradient=[(sample(u+step,v,level)[0]-sample(u-step,v,level)[0])/(2*meters),(sample(u,v-step,level)[0]-sample(u,v+step,level)[0])/(2*meters)];
   const result=[...value,...gradient];memo.set(key,result);return result;
  };
  return (item,n,edges,gradients)=>{
   const [z,x,y]=item.row,[ru,rv,rw,rh]=item.range,left=(x+ru)/2**z,top=(y+rv)/2**z,span=rw/2**z;
   for(let edge=0;edge<4;edge++)for(let i=0;i<=n;i++){
    const t=i/n,u=left+(edge===1?span:edge===3?0:span*t),v=top+(edge===2?span:edge===0?0:span*t);
    let level=z,step=span/n;
    for(const other of around(u,v)){level=Math.min(level,other.row[0]);step=Math.max(step,other.range[2]/(2**other.row[0]*other.lodN));}
    const along=edge===0||edge===2?u:v,a=Math.floor(along/step+1e-7)*step,b=a+step,f=Math.max(0,Math.min(1,(along-a)/step));
    const first=point(edge===0||edge===2?a:u,edge===0||edge===2?v:a,level),second=point(edge===0||edge===2?b:u,edge===0||edge===2?v:b,level),offset=(edge*(n+1)+i)*2;
    edges[offset]=first[0]*(1-f)+second[0]*f;edges[offset+1]=first[1]*(1-f)+second[1]*f;
    gradients[offset]=first[2]*(1-f)+second[2]*f;gradients[offset+1]=first[3]*(1-f)+second[3]*f;
   }
  };
 }
};
if(typeof module!=='undefined')module.exports=HeightLODEdges;

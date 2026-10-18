/* 標高の画素中心を基準に、タイル境界・角の高さを共通化する。 */
'use strict';
const HeightBoundaries={
 sample(u,v,level,cache,index){
  let partial;
  for(let z=level;z>=7;z--){const size=2**z*256,px=u*size-.5,py=v*size-.5,x=Math.floor(px),y=Math.floor(py),fx=px-x,fy=py-y;let height=0,valid=0,weight=0,ready=true;
   for(const [dx,dy,w]of [[0,0,(1-fx)*(1-fy)],[1,0,fx*(1-fy)],[0,1,(1-fx)*fy],[1,1,fx*fy]]){if(w===0)continue;const sx=x+dx,sy=y+dy,tx=Math.floor(sx/256),ty=Math.floor(sy/256),key=`${z}/${tx}/${ty}`,tile=cache.get(key);if(!tile){if(index.has(key)){ready=false;}continue;}const i=(sy-ty*256)*256+sx-tx*256;height+=tile.heights[i]*w;valid+=tile.validValues[i]/255*w;weight+=w;}
   if(ready)return weight>0?[height,valid]:(partial||[0,0]);
   if(!partial&&weight>0)partial=[height/weight,valid/weight];
  }
  return partial||[0,0];
 },
 edges(row,range,n,level,cache,index,memo=new Map()){
  const [z,x,y]=row,scale=2**z,out=new Float32Array((n+1)*4*2);
  for(let edge=0;edge<4;edge++)for(let i=0;i<=n;i++){const t=i/n,uv=edge===0?[t,0]:edge===1?[1,t]:edge===2?[t,1]:[0,t],u=(x+range[0]+uv[0]*range[2])/scale,v=(y+range[1]+uv[1]*range[3])/scale,key=u+','+v;let value=memo.get(key);if(!value){value=this.sample(u,v,level,cache,index);memo.set(key,value);}out.set(value,(edge*(n+1)+i)*2);}
  return out;
 }
};
if(typeof module!=='undefined')module.exports=HeightBoundaries;

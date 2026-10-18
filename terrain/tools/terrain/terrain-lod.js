/* 距離に応じた詳細度と、画面全体のタイル数を制限する。 */
'use strict';
const TerrainLOD={
 world:40075016.68557849,
 tileBudget:64,
 requestBudget:96,
 heightPixelBudget:64*256*256,
 meshPointBudget:131072,
 bounds(row){const [z,x,y]=row,s=1/2**z;return [x*s,y*s,s,s];},
 distance(row,p){
  const [u,v,s]=this.bounds(row),x0=u*this.world-p.center[0],x1=x0+s*this.world,y0=p.center[1]-(v+s)*this.world,y1=y0+s*this.world;
  return Math.hypot(Math.max(x0,0,-x1),Math.max(y0,0,-y1));
 },
 limit(row,p){
  if(p.distanceLOD===false)return p.target;
  if(p.ground){const distance=Math.max(40,this.distance(row,p)),f=p.height/(2*Math.tan(p.fov/2));return Math.max(p.base,Math.min(p.target,Math.ceil(Math.log2(this.world*f/(distance*256*2)))));}
  const [u,v,s]=this.bounds(row),c=Math.cos(p.bearing),b=Math.sin(p.bearing);
  const forward=Math.min(...[0,1].flatMap(dx=>[0,1].map(dy=>-b*((u+dx*s)*this.world-p.center[0])+c*(p.center[1]-(v+dy*s)*this.world))));
  const reduction=Math.floor(Math.log2(1+Math.max(0,forward)*Math.sin(p.tilt)/p.scale));
  return Math.max(p.base,p.target-reduction);
 },
 mesh(item,p){
  const [z]=item.row,span=this.world/2**z*item.range[2];
  const row=item.displayRow||item.row,distance=Math.max(40,this.distance(row,p));
  const pixels=p.ground?span*p.height/(2*Math.tan(p.fov/2)*distance):span*p.height/(2*p.scale);
  return Math.max(16,Math.min(128,2**Math.ceil(Math.log2(Math.max(1,pixels/5)))));
 },
 fitMeshes(items,p,budget=this.meshPointBudget){
  const cost=item=>(item.lodN+1)**2;
  let points=items.reduce((n,item)=>n+cost(item),0);
  const ordered=[...items].sort((a,b)=>this.distance(b.displayRow||b.row,p)-this.distance(a.displayRow||a.row,p));
  for(const item of ordered)while(points>budget&&item.lodN>16){const previous=cost(item);item.lodN/=2;points+=cost(item)-previous;}
  return points;
 },
 choose(roots,index,visible,p,budget=this.tileBudget,requestBudget=this.requestBudget){
  const leaves=new Set(),queue=[];let nodes=0;
  const add=node=>{nodes++;leaves.add(node);if(node.row[0]<this.limit(node.row,p))queue.push(node);};
  const initial=roots.filter(visible).sort((a,b)=>this.distance(a,p)-this.distance(b,p));for(const row of initial.slice(0,budget))add({row,parent:null});
  while(queue.length){
   queue.sort((a,b)=>(this.limit(b.row,p)-b.row[0])-(this.limit(a.row,p)-a.row[0])||this.distance(a.row,p)-this.distance(b.row,p));
   const node=queue.shift(),[z,x,y]=node.row,children=[];
   for(let dy=0;dy<2;dy++)for(let dx=0;dx<2;dx++){const row=index.get(`${z+1}/${x*2+dx}/${y*2+dy}`);if(row&&visible(row))children.push({row,parent:node});}
   if(!children.length||leaves.size+children.length-1>budget||nodes+children.length>requestBudget)continue;
   leaves.delete(node);children.forEach(add);
  }
  const requests=new Map();
  for(const leaf of leaves)for(let node=leaf;node;node=node.parent)requests.set(node.row.slice(0,3).join('/'),node.row);
  return {coverageLimited:initial.length>budget,leaves:[...leaves],requests:[...requests.values()].sort((a,b)=>a[0]-b[0]||this.distance(a,p)-this.distance(b,p))};
 }
};
if(typeof module!=='undefined')module.exports=TerrainLOD;

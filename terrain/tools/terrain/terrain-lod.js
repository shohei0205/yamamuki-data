/* 距離に応じた詳細度と、画面全体のタイル数を制限する。 */
'use strict';
const TerrainLOD={
 world:40075016.68557849,
 tileBudget:64,
 requestBudget:96,
 heightPixelBudget:64*256*256,
 meshPointBudget:131072,
 topMeshPointBudget:300000,
 topView(p){return !p.ground&&Math.abs(p.tilt)<.001;},
 pointBudget(p){return this.topView(p)?this.topMeshPointBudget:this.meshPointBudget;},
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
 focusWeight(row,p){
  if(p.ground||this.topView(p))return 1;
  const r=this.distance(row,p)/Math.max(1,p.scale*.35);
  return 1+3*Math.exp(-r*r);
 },
 projectedPixels(item,p){
  const [z]=item.row,span=this.world/2**z*item.range[2];
  const row=item.displayRow||item.row,distance=Math.max(40,this.distance(row,p));
  return p.ground?span*p.height/(2*Math.tan(p.fov/2)*distance):span*p.height/(2*p.scale);
 },
 mesh(item,p){
  if(p.distanceLOD===false)return 128;
  const row=item.displayRow||item.row;
  if(!p.ground&&!this.topView(p)&&this.distance(row,p)===0)return 128;
  const pixels=this.projectedPixels(item,p)*this.focusWeight(row,p);
  return Math.max(16,Math.min(128,2**Math.ceil(Math.log2(Math.max(1,pixels/(this.topView(p)?2:5))))));
 },
 fitMeshes(items,p,budget=this.pointBudget(p)){
  const cost=item=>(item.lodN+1)**2;
  if(p.distanceLOD===false)return items.reduce((n,item)=>n+cost(item),0);
  if(this.topView(p)&&items.length){let n=Math.max(...items.map(item=>item.lodN));while(n>16&&items.length*(n+1)**2>budget)n/=2;for(const item of items)item.lodN=n;return items.length*(n+1)**2;}
  let points=items.reduce((n,item)=>n+cost(item),0);
  // 一つの遠方区画を最小まで削らず、画面上の密度が高い区画から一段ずつ減らす。
  const projected=new Map(items.map(item=>[item,Math.max(1,this.projectedPixels(item,p)*this.focusWeight(item.displayRow||item.row,p))]));
  while(points>budget){
   let candidate=null,highest=-Infinity;
   for(const item of items){if(item.lodN<=16)continue;const density=item.lodN/projected.get(item);
    if(density>highest){candidate=item;highest=density;}
   }
   if(!candidate)break;
   const previous=cost(candidate);candidate.lodN/=2;points+=cost(candidate)-previous;
  }
  return points;
 },
 choose(roots,index,visible,p,budget=this.tileBudget,requestBudget=this.requestBudget){
  if(p.distanceLOD===false){budget=Infinity;requestBudget=Infinity;}
  const leaves=new Set(),queue=[];let nodes=0;
  const add=node=>{nodes++;leaves.add(node);if(node.row[0]<this.limit(node.row,p))queue.push(node);};
  const initial=roots.filter(visible).sort((a,b)=>this.distance(a,p)-this.distance(b,p));for(const row of initial.slice(0,budget))add({row,parent:null});
  if(this.topView(p)){
   let current=[...leaves],count=current.length;
   for(let z=p.base;z<p.target;z++){
    const next=[];
    for(const node of current){const [level,x,y]=node.row;if(level!==z){next.push(node);continue;}let found=false;
     for(let dy=0;dy<2;dy++)for(let dx=0;dx<2;dx++){const row=index.get(`${z+1}/${x*2+dx}/${y*2+dy}`);if(row&&visible(row)){next.push({row,parent:node});found=true;}}
     if(!found)next.push(node);
    }
    const added=next.filter(n=>!current.includes(n)).length;
    if(next.length>budget||count+added>requestBudget)break;
    current=next;count+=added;
   }
   leaves.clear();current.forEach(n=>leaves.add(n));queue.length=0;
  }
  while(queue.length){
   queue.sort((a,b)=>(this.distance(a.row,p)===0?-1:0)-(this.distance(b.row,p)===0?-1:0)||(this.limit(b.row,p)-b.row[0])-(this.limit(a.row,p)-a.row[0])||this.distance(a.row,p)-this.distance(b.row,p));
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

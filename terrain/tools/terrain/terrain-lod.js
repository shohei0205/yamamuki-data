/* 距離に応じた詳細度と、画面全体のタイル数を制限する。 */
'use strict';
const TerrainLOD={
 world:40075016.68557849,
 tileBudget:16,
 coverageTileBudget:64,
 meshDivisions:256,
 requestBudget:96,
 heightPixelBudget:16*256*256,
 topView(p){return !p.ground&&Math.abs(p.tilt)<.001;},
 pointBudget(p){return (p.tileBudget??this.tileBudget)*(this.meshDivisions+1)**2;},
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
 overviewVisible(row,p){
  const [z,x,y,min=0,step=0]=row,span=this.world/2**z;
  const c=Math.cos(p.bearing),b=Math.sin(p.bearing),ct=Math.cos(p.tilt),st=Math.sin(p.tilt);
  const low=Math.min(0,min)*p.exaggeration,high=Math.max(0,min+65535*step)*p.exaggeration;
  let left=Infinity,right=-Infinity,bottom=Infinity,top=-Infinity;
  // 標高ゼロの四隅だけでは、手前の山が画面内へせり上がる範囲を落としてしまう。
  for(const dx of [0,1])for(const dy of [0,1]){
   const px=(x+dx)*span-p.center[0],py=p.center[1]-(y+dy)*span;
   const rx=c*px+b*py,ry=(-b*px+c*py)*ct;
   left=Math.min(left,rx);right=Math.max(right,rx);
   bottom=Math.min(bottom,ry+low*st);top=Math.max(top,ry+high*st);
  }
  return left<p.scale*p.aspect&&right>-p.scale*p.aspect&&bottom<p.scale&&top>-p.scale;
 },
 // 分割数は表示条件で減らさず、詳細度は取得するタイルのレベルだけで変える。
 mesh(){return this.meshDivisions;},
 choose(roots,index,visible,p,budget=this.tileBudget,requestBudget=this.requestBudget){
  const leaves=new Set(),queue=[];let nodes=0;
  const add=node=>{nodes++;leaves.add(node);if(node.row[0]<this.limit(node.row,p))queue.push(node);};
  const initial=roots.filter(visible).sort((a,b)=>this.distance(a,p)-this.distance(b,p));
  if(p.distanceLOD===false){budget=Infinity;requestBudget=Infinity;}
  else{
   // 最低レベルより粗いデータがない場合、既存の表示範囲を保つ最低限の区画を優先する。
   budget=Math.max(budget,Math.min(initial.length,this.coverageTileBudget));
  }
  for(const row of initial.slice(0,budget))add({row,parent:null});
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
  return {tileBudget:budget,coverageLimited:initial.length>budget,leaves:[...leaves],requests:[...requests.values()].sort((a,b)=>a[0]-b[0]||this.distance(a,p)-this.distance(b,p))};
 }
};
if(typeof module!=='undefined')module.exports=TerrainLOD;

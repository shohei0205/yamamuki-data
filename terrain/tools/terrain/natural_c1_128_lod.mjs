// 可視の矩形断面を視錐台で切り、画面内で最も近い深度を求める。
// タイル中心や画面外の手前の隅を代表点にしない。
export function visibleDepth(bounds,project){
 let nearest=Infinity;
 const planes=[p=>p.z-10,p=>p.x+1.15*p.z,p=>1.15*p.z-p.x,p=>p.y+1.15*p.z,p=>1.15*p.z-p.y];
 for(const height of [0,1500,4000]){
 let polygon=[[bounds[0],bounds[1]],[bounds[2],bounds[1]],[bounds[2],bounds[3]],[bounds[0],bounds[3]]].map(([x,y])=>{const p=project(x,y,height);return {x:p.x*p.z,y:p.y*p.z,z:p.z};});
 for(const plane of planes){const next=[];for(let i=0;i<polygon.length;i++){const a=polygon[i],b=polygon[(i+1)%polygon.length],da=plane(a),db=plane(b);if(da>=0)next.push(a);if((da>=0)!==(db>=0)){const t=da/(da-db);next.push({x:a.x+(b.x-a.x)*t,y:a.y+(b.y-a.y)*t,z:a.z+(b.z-a.z)*t});}}polygon=next;if(!polygon.length)break;}
 for(const p of polygon)nearest=Math.min(nearest,p.z);
 }
 if(!Number.isFinite(nearest)){nearest=Infinity;for(const x of [bounds[0],bounds[2]])for(const y of [bounds[1],bounds[3]])for(const h of [0,4000])nearest=Math.min(nearest,project(x,y,h).z);}
 return Math.max(200,nearest);
}

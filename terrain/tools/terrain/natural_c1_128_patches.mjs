// GPUパケットの既存制御点から、32×32区画の局所的な間隔を求める。
export function controlDensity(packet,offset,info){
 if(!info.pointBytes||!info.side||!info.origin)return null;
 const grid=new Uint32Array(1024),points=new Float32Array(packet,offset,info.pointBytes/4),side=info.side,origin=info.origin;
 for(let i=0;i<points.length;i+=8){const x=(points[i]*1000+side/2)/side,y=(points[i+1]*1000+side/2)/side;if(x>=0&&x<=1&&y>=0&&y<=1)grid[Math.min(31,Math.floor(y*32))*32+Math.min(31,Math.floor(x*32))]++;}
 return {grid,side,bounds:[origin[0]-side/2,origin[1]-side/2,origin[0]+side/2,origin[1]+side/2]};
}
function controlStep(d,b){if(!d)return Infinity;const cell=d.side/32;let step=Infinity;const loX=Math.max(0,Math.floor((b[0]-d.bounds[0])/cell)),hiX=Math.min(31,Math.ceil((b[2]-d.bounds[0])/cell)-1),loY=Math.max(0,Math.floor((b[1]-d.bounds[1])/cell)),hiY=Math.min(31,Math.ceil((b[3]-d.bounds[1])/cell)-1);for(let y=loY;y<=hiY;y++)for(let x=loX;x<=hiX;x++){const count=d.grid[y*32+x];if(count)step=Math.min(step,cell/Math.sqrt(count));}return step;}
// 描画パッチは制御点・GPU曲面キャッシュを共有する。
export function makePatches(tiles, project, width, limit=2000000, visible=()=>true){
 const out=[];
 tiles.forEach((t,tile)=>{const b=t.bounds,k=Math.min(t.n,Math.max(4,Math.min(32,2**Math.ceil(Math.log2((b[2]-b[0])/2000))))),side=(b[2]-b[0])/k,max=Math.max(1,t.n/Math.min(k,8));
 for(let y=0;y<k;y++)for(let x=0;x<k;x++){
 const bounds=[b[0]+x*side,b[1]+y*side,b[0]+(x+1)*side,b[1]+(y+1)*side];
 // 中央と四隅のうち最も近い点を使い、傾斜視点の手前を確保する。
 let z=Infinity;for(const [u,v] of [[.5,.5],[0,0],[0,1],[1,0],[1,1]])z=Math.min(z,Math.max(200,project(bounds[0]+u*side,bounds[1]+v*side).z));
 if(!visible({bounds}))continue;
 const pixels=side*width*.8660254/z,required=Math.max(1,2**Math.ceil(Math.log2(side/controlStep(t.controlDensity,bounds)))),minN=Math.min(max,required),n=Math.max(minN,Math.min(max,2**Math.ceil(Math.log2(Math.max(1,pixels/6)))));
 out.push({bounds,tile,n,minN,densityLimited:required>max,side,pixels});
 }});
 let total=out.reduce((s,p)=>s+2*p.n*p.n,0);
 // 毎回全体をソートせず、変更したパッチだけ優先度を更新する。
 const heap=[],rank=new Map(out.map((p,i)=>[p,i]));
 const less=(a,b)=>a.pixels/a.n<b.pixels/b.n||(a.pixels/a.n===b.pixels/b.n&&rank.get(a)<rank.get(b));
 const push=p=>{let i=heap.length;heap.push(p);while(i>0){const parent=(i-1)>>1;if(!less(p,heap[parent]))break;heap[i]=heap[parent];i=parent;}heap[i]=p;};
 const pop=()=>{const first=heap[0],last=heap.pop();if(heap.length){let i=0;while(i*2+1<heap.length){let child=i*2+1;if(child+1<heap.length&&less(heap[child+1],heap[child]))child++;if(!less(heap[child],last))break;heap[i]=heap[child];i=child;}heap[i]=last;}return first;};
 if(total>limit){for(const p of out)if(p.n>p.minN)push(p);while(total>limit&&heap.length){const p=pop();total-=1.5*p.n*p.n;p.n/=2;if(p.n>p.minN)push(p);}}

 // 同一直線上の反対向き辺を照合。部分接触と異なるタイルサイズにも対応。
 const lines=new Map();
 for(const p of out){p.edges=Array(4).fill(p.side/p.n);for(let e=0;e<4;e++){const vertical=e<2,b=p.bounds,key=(vertical?'x':'y')+b[e===0?0:e===1?2:e===2?1:3];if(!lines.has(key))lines.set(key,[]);lines.get(key).push({p,e,a:b[vertical?1:0],b:b[vertical?3:2]});}}
 for(const edges of lines.values()){
 for(const e of edges){e.parent=e;e.step=e.p.side/e.p.n;}
 const root=e=>{while(e.parent!==e){e.parent=e.parent.parent;e=e.parent;}return e;};
 edges.sort((a,b)=>a.a-b.a);for(let i=0;i<edges.length;i++)for(let j=i+1;j<edges.length&&edges[j].a<edges[i].b-1e-6;j++){const a=edges[i],b=edges[j];if((a.e^1)!==b.e||Math.min(a.b,b.b)<=Math.max(a.a,b.a)+1e-6)continue;const ra=root(a),rb=root(b);if(ra!==rb){rb.parent=ra;ra.step=Math.max(ra.step,rb.step);}}
 for(const e of edges)e.p.edges[e.e]=root(e).step;
 }

 return out;
}

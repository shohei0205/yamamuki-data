// 可視区画・接合成分・描画密度を並列計算し、従来と同じ優先順位でLODを決める。
const MAX_ITEMS=16384u;const HEADS=65536u;
struct Source {grid:vec4<u32>,coverage:vec4<u32>}
struct Item {grid:vec4<u32>,plan:vec4<u32>,edges:vec4<f32>,origins:vec4<f32>}
struct Edge {geometry:vec4<u32>,tree:vec4<u32>,stats:vec4<f32>,extra:vec4<u32>}
struct Patch {bounds:vec4<f32>,edges:vec4<f32>,origins:vec4<f32>,lines:vec4<f32>}
struct Workspace {items:array<Item,16384>,temporary:array<Item,16384>,edges:array<Edge,65536>,counts:array<u32,40960>,offsets:array<u32,40960>}
struct Params {grid:vec4<u32>,fraction:vec4<f32>,earth:vec4<f32>,right:vec4<f32>,up:vec4<f32>,back:vec4<f32>,camera:vec4<f32>,viewport:vec4<f32>,eye:vec4<f32>,sphereEye:vec4<f32>}
struct Header {count:u32,status:u32,sources:u32,triangles:u32,triangles_high:u32,budget:u32,complete:u32,pad:u32}
@group(0) @binding(0) var<storage,read> sources:array<Source>;
@group(0) @binding(1) var<storage,read> covers:array<vec4<u32>>;
@group(0) @binding(2) var<uniform> params:Params;
@group(0) @binding(3) var<storage,read_write> work:Workspace;
@group(0) @binding(4) var<storage,read_write> output:array<Patch>;
@group(0) @binding(5) var<storage,read_write> groups:array<vec2<u32>>;
@group(0) @binding(6) var<storage,read_write> result:Header;
@group(0) @binding(7) var<storage,read_write> scratch:array<atomic<u32>>;
var<private> itemCount:u32;var<private> low:u32;var<private> high:u32;
fn unit()->f32{return params.fraction.z;}
fn coordinate(g:u32,axis:u32)->f32{
 let integer=bitcast<i32>(g-params.grid[axis]);let d=f32(integer);
 let residual=f32(integer-i32(d));let product=d*params.fraction.z;
 let error=fma(d,params.fraction.z,-product)+d*params.fraction.w+residual*params.fraction.z+residual*params.fraction.w-params.fraction[axis];
 return product+error;
}
fn rect(g:vec3<u32>)->vec4<f32>{return vec4(coordinate(g.x,0u),coordinate(g.y,1u),coordinate(g.x+g.z,0u),coordinate(g.y+g.z,1u));}
fn nextPower(n:u32)->u32{return 1u<<(32u-countLeadingZeros(max(n,1u)-1u));}
fn visibleDepth(g:vec3<u32>)->f32{
 let b=rect(g);let side=f32(g.z)*unit();let near=params.right.w;
 if params.earth.w>0.&&side>params.earth.z*.5{return .2;}
 let curve=select(0.,side*side/params.earth.z,params.earth.w>0.);
 let eye=params.sphereEye;
 if eye.x>=b.x&&eye.x<=b.z&&eye.y>=b.y&&eye.y<=b.w&&eye.z>=-500.-curve&&eye.z<=4500.+curve{return near;}
 var vertices:array<vec3<f32>,8>;
 for(var i=0u;i<8u;i++){
  let q=vec2(select(b.x,b.z,(i&1u)!=0u),select(b.y,b.w,(i&2u)!=0u));
  let height=select(-500.-curve,4500.+curve,(i&4u)!=0u);
  let d=earthPosition(q,height,params.earth,params.camera.w)-vec3(0.,params.camera.x,0.);
  vertices[i]=vec3(dot(d,params.right.xyz),dot(d,params.up.xyz),params.back.w-dot(d,params.back.xyz));
 }
 let planes=array<vec4<f32>,6>(vec4(0.,0.,1.,-near),vec4(0.,0.,-1.,params.up.w),vec4(1.,0.,params.camera.y*params.camera.z*1.15,0.),vec4(-1.,0.,params.camera.y*params.camera.z*1.15,0.),vec4(0.,1.,params.camera.y*1.15,0.),vec4(0.,-1.,params.camera.y*1.15,0.));
 var crossing:array<bool,6>;var clipped=false;
 for(var i=0u;i<6u;i++){
  var inside=0u;for(var j=0u;j<8u;j++){inside+=select(0u,1u,dot(planes[i].xyz,vertices[j])+planes[i].w>=0.);}
  if inside==0u{return -1.;}crossing[i]=inside!=8u;clipped=clipped||crossing[i];
 }
 var minimum=1e30;
 if !clipped{for(var i=0u;i<8u;i++){minimum=min(minimum,vertices[i].z);}return max(minimum,near);}
 let faces=array<vec4<u32>,6>(vec4(0u,1u,3u,2u),vec4(4u,6u,7u,5u),vec4(0u,4u,5u,1u),vec4(2u,3u,7u,6u),vec4(0u,2u,6u,4u),vec4(1u,5u,7u,3u));
 for(var face=0u;face<6u;face++){
  var polygon:array<vec3<f32>,16>;var count=4u;
  for(var i=0u;i<4u;i++){polygon[i]=vertices[faces[face][i]];}
  for(var plane=0u;plane<6u;plane++){
   if !crossing[plane]{continue;}
   var next:array<vec3<f32>,16>;var size=0u;
   for(var i=0u;i<count;i++){
    let a=polygon[i];let b=polygon[(i+1u)%count];let da=dot(planes[plane].xyz,a)+planes[plane].w;let db=dot(planes[plane].xyz,b)+planes[plane].w;
    if da>=0.{if size>=16u{return near;}next[size]=a;size++;}
    if (da>=0.)!=(db>=0.){if size>=16u{return near;}next[size]=mix(a,b,da/(da-db));size++;}
   }
   polygon=next;count=size;if count==0u{break;}
  }
  for(var i=0u;i<count;i++){minimum=min(minimum,polygon[i].z);}
 }
 if minimum>=1e29{return -1.;}return max(minimum,near);
}
fn depthForDensity(g:vec3<u32>,depth:f32)->f32{
 if params.viewport.z==0.{return depth;}
 let b=rect(g);let q=params.eye.xyz;
 let delta=vec2(q.x-clamp(q.x,b.x,b.z),q.y-clamp(q.y,b.y,b.w))*params.viewport.w;
 let dz=q.z-clamp(q.z,0.,4500.);
 return max(length(vec3(delta,dz)),.2);
}
fn hashLine(line:u32,axis:u32)->u32{
 var x=line^(axis*0x9e3779b9u);x^=x>>16u;x*=0x7feb352du;x^=x>>15u;x*=0x846ca68bu;x^=x>>16u;return x&(HEADS-1u);
}
const EDGE_HEAD=8u;const EDGE_DATA=65544u;
fn edgeAddress(id:u32)->u32{return EDGE_DATA+id*8u;}
fn edgeRoot(id:u32)->u32{
 var at=id;
 loop{let parent=atomicLoad(&scratch[edgeAddress(at)]);if parent==at{break;}at=parent;}
 return at;
}
fn addTriangles(n:u32){let old=low;low+=2u*n*n;high+=select(0u,1u,low<old);}
fn removeTriangles(n:u32){let value=2u*n*n;high-=select(0u,1u,low<value);low-=value;}
// 元の深さ優先順を種区画の連番で保持し、atomic追加の実行順には依存しない。
@compute @workgroup_size(64)
fn prepareSources(@builtin(global_invocation_id) id:vec3<u32>){
 let tile=id.x;if tile>=params.grid.z{return;}
 let source=sources[tile];let g=source.grid.xyz;
 let raw=visibleDepth(g);var k=0u;
 if raw>=0.{
  let pixels=f32(g.z)*unit()*params.camera.w*params.viewport.x/(2.*params.camera.y*depthForDensity(g,raw));
  let screen=nextPower(u32(clamp(ceil(pixels/128.),1.,32.)));
  k=min(min(clamp(nextPower(u32(max(ceil(f32(g.z)*unit()/2000.),1.))),1u,32u),max(source.grid.w/8u,1u)),screen);
 }
 work.counts[tile]=k;
}

// 一つの粗区画に細分化が集中しても、8スレッドで深さ優先の候補を処理する。
// 最大30段でも3*8*30+1ノードなので、1024件の共有スタックに収まる。
var<workgroup> stackX:array<u32,1024>;
var<workgroup> stackY:array<u32,1024>;
var<workgroup> stackSide:array<u32,1024>;
var<workgroup> pendingNodes:atomic<u32>;
var<workgroup> batchSize:u32;
var<workgroup> batchBase:u32;
fn generateNode(tile:u32,rank:u32,node:vec3<u32>,k:u32,base:u32){
 let source=sources[tile];let raw=visibleDepth(node);if raw<0.{return;}
 var overlap=false;var covered=false;
 for(var j=0u;j<source.coverage.y;j++){
  let c=covers[source.coverage.x+j];
  if node.x<c.z&&node.x+node.z>c.x&&node.y<c.w&&node.y+node.z>c.y{
   overlap=true;if node.x>=c.x&&node.y>=c.y&&node.x+node.z<=c.z&&node.y+node.z<=c.w{covered=true;}
  }
 }
 if covered{return;}
 if overlap&&node.z>1u{
  let half=node.z/2u;let start=atomicAdd(&pendingNodes,4u);
  if start+4u>1024u{atomicStore(&scratch[1],2u);return;}
  for(var y=0u;y<2u;y++){for(var x=0u;x<2u;x++){let index=start+y*2u+x;stackX[index]=node.x+x*half;stackY[index]=node.y+y*half;stackSide[index]=half;}}
  return;
 }
 let depth=depthForDensity(node,raw);
 let pixels=f32(node.z)*unit()*params.camera.w*params.viewport.x/(2.*params.camera.y*max(depth,.2));
 let maximum=min(nextPower(u32(max(ceil(f32(max(source.grid.w/min(k,8u),1u))*f32(node.z)/f32(base)),1.))),512u);
 let n=clamp(nextPower(u32(clamp(ceil(pixels/params.viewport.y),1.,f32(maximum)))),1u,maximum);
 let out=atomicAdd(&scratch[0],1u);if out>=MAX_ITEMS{atomicStore(&scratch[1],1u);return;}
 let next=atomicExchange(&scratch[8u+tile*1024u+rank],out+1u);
 work.temporary[out]=Item(vec4(tile,node),vec4(n,1u,bitcast<u32>(pixels),rank),bitcast<vec4<f32>>(vec4(0u,next,0u,0u)),vec4(0.));
}
@compute @workgroup_size(8)
fn generateSeeds(@builtin(workgroup_id) invocation:vec3<u32>,@builtin(local_invocation_index) lane:u32){
 let task=invocation.x+invocation.y*65535u;
 // uniform loadにより全スレッドが同じ終了条件とスタック位置を使う。
 if lane==0u{
  batchSize=select(0u,1u,task<atomicLoad(&scratch[3]));
 }
 workgroupBarrier();let hasTask=workgroupUniformLoad(&batchSize);if hasTask==0u{return;}
 var lo=0u;var hi=params.grid.z;
 loop{if lo+1u>=hi{break;}let middle=(lo+hi)/2u;if work.offsets[middle]<=task{lo=middle;}else{hi=middle;}}
 let tile=lo;let seed=task-work.offsets[tile];let k=work.counts[tile];let rank=k*k-1u-seed;let base=sources[tile].grid.z/k;
 if lane==0u{atomicStore(&pendingNodes,1u);stackX[0]=sources[tile].grid.x+(seed%k)*base;stackY[0]=sources[tile].grid.y+(seed/k)*base;stackSide[0]=base;}
 workgroupBarrier();
 loop{
  if lane==0u{
   let pending=atomicLoad(&pendingNodes);let batch=select(min(pending,8u),0u,atomicLoad(&scratch[1])!=0u);
   batchSize=batch;batchBase=pending-batch;atomicStore(&pendingNodes,pending-batch);
  }
  workgroupBarrier();let amount=workgroupUniformLoad(&batchSize);let start=workgroupUniformLoad(&batchBase);if amount==0u{break;}
  var node=vec3(0u);if lane<amount{node=vec3(stackX[start+lane],stackY[start+lane],stackSide[start+lane]);}
  workgroupBarrier();
  if lane<amount{generateNode(tile,rank,node,k,base);}
  workgroupBarrier();
 }
}
@compute @workgroup_size(64)
fn prefixSeeds(@builtin(global_invocation_id) id:vec3<u32>){
 let tile=id.x;if tile>=params.grid.z||atomicLoad(&scratch[1])!=0u{return;}let k=work.counts[tile];var sum=0u;
 for(var i=0u;i<k*k;i++){
  let index=8u+tile*1024u+i;let head=atomicLoad(&scratch[index]);var cursor=head;var count=0u;
  loop{if cursor==0u{break;}let leaf=cursor-1u;work.temporary[leaf].edges.z=bitcast<f32>(head);cursor=bitcast<u32>(work.temporary[leaf].edges.y);count++;}
  atomicStore(&scratch[index],sum);sum+=count;
 }
 work.offsets[tile]=sum;
}
@compute @workgroup_size(1)
fn prefixFields(){
 result.status=atomicLoad(&scratch[1]);result.count=min(atomicLoad(&scratch[0]),MAX_ITEMS);
 var sum=0u;for(var i=0u;i<params.grid.z;i++){work.counts[i]=sum;sum+=work.offsets[i];}
}
@compute @workgroup_size(64)
fn scatterItems(@builtin(global_invocation_id) id:vec3<u32>){
 let i=id.x;if i>=result.count||result.status!=0u{return;}
 var p=work.temporary[i];var local=0u;var cursor=bitcast<u32>(p.edges.z);
 loop{
  if cursor==0u{break;}let other=work.temporary[cursor-1u];cursor=bitcast<u32>(other.edges.y);
  let x=p.grid.y^other.grid.y;let y=p.grid.z^other.grid.z;
  if (x|y)==0u{continue;}
  // 子の処理順はY優先の逆Morton順。共有スタックの実行順から独立させる。
  let xbit=32u-countLeadingZeros(x);let ybit=32u-countLeadingZeros(y);
  let before=select(other.grid.y>p.grid.y,other.grid.z>p.grid.z,ybit>=xbit);
  local+=select(0u,1u,before);
 }
 let seed=atomicLoad(&scratch[8u+p.grid.x*1024u+p.plan.w]);
 let index=work.counts[p.grid.x]+seed+local;p.plan.w=0u;p.edges=vec4(0.);work.items[index]=p;
}

@compute @workgroup_size(64)
fn edgeInit(@builtin(global_invocation_id) invocation:vec3<u32>){
 let id=invocation.x;if result.status!=0u||id>=result.count*4u{return;}
 let i=id/4u;let e=id%4u;let item=work.items[i];let axis=e/2u;
 let line=select(item.grid.y,item.grid.z,axis==1u)+select(0u,item.grid.w,(e&1u)!=0u);
 let start=select(item.grid.z,item.grid.y,axis==1u);let address=edgeAddress(id);
 atomicStore(&scratch[address],id);atomicStore(&scratch[address+1u],0xffffffffu);
 atomicStore(&scratch[address+2u],0u);atomicStore(&scratch[address+3u],0u);
 atomicStore(&scratch[address+4u],0xffffffffu);atomicStore(&scratch[address+5u],0xffffffffu);atomicStore(&scratch[address+6u],0u);
 let previous=atomicExchange(&scratch[EDGE_HEAD+hashLine(line,axis)],id+1u);
 work.edges[id]=Edge(vec4(i,e,start,start+item.grid.w),vec4(line,axis,id,previous),vec4(0.),vec4(0u));
}
@compute @workgroup_size(64)
fn edgeUnion(@builtin(global_invocation_id) invocation:vec3<u32>){
 let id=invocation.x;if result.status!=0u||id>=result.count*4u{return;}
 let edge=work.edges[id];let item=work.items[id/4u];let source=sources[item.grid.x];
 var cursor=atomicLoad(&scratch[EDGE_HEAD+hashLine(edge.tree.x,edge.tree.y)]);
 loop{
  if cursor==0u{break;}let otherId=cursor-1u;let other=work.edges[otherId];cursor=other.tree.w;
  if otherId>=id||other.tree.x!=edge.tree.x||other.tree.y!=edge.tree.y||(other.geometry.y^1u)!=edge.geometry.y||other.geometry.z>=edge.geometry.w||other.geometry.w<=edge.geometry.z{continue;}
  let tile=work.items[other.geometry.x].grid.x;let b=sources[tile];
  if tile!=item.grid.x&&source.coverage.y==0u&&b.coverage.y==0u&&all(min(source.grid.xy+vec2(source.grid.z),b.grid.xy+vec2(b.grid.z))>max(source.grid.xy,b.grid.xy)){continue;}
  var a=id;var bId=otherId;
  loop{
   let aroot=edgeRoot(a);let broot=edgeRoot(bId);if aroot==broot{break;}
   let lo=min(aroot,broot);let hi=max(aroot,broot);
   let previous=atomicMin(&scratch[edgeAddress(hi)],lo);if previous==hi{break;}
   // 先に別の辺が連結した場合、その連結先も結ぶ。根の番号は単調に減る。
   a=lo;bId=previous;
  }
 }
}
@compute @workgroup_size(64)
fn edgeReduce(@builtin(global_invocation_id) invocation:vec3<u32>){
 let id=invocation.x;if result.status!=0u||id>=result.count*4u{return;}
 let root=edgeRoot(id);let address=edgeAddress(root);let item=work.items[id/4u];let source=sources[item.grid.x];
 atomicMin(&scratch[address+1u],item.grid.w);
 atomicMax(&scratch[address+3u],bitcast<u32>(f32(source.grid.z)*unit()/f32(source.grid.w)));
 atomicMin(&scratch[address+4u],work.edges[id].geometry.z);
 atomicMin(&scratch[address+5u],item.grid.x);atomicMax(&scratch[address+6u],item.grid.x);
}
@compute @workgroup_size(64)
fn edgeFinalize(@builtin(global_invocation_id) invocation:vec3<u32>){
 let id=invocation.x;if result.status!=0u||id>=result.count*4u||edgeRoot(id)!=id{return;}
 let address=edgeAddress(id);let tile=atomicLoad(&scratch[address+5u]);let other=atomicLoad(&scratch[address+6u]);
 work.edges[id].stats=vec4(f32(atomicLoad(&scratch[address+1u])),0.,bitcast<f32>(atomicLoad(&scratch[address+3u])),0.);
 work.edges[id].extra=vec4(tile,select(0u,1u,tile!=other),atomicLoad(&scratch[address+4u]),0u);
}

// 各パッチの「半減候補」は優先度が単調増加する。閾値を並列探索し、
// 閾値と同点の候補だけを従来の逆番号順で確定する。ヒープの逐次更新を省く。
@compute @workgroup_size(64)
fn prepareBudget(@builtin(global_invocation_id) invocation:vec3<u32>){
 let i=invocation.x;if i>=result.count||result.status!=0u{return;}var required=1u;let side=work.items[i].grid.w;
 for(var e=0u;e<4u;e++){let shortest=work.edges[edgeRoot(i*4u+e)].stats.x;required=max(required,min(nextPower(u32(round(f32(side)/shortest))),512u));}
 work.items[i].plan.y=required;work.items[i].plan.x=max(work.items[i].plan.x,required);
}
var<workgroup> sums:array<u32,256>;
var<workgroup> carries:array<u32,256>;
var<workgroup> budgetLimit:u32;
fn sumGroup(value:u32,lane:u32)->vec2<u32>{
 sums[lane]=value;carries[lane]=0u;workgroupBarrier();
 for(var stride=128u;stride>0u;stride/=2u){
  if lane<stride{let previous=sums[lane];let next=previous+sums[lane+stride];sums[lane]=next;carries[lane]+=carries[lane+stride]+select(0u,1u,next<previous);}
  workgroupBarrier();
 }
 let answer=vec2(sums[0],carries[0]);workgroupBarrier();return answer;
}
fn priority(pixels:f32,n:u32)->u32{return bitcast<u32>(floor(pixels/f32(n)*1024.));}
@compute @workgroup_size(256)
fn budgetThreshold(@builtin(local_invocation_index) lane:u32){
 var sourceSum=0u;for(var i=lane;i<params.grid.z;i+=256u){sourceSum+=2u*sources[i].grid.w*sources[i].grid.w;}
 let total=sumGroup(sourceSum,lane);
 if lane==0u{budgetLimit=select(clamp(total.x,2000000u,4000000u),4000000u,total.y>0u);result.budget=budgetLimit;}
 workgroupBarrier();
 var lo=0u;var hi=0x7f7fffffu;
 for(var iteration=0u;iteration<31u;iteration++){
  let middle=lo+(hi-lo)/2u;var local=0u;
  for(var i=lane;i<result.count;i+=256u){
   let item=work.items[i];var n=item.plan.x;
   loop{if n<=item.plan.y||priority(bitcast<f32>(item.plan.z),n)>middle{break;}n/=2u;}
   local+=2u*n*n;
  }
  let remaining=sumGroup(local,lane);
  if remaining.y!=0u||remaining.x>budgetLimit{lo=middle+1u;}else{hi=middle;}
 }
 if lane==0u{atomicStore(&scratch[2],lo);}
}
@compute @workgroup_size(64)
fn budgetApply(@builtin(global_invocation_id) invocation:vec3<u32>){
 let i=invocation.x;if i>=result.count||result.status!=0u{return;}
 let threshold=atomicLoad(&scratch[2]);let item=work.items[i];var n=item.plan.x;
 loop{if n<=item.plan.y||priority(bitcast<f32>(item.plan.z),n)>=threshold{break;}n/=2u;}
 work.items[i].plan.x=n;
}
@compute @workgroup_size(1)
fn finishBudget(){
 if result.status!=0u{return;}itemCount=result.count;low=0u;high=0u;
 for(var i=0u;i<itemCount;i++){addTriangles(work.items[i].plan.x);}
 let threshold=atomicLoad(&scratch[2]);
 for(var index=itemCount;index>0u;index--){
  let i=index-1u;
  loop{
   if high==0u&&low<=result.budget{break;}
   let item=work.items[i];let n=item.plan.x;
   if n<=item.plan.y||priority(bitcast<f32>(item.plan.z),n)!=threshold{break;}
   removeTriangles(n);work.items[i].plan.x=n/2u;addTriangles(n/2u);
  }
 }
 result.triangles=low;result.triangles_high=high;
}
@compute @workgroup_size(64)
fn edgeDrawReduce(@builtin(global_invocation_id) invocation:vec3<u32>){
 let id=invocation.x;if id>=result.count*4u||result.status!=0u{return;}
 let item=work.items[id/4u];let address=edgeAddress(edgeRoot(id));
 atomicMax(&scratch[address+2u],bitcast<u32>(f32(item.grid.w)*unit()/f32(item.plan.x)));
}
@compute @workgroup_size(64)
fn writePatchEdges(@builtin(global_invocation_id) invocation:vec3<u32>){
 let i=invocation.x;if i>=result.count||result.status!=0u{return;}
 for(var e=0u;e<4u;e++){
  let address=edgeAddress(edgeRoot(i*4u+e));let draw=bitcast<f32>(atomicLoad(&scratch[address+2u]));let field=bitcast<f32>(atomicLoad(&scratch[address+3u]));
  let cross=atomicLoad(&scratch[address+5u])!=atomicLoad(&scratch[address+6u]);
  let step=min(select(draw,max(draw,field),cross),f32(atomicLoad(&scratch[address+1u]))*unit());
  work.items[i].edges[e]=step;work.items[i].origins[e]=bitcast<f32>(atomicLoad(&scratch[address+4u]));
 }
}

@compute @workgroup_size(1)
fn prepareTasks(){
 var total=0u;for(var i=0u;i<params.grid.z;i++){work.offsets[i]=total;let k=work.counts[i];total+=k*k;}
 atomicStore(&scratch[3],total);let groups=max(total,1u);
 atomicStore(&scratch[4],min(groups,65535u));atomicStore(&scratch[5],(groups+65534u)/65535u);atomicStore(&scratch[6],1u);
}
@compute @workgroup_size(64)
fn markSplit(@builtin(global_invocation_id) invocation:vec3<u32>){
 let i=invocation.x;if i>=result.count||result.status!=0u{return;}let p=work.items[i];var divide=false;
 for(var e=0u;e<4u;e++){divide=divide||f32(p.grid.w)/f32(p.plan.x)>work.edges[edgeRoot(i*4u+e)].stats.x*1.001;}
 work.counts[i]=select(1u,4u,divide);
}
@compute @workgroup_size(1)
fn prefixSplit(){
 if result.status!=0u{return;}let old=result.count;var sum=0u;
 for(var i=0u;i<old;i++){work.offsets[i]=sum;sum+=work.counts[i];}
 if sum>MAX_ITEMS{result.status=1u;return;}atomicStore(&scratch[7],old);result.count=sum;
}
@compute @workgroup_size(64)
fn scatterSplit(@builtin(global_invocation_id) invocation:vec3<u32>){
 let i=invocation.x;if i>=atomicLoad(&scratch[7])||result.status!=0u{return;}
 let p=work.items[i];let out=work.offsets[i];
 if work.counts[i]==1u{work.temporary[out]=p;return;}
 let half=p.grid.w/2u;
 for(var y=0u;y<2u;y++){for(var x=0u;x<2u;x++){
  work.temporary[out+y*2u+x]=Item(vec4(p.grid.x,p.grid.y+half*x,p.grid.z+half*y,half),vec4(max(p.plan.x/2u,1u),1u,bitcast<u32>(bitcast<f32>(p.plan.z)*.5),0u),vec4(0.),vec4(0.));
 }}
}
@compute @workgroup_size(64)
fn copySplit(@builtin(global_invocation_id) invocation:vec3<u32>){
 let i=invocation.x;if i<result.count&&result.status==0u{work.items[i]=work.temporary[i];}
}
fn tileBegin(tile:u32)->u32{
 var lo=0u;var hi=result.count;
 loop{if lo>=hi{break;}let middle=(lo+hi)/2u;if work.items[middle].grid.x<tile{lo=middle+1u;}else{hi=middle;}}
 return lo;
}
@compute @workgroup_size(64)
fn serializeFields(@builtin(global_invocation_id) invocation:vec3<u32>){
 let tile=invocation.x;if tile>=params.grid.z||result.status!=0u{return;}
 let start=tileBegin(tile);let end=tileBegin(tile+1u);var counts:array<u32,10>;var offsets:array<u32,10>;
 for(var i=start;i<end;i++){let level=31u-countLeadingZeros(work.items[i].plan.x);counts[level]++;}
 var offset=start;for(var level=0u;level<10u;level++){offsets[level]=offset;groups[tile*10u+level]=vec2(offset,counts[level]);offset+=counts[level];}
 for(var i=start;i<end;i++){
  let p=work.items[i];let level=31u-countLeadingZeros(p.plan.x);let out=offsets[level];offsets[level]++;
  let origin=sources[tile].grid.xy;
  output[out]=Patch(bitcast<vec4<f32>>(vec4(p.grid.yz-origin,p.grid.w,p.plan.x)),p.edges,p.origins,bitcast<vec4<f32>>(vec4(p.grid.y,p.grid.y+p.grid.w,p.grid.z,p.grid.z+p.grid.w)));
 }
}
@compute @workgroup_size(1)
fn completePlan(){if result.status==0u{result.sources=params.grid.z;result.complete=1u;}}

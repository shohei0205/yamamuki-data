struct Sample { original: vec4<f32>, adjusted: vec4<f32> }
struct View { matrix: mat4x4<f32>, offset: vec4<f32>, settings: vec4<f32>, flags: vec4<u32>, extra:vec4<u32>, earth:vec4<f32>, eye:vec4<f32>, frame:vec4<f32> }
struct Patch { bounds:vec4<f32>, edges:vec4<f32>, origins:vec4<f32>, lines:vec4<f32> }
struct Record { bounds:vec4<f32>, data:vec4<u32>, neighbors:vec4<u32>, tile:vec4<f32>, tile_end:vec4<f32>, draw:vec4<u32>, transition:vec4<u32>, blend_data:vec4<u32> }
@group(0) @binding(0) var<storage,read> samples: array<Sample>;
@group(0) @binding(1) var<uniform> view: View;
@group(0) @binding(2) var<storage,read> patches: array<Patch>;
@group(0) @binding(3) var<storage,read> atlas: array<Sample>;
@group(0) @binding(4) var<storage,read> records: array<Record>;
@group(0) @binding(5) var<storage,read> links:array<u32>;
@group(0) @binding(6) var<storage,read> controls:array<vec4<f32>>;
@group(0) @binding(7) var<storage,read> triangles:array<u32>;
struct LandcoverView { areas:array<vec4<f32>,3>, settings:vec4<f32> }
@group(1) @binding(0) var landcoverOverview:texture_2d<f32>;
@group(1) @binding(1) var landcoverDetail:texture_2d<f32>;
@group(1) @binding(2) var landcoverSampler:sampler;
@group(1) @binding(3) var<uniform> landcover:LandcoverView;

@group(1) @binding(4) var landcoverPrevious:texture_2d<f32>;

fn landcoverSample(tex:texture_2d<f32>,area:vec4<f32>,q:vec2<f32>)->vec4<f32>{
 if area.z>0.{
  let uv=(q-area.xy)/area.zw;
  if all(uv>=vec2(0.))&&all(uv<vec2(1.)){
   if landcover.settings.z>0.{
    let size=textureDimensions(tex);
    let pixel=min(vec2<u32>(uv*vec2<f32>(size)),size-vec2(1u));
    return textureLoad(tex,vec2<i32>(pixel),0);
   }
   return textureSampleLevel(tex,landcoverSampler,uv,0.);
  }
 }
 return vec4(0.);
}

fn landcoverContains(area:vec4<f32>,q:vec2<f32>)->bool{
 return area.z>0. && all(q>=area.xy) && all(q<area.xy+area.zw);
}

fn landcoverColor(p:vec3<f32>,fallback:vec3<f32>)->vec3<f32>{
 let q=p.xz/view.offset.z;
 if landcover.settings.y<0.{
  let wide=landcoverSample(landcoverOverview,landcover.areas[0],q);
  var result=fallback*(1.-wide.a)+wide.rgb;
  let mid=landcoverSample(landcoverPrevious,landcover.areas[2],q);
  let near=landcoverSample(landcoverDetail,landcover.areas[1],q);
  let midUV=(q-landcover.areas[2].xy)/max(landcover.areas[2].zw,vec2(1.));
  let nearUV=(q-landcover.areas[1].xy)/max(landcover.areas[1].zw,vec2(1.));
  let midEdge=smoothstep(0.,0.08,min(min(midUV.x,midUV.y),min(1.-midUV.x,1.-midUV.y)));
  let nearEdge=smoothstep(0.,0.08,min(min(nearUV.x,nearUV.y),min(1.-nearUV.x,1.-nearUV.y)));
  result=mix(result,result*(1.-mid.a)+mid.rgb,midEdge);
  return mix(result,result*(1.-near.a)+near.rgb,nearEdge);
 }
 let blend=landcover.settings.x;
 let next=landcoverSample(landcoverDetail,landcover.areas[1],q);
 if blend>=1. && next.a>=1.{ return next.rgb; }
 let overview=landcoverSample(landcoverOverview,landcover.areas[0],q);
 let base=fallback*(1.-overview.a)+overview.rgb;
 let waterOnly=landcover.settings.y>0.;
 // Transparent water texels inside the detailed image expose the terrain.
 let currentBase=select(base,fallback,waterOnly && landcoverContains(landcover.areas[1],q));
 let current=currentBase*(1.-next.a)+next.rgb;
 if blend>=1.{ return current; }
 let old=landcoverSample(landcoverPrevious,landcover.areas[2],q);
 let oldBase=select(base,fallback,waterOnly && landcoverContains(landcover.areas[2],q));
 return mix(oldBase*(1.-old.a)+old.rgb,current,blend);
}
struct Varying {
 @builtin(position) clip: vec4<f32>,
 @location(0) position: vec3<f32>,
 @location(1) normal: vec3<f32>,
 @location(2) height: f32,
 @location(3) bary:vec3<f32>,
 @location(4) uv: vec2<f32>,
}
fn sampleAt(q:vec2<f32>,id:u32)->vec3<f32>{
 let r=records[id];let n=r.data.y;
 let uv=clamp((q-r.bounds.xy)/r.bounds.z,vec2(0.),vec2(1.))*f32(n);
 let ij=min(vec2<u32>(floor(uv)),vec2(n-1u));let f=uv-vec2<f32>(ij);
 let base=r.data.x+ij.y*(n+1u)+ij.x;
 let a=atlas[base];let b=atlas[base+1u];let c=atlas[base+n+1u];let d=atlas[base+n+2u];
 let va=mix(a.original,a.adjusted,view.settings.y).xyz;
 let vb=mix(b.original,b.adjusted,view.settings.y).xyz;
 let vc=mix(c.original,c.adjusted,view.settings.y).xyz;
 let vd=mix(d.original,d.adjusted,view.settings.y).xyz;
 let value=mix(mix(va,vb,f.x),mix(vc,vd,f.x),f.y);
 let anchor=clamp(q,r.bounds.xy,r.bounds.xy+vec2(r.bounds.z));
 return vec3(value.x+dot(value.yz,q-anchor),value.yz);
}
fn inside(q:vec2<f32>,r:Record)->bool{
 return all(q>=r.bounds.xy-vec2(.01))&&all(q<=r.bounds.xy+vec2(r.bounds.z+.01));
}
// 隣接する粗タイルの辺長は数十万mになる。端点を加算で復元すると
// 目の前の境界が数cmずれ、隣接面を重なった代替面と誤判定してしまう。
fn tileEnd(r:Record)->vec2<f32>{
 if (view.extra.w&32u)!=0u{return r.tile_end.xy;}
 return r.tile.xy+vec2(r.tile.z);
}
fn band(r:Record)->f32{
 if (view.extra.w&16u)!=0u {return r.tile.w;}
 if (view.extra.w&8u)!=0u&&r.neighbors.y!=0u {return bitcast<f32>(r.neighbors.y);}
 return min(r.bounds.z/8.,max(1.,2.*r.bounds.z/f32(r.data.y)));
}
fn blend(q:vec2<f32>,owner:u32,old:bool)->vec3<f32>{
 let own=records[owner];
 let compact=(view.extra.w&64u)!=0u;
 let neighborBase=select(own.neighbors.x,own.blend_data.x,compact);
 let neighborCount=select(own.data.w,own.blend_data.y,compact);
 let groupEndBase=select(own.neighbors.y,own.blend_data.z,compact);
 if view.extra.z==0u{return sampleAt(q,owner);}
 if (view.extra.w&8u)!=0u&&own.draw.w!=0u&&inside(q,own){
  let edge=min(q-own.tile.xy,tileEnd(own)-q);
  if min(edge.x,edge.y)>min(own.tile.w,bitcast<f32>(own.draw.w)){return sampleAt(q,owner);}
 }
 var sum=vec3(0.);var weight=0.;var j=0u;
 // 評価領域の数を接合の重みにしない。一つの元タイルから一つの値だけを使う。
 loop {
  if j>=neighborCount {break;}
  let first=links[neighborBase+j];let source=records[first];
  let isOld=first>=view.extra.x;
  if (view.extra.w&16u)!=0u {
   let pad=source.tile.w;
   let distance=max(max(source.tile.x-q.x,q.x-tileEnd(source).x),max(source.tile.y-q.y,q.y-tileEnd(source).y));
   if isOld!=old||distance>=pad {j=links[groupEndBase+j];continue;}
  }
  // 同じ元タイルの評価区画では、元タイルの包含関係は共通している。
  // 区画ごとの反復へ入れると数十区画ぶん同じ判定を繰り返す。
  let overlap=all(min(tileEnd(source),tileEnd(own))-max(source.tile.xy,own.tile.xy)>vec2(.01));
  let outside=any(q<source.tile.xy-vec2(.01))||any(q>tileEnd(source)+vec2(.01));
  var chosen=0xffffffffu;var nearest=1e30;var spacing=1e30;var background=1e30;
  let interior=(q>own.bounds.xy+vec2(.01))&(q<own.bounds.xy+vec2(own.bounds.z-.01));
  let sourceEdge=(abs(q-own.tile.xy)<vec2(.01))|(abs(q-tileEnd(own))<vec2(.01));
  if (view.extra.w&16u)!=0u&&own.transition.w!=0u&&all(source.tile.xyz==own.tile.xyz)&&inside(q,own)&&all(interior|sourceEdge) {
   chosen=owner;j=links[groupEndBase+j];
  }else{loop {
   if j>=neighborCount {break;}
   let id=links[neighborBase+j];let r=records[id];
   if any(r.tile.xyz!=source.tile.xyz)||(id>=view.extra.x)!=isOld {break;}
   j++;
   if isOld!=old {continue;}
   if overlap&&r.bounds.w>0.&&(own.bounds.w==0.||r.bounds.z>own.bounds.z*1.001){continue;}
   // 局所区画の端点にもinsideと同じ許容幅を使う。丸め差だけで
   // 同じ元タイルの粗い代替面へ戻ることを避ける。
   let d=max(0.,max(max(r.bounds.x-q.x,q.x-r.bounds.x-r.bounds.z),max(r.bounds.y-q.y,q.y-r.bounds.y-r.bounds.z))-.01);
   let cell=r.bounds.z/f32(r.data.y);
   // 隣の元タイルから外挿する場合は、カメラで出入りする局所区画でなく
   // 元タイル全体の固定評価格子を使う。近い区画の切替で外挿の傾きを変えない。
   let full=all(abs(r.bounds.xy-r.tile.xy)<vec2(.01))&&abs(r.bounds.z-r.tile.z)<.01;
   let fixed=full&&r.data.y==32u;
   // 元タイル外では固定格子が常に最優先。残りの局所区画を調べても
   // 結果は変わらないため、元タイル単位の終端表で次の候補群へ進む。
   if outside&&fixed&&(view.extra.w&16u)!=0u {
    chosen=id;nearest=d;spacing=cell;background=0.;
    j=links[groupEndBase+j-1u];break;
   }
   let rank=select(r.bounds.w,select(select(2.,1.,full),0.,fixed),outside);
   let better=d<nearest||(d==nearest&&(rank<background||(rank==background&&(cell<spacing||(cell==spacing&&id<chosen)))));
   if select(better,rank<background||(rank==background&&better),outside){
    chosen=id;nearest=d;spacing=cell;background=rank;
   }
  }
  }
  if chosen==0xffffffffu {continue;}
  let r=records[chosen];
  let sourcePad=select(band(r),r.tile.w,(view.extra.w&8u)!=0u);
  let pad=select(sourcePad,min(sourcePad,own.tile.w),(view.extra.w&8u)!=0u);
  let overlappingSource=any(r.tile.xyz!=own.tile.xyz)&&all(min(tileEnd(r),tileEnd(own))-max(r.tile.xy,own.tile.xy)>vec2(.01));
  let area=select(r.tile.xyz,r.bounds.xyz,overlappingSource);
  let end=select(tileEnd(r),r.bounds.xy+vec2(r.bounds.z),overlappingSource);
  let distance=max(max(area.x-q.x,q.x-end.x),max(area.y-q.y,q.y-end.y));
  if distance>=pad {continue;}
  let w=smoothstep(-pad,pad,-distance);
  sum+=sampleAt(q,chosen)*w;weight+=w;
 }
 if weight>1e-6{return sum/weight;}
 return sampleAt(q,owner);
}


// 旧表示の三角形を再現する。数学的な曲面の再評価を遷移の始点にしない。
fn previousPatch(index:u32)->Patch {
 var p:Patch;
 p.bounds=bitcast<vec4<f32>>(vec4(links[index],links[index+1u],links[index+2u],links[index+3u]));
 p.edges=bitcast<vec4<f32>>(vec4(links[index+4u],links[index+5u],links[index+6u],links[index+7u]));
 p.origins=bitcast<vec4<f32>>(vec4(links[index+8u],links[index+9u],links[index+10u],links[index+11u]));
 p.lines=bitcast<vec4<f32>>(vec4(links[index+12u],links[index+13u],links[index+14u],links[index+15u]));
 return p;
}
fn previousBoundaryOwner(q:vec2<f32>,reference:u32)->u32{
 let own=records[reference];
 var owner=reference;var found=false;
 // 共通の粗い格子の端点が、所属する細タイルの外にある場合もある。
 // その地点を実際に覆う曲面を使い、細タイルの傾きで遠方へ外挿しない。
 for(var j=0u;j<own.data.w;j++){
  let id=links[own.neighbors.x+j];if id<view.extra.x{continue;}
  let r=records[id];if !inside(q,r){continue;}
  let best=records[owner];
  if !found||r.bounds.w<best.bounds.w||(r.bounds.w==best.bounds.w&&(r.data.z>best.data.z||(r.data.z==best.data.z&&(r.bounds.z<best.bounds.z||(r.bounds.z==best.bounds.z&&id<owner))))){owner=id;found=true;}
 }
 return owner;
}
fn previousBoundaryField(q:vec2<f32>,stepX:f32,originX:f32,stepY:f32,originY:f32,owner:u32)->vec3<f32>{
 let boundary=max(stepX,stepY)>0.;
 var a=q;var b=q;var t=0.;
 if boundary {
  let fx=fract((q.x-originX)/max(stepX,0.000001));
  let fy=fract((q.y-originY)/max(stepY,0.000001));
  let betweenX=stepX>0.&&min(fx,1.-fx)>0.00001;
  let betweenY=stepY>0.&&min(fy,1.-fy)>0.00001;
  let alongX=select(stepX>=stepY,betweenX,betweenX!=betweenY);
  let step=select(stepY,stepX,alongX);
  let origin=select(originY,originX,alongX);
  let coordinate=select(q.y,q.x,alongX);
  let start=origin+floor((coordinate-origin)/step)*step;
  t=clamp((coordinate-start)/step,0.,1.);
  if alongX{a.x+=start-coordinate;b.x+=start+step-coordinate;}
  else{a.y+=start-coordinate;b.y+=start+step-coordinate;}
 }
 // 内部と境界で重い接合関数を重複展開しない。端点上では一回だけ評価する。
 let count=select(2u,1u,t<=0.||t>=1.);
 var value=vec3(0.);
 for(var i=0u;i<count;i++){
  let endpoint=select(i,select(0u,1u,t>=1.),count==1u);
  let weight=select(select(1.-t,t,endpoint>0u),1.,count==1u);
  var anchor=select(a,b,endpoint>0u);
  var selected=owner;
  if boundary {
   selected=previousBoundaryOwner(anchor,owner);
   let r=records[selected];
   anchor=clamp(anchor,r.bounds.xy,r.bounds.xy+vec2(r.bounds.z));
  }
  value+=blend(anchor,selected,true)*weight;
 }
 return value;
}
fn previousVertex(id:u32,part:Patch,owner:u32)->vec3<f32>{
 let n=u32(part.bounds.w);
 let ij=vec2(id%(n+1u),id/(n+1u));
 let corner=(ij.x+ij.y)%3u;
 var xy=part.bounds.xy+vec2<f32>(ij)*part.bounds.z/f32(n);
 var stepX=0.;var originX=0.;var stepY=0.;var originY=0.;
 for(var e=0u;e<4u;e++){
  let hit=select(select(ij.y==0u,ij.y==n,e==3u),select(ij.x==0u,ij.x==n,e==1u),e<2u);
  if hit{
   if e<2u{if part.edges[e]>stepY{stepY=part.edges[e];originY=part.origins[e]+records[owner].bounds.y;}}
   else{if part.edges[e]>stepX{stepX=part.edges[e];originX=part.origins[e]+records[owner].bounds.x;}}
  }
 }
 var q=records[owner].bounds.xy+xy;
 if ij.x==0u{q.x=part.lines.x+records[owner].bounds.x;}if ij.x==n{q.x=part.lines.y+records[owner].bounds.x;}
 if ij.y==0u{q.y=part.lines.z+records[owner].bounds.y;}if ij.y==n{q.y=part.lines.w+records[owner].bounds.y;}
 // 新しい境界分割では端点を共有済み。途中の頂点だけを共通格子へ縮退させ、
 // 同一直線上のT字頂点がGPU丸めで1画素の穴になることも避ける。
 if (view.extra.w&8u)!=0u {
  if stepX>0.&&ij.x>0u&&ij.x<n {q.x=originX+round((q.x-originX)/stepX)*stepX;}
  if stepY>0.&&ij.y>0u&&ij.y<n {q.y=originY+round((q.y-originY)/stepY)*stepY;}
 }
 let field=previousBoundaryField(q,stepX,originX,stepY,originY,owner);
 return vec3(q.x,field.x,q.y);
}

fn previousTriangleHeight(q:vec2<f32>,a:vec3<f32>,b:vec3<f32>,c:vec3<f32>)->vec2<f32>{
 let u=b.xz-a.xz;let v=c.xz-a.xz;let d=q-a.xz;
 let det=u.x*v.y-u.y*v.x;if abs(det)<1e-12{return vec2(0.);}
 let w=vec2(d.x*v.y-d.y*v.x,u.x*d.y-u.y*d.x)/det;
 if any(w<vec2(-.0001))||w.x+w.y>1.0001{return vec2(0.);}
 return vec2(a.y+w.x*(b.y-a.y)+w.y*(c.y-a.y),1.);
}
fn previousHeight(q:vec2<f32>,owner:u32,fallback:f32)->vec2<f32>{
 let r=records[owner];
 for(var j=0u;j<r.draw.y;j++) {
  let part=previousPatch(r.draw.x+j*16u);let origin=r.bounds.xy+part.bounds.xy;
  if any(q<origin-vec2(.01))||any(q>origin+vec2(part.bounds.z+.01)){continue;}
  let n=u32(part.bounds.w);
  let grid=(q-origin)*f32(n)/part.bounds.z;let vertex=round(grid);
  // 旧格子の内部頂点と一致する場所は、既に求めた旧曲面の値がそのまま
  // 描画済み三角形の頂点値になる。四頂点の再評価は境界と新規分割だけにする。
  if all(abs(grid-vertex)<vec2(.001))&&all(vertex>vec2(0.))&&all(vertex<vec2(f32(n))){return vec2(fallback,1.);}
  let ij=min(vec2<u32>(max(vec2(0.),floor(grid))),vec2(n-1u));
  let id=ij.y*(n+1u)+ij.x;
  let ids=array<u32,4>(id,id+1u,id+n+2u,id+n+1u);
  var vertices:array<vec3<f32>,4>;
  for(var k=0u;k<r.draw.z;k++){vertices[k]=previousVertex(ids[k],part,owner);}
  let first=previousTriangleHeight(q,vertices[0],vertices[1],vertices[2]);if first.y>0.{return first;}
  let second=previousTriangleHeight(q,vertices[0],vertices[2],vertices[3]);if second.y>0.{return second;}
 }
 return vec2(0.);
}
fn surfaceFor(q:vec2<f32>,owner:u32,allowMorph:bool)->vec3<f32>{
 let own=records[owner];
 var prior=0xffffffffu;var finest=0u;
 if allowMorph&&view.settings.w<1.&&view.extra.x<arrayLength(&records) {
  if own.transition.x>0u {prior=own.transition.x-1u;}
  else {for(var j=0u;j<own.data.w;j++){
   let id=links[own.neighbors.x+j];
   if id<view.extra.x{continue;}
   let r=records[id];
   let best=records[select(owner,prior,prior!=0xffffffffu)];
   // 同じ元タイルの背景を後から列挙しただけで、表示済みの局所詳細面を
   // 上書きしない。旧表示と同じ優先順位で参照元を選ぶ。
   if inside(q,r)&&(prior==0xffffffffu||r.bounds.w<best.bounds.w||(r.bounds.w==best.bounds.w&&(r.data.z>finest||(r.data.z==finest&&(r.bounds.z<best.bounds.z||(r.bounds.z==best.bounds.z&&id<prior)))))){
    prior=id;finest=r.data.z;
   }
  }}
 }
 // 新旧の接合関数を一つの呼出し箇所にまとめ、コンパイラーの重複展開を抑える。
 let count=select(1u,2u,prior!=0xffffffffu);
 var value=vec3(0.);
 for(var i=0u;i<count;i++){
  let old=i>0u;
  let id=select(owner,prior,old);
  let weight=select(1.,select(view.settings.w,1.-view.settings.w,old),count>1u);
  var field=blend(q,id,old);
  if old {let height=previousHeight(q,id,field.x);if height.y>0.{field.x=height.x;}}
  value+=field*weight;
 }
 return value;
}
fn surface(q:vec2<f32>,owner:u32)->vec3<f32>{return surfaceFor(q,owner,true);}
// 細かい辺のXYを丸めると、部分接合の端点まで移動して面に穴が開く。
// XYは保ち、高さを粗い辺の線分上へ合わせてT字接合を閉じる。
fn boundaryOwner(q:vec2<f32>,reference:u32)->u32{
 let own=records[reference];
 var owner=reference;var found=false;
 // 共通の粗い格子の端点が、所属する細タイルの外にある場合もある。
 // その地点を実際に覆う曲面を使い、細タイルの傾きで遠方へ外挿しない。
 for(var j=0u;j<own.data.w;j++){
  let id=links[own.neighbors.x+j];if id>=view.extra.x{continue;}
  let r=records[id];if !inside(q,r){continue;}
  let best=records[owner];
  if !found||r.bounds.w<best.bounds.w||(r.bounds.w==best.bounds.w&&(r.data.z>best.data.z||(r.data.z==best.data.z&&(r.bounds.z<best.bounds.z||(r.bounds.z==best.bounds.z&&id<owner))))){owner=id;found=true;}
 }
 return owner;
}
fn boundarySampleFor(q:vec2<f32>,reference:u32)->vec3<f32>{
 let owner=boundaryOwner(q,reference);
 let r=records[owner];
 // 共通の参照元を選んでから補間し、境界の双方を同じ速度で切り替える。
 let anchor=clamp(q,r.bounds.xy,r.bounds.xy+vec2(r.bounds.z));
 if (view.extra.w&8u)!=0u {return surface(anchor,owner);}
 return blend(anchor,owner,false);
}
fn boundarySample(q:vec2<f32>)->vec3<f32>{return boundarySampleFor(q,view.flags.w);}
fn boundaryField(q:vec2<f32>,stepX:f32,originX:f32,stepY:f32,originY:f32,owner:u32)->vec3<f32>{
 let boundary=max(stepX,stepY)>0.;
 var a=q;var b=q;var t=0.;
 if boundary {
  let fx=fract((q.x-originX)/max(stepX,0.000001));
  let fy=fract((q.y-originY)/max(stepY,0.000001));
  let betweenX=stepX>0.&&min(fx,1.-fx)>0.00001;
  let betweenY=stepY>0.&&min(fy,1.-fy)>0.00001;
  let alongX=select(stepX>=stepY,betweenX,betweenX!=betweenY);
  let step=select(stepY,stepX,alongX);
  let origin=select(originY,originX,alongX);
  let coordinate=select(q.y,q.x,alongX);
  let start=origin+floor((coordinate-origin)/step)*step;
  t=clamp((coordinate-start)/step,0.,1.);
  if alongX{a.x+=start-coordinate;b.x+=start+step-coordinate;}
  else{a.y+=start-coordinate;b.y+=start+step-coordinate;}
 }
 // 内部と境界で重い接合関数を重複展開しない。端点上では一回だけ評価する。
 let count=select(2u,1u,t<=0.||t>=1.);
 var value=vec3(0.);
 for(var i=0u;i<count;i++){
  let endpoint=select(i,select(0u,1u,t>=1.),count==1u);
  let weight=select(select(1.-t,t,endpoint>0u),1.,count==1u);
  var anchor=select(a,b,endpoint>0u);
  var selected=owner;
  if boundary {
   selected=boundaryOwner(anchor,owner);
   let r=records[selected];
   anchor=clamp(anchor,r.bounds.xy,r.bounds.xy+vec2(r.bounds.z));
  }
  value+=surfaceFor(anchor,selected,!boundary||(view.extra.w&8u)!=0u)*weight;
 }
 return value;
}
fn meshVertex(id:u32,part:Patch,owner:u32)->Varying{
 let n=u32(part.bounds.w);
 let ij=vec2(id%(n+1u),id/(n+1u));
 let corner=(ij.x+ij.y)%3u;
 var xy=part.bounds.xy+vec2<f32>(ij)*part.bounds.z/f32(n);
 var stepX=0.;var originX=0.;var stepY=0.;var originY=0.;
 for(var e=0u;e<4u;e++){
  let hit=select(select(ij.y==0u,ij.y==n,e==3u),select(ij.x==0u,ij.x==n,e==1u),e<2u);
  if hit{
   if e<2u{if part.edges[e]>stepY{stepY=part.edges[e];originY=part.origins[e]+view.offset.y;}}
   else{if part.edges[e]>stepX{stepX=part.edges[e];originX=part.origins[e]+view.offset.x;}}
  }
 }
 var q=records[owner].bounds.xy+xy;
 if ij.x==0u{q.x=part.lines.x+view.offset.x;}if ij.x==n{q.x=part.lines.y+view.offset.x;}
 if ij.y==0u{q.y=part.lines.z+view.offset.y;}if ij.y==n{q.y=part.lines.w+view.offset.y;}
 // 新しい境界分割では端点を共有済み。途中の頂点だけを共通格子へ縮退させ、
 // 同一直線上のT字頂点がGPU丸めで1画素の穴になることも避ける。
 if (view.extra.w&8u)!=0u {
  if stepX>0.&&ij.x>0u&&ij.x<n {q.x=originX+round((q.x-originX)/stepX)*stepX;}
  if stepY>0.&&ij.y>0u&&ij.y<n {q.y=originY+round((q.y-originY)/stepY)*stepY;}
 }
 let field=boundaryField(q,stepX,originX,stepY,originY,owner);
 let p=vec3(q.x*view.offset.z,field.x,q.y*view.offset.z);
 var out:Varying;out.clip=view.matrix*vec4(earthLocal(q,field.x),1.);out.position=p;
 out.normal=normalize(vec3(-field.y/view.offset.z,1.,-field.z/view.offset.z));
 out.height=field.x;out.uv=xy/view.settings.x;
 out.bary=vec3(select(0.,1.,corner%3u==0u),select(0.,1.,corner%3u==1u),select(0.,1.,corner%3u==2u));return out;
}
fn gridVertex(id:u32,instance:u32)->Varying{return meshVertex(id,patches[instance],view.flags.w);}
@vertex fn vs(@builtin(vertex_index) id:u32,@builtin(instance_index) instance:u32)->Varying{
 return gridVertex(id,instance);
}
fn linear(c:vec3<f32>)->vec3<f32>{
 return select(pow((c+vec3(.055))/vec3(1.055),vec3(2.4)),c/vec3(12.92),c<=vec3(.04045));
}
// 画面上のステンシルだけでは、粗い地形の裏側が細かい地形の空へ出てしまう。
// 高い背景だけを除く。低い背景は詳細面の手前の隙間を埋めるために残す。
fn coveredByFiner(q:vec2<f32>,height:f32)->bool {
 let own=records[view.flags.w];
 for(var j=0u;j<own.neighbors.w;j++){
  let r=records[links[own.neighbors.z+j]];
  if inside(q,r)&&height>surface(q,links[own.neighbors.z+j]).x+0.05{return true;}
 }
 return false;
}
struct Fragment { @location(0) color:vec4<f32>, @location(1) world:vec4<f32> }
@fragment fn fs(v:Varying)->Fragment{
 if (view.extra.w&4u)!=0u&&coveredByFiner(v.position.xz/view.offset.z,v.height){discard;}
 var normal=normalize(v.normal);
 if(view.flags.y==0u){normal=normalize(cross(dpdx(v.position),dpdy(v.position)));if(normal.y<0.){normal=-normal;}}
 let light=normalize(vec3(-.55,1.,.65));
 let flat=.39+.70*light.y;
 let shade=clamp(flat+.70*(max(dot(normal,light),0.)-light.y)*view.settings.z,.08,1.3);
 var base=landcoverColor(v.position,mix(vec3(.41,.49,.33),vec3(.65,.62,.47),smoothstep(900.,3800.,v.height)));
 if (view.extra.w&2u)!=0u{
  let hue=fract(f32(records[view.flags.w].data.z)*.618034);
  let channels=abs(fract(vec3(hue)+vec3(0.,2./3.,1./3.))*6.-3.);
  base=.95*mix(vec3(1.),clamp(channels-1.,vec3(0.),vec3(1.)),.7);
 }
 var color=clamp(base*select(shade,1.,landcover.settings.w>0.),vec3(0.),vec3(1.));
 var materialWeight=select(1.,-1.,(view.extra.w&2u)!=0u);
 let width=max(fwidth(v.bary),vec3(.0001));
 let edge=min(min(v.bary.x/width.x,v.bary.y/width.y),v.bary.z/width.z);
 if view.flags.z>0u&&view.flags.z<=2u{
  if view.flags.z==2u&&edge>1.{discard;}
  color=mix(vec3(.035,.055,.04),color,smoothstep(.15,.45,edge));
  if edge<=1. {materialWeight=-1.;}
 }
 if (view.extra.w&1u)!=0u{
  let d=min(v.uv,vec2(1.)-v.uv)/max(fwidth(v.uv),vec2(.000001));
  color*=mix(.25,1.,clamp(min(d.x,d.y),0.,1.));
  if min(d.x,d.y)<1. {materialWeight=-1.;}
 }
 return Fragment(vec4(linear(color),1.),vec4(v.position.xz/view.offset.z,shade,materialWeight*(v.height+10000.)));
}

@vertex fn control_vs(@builtin(vertex_index)id:u32)->Varying{
 let v=controls[triangles[id]];
 let q=records[view.flags.w].tile.xy+v.xy;
 let p=vec3(q.x*view.offset.z,v.z,q.y*view.offset.z);
 var out:Varying;out.clip=view.matrix*vec4(earthLocal(q,v.z),1.);out.position=p;out.height=v.z;out.normal=vec3(0.,1.,0.);
 out.uv=(q-records[view.flags.w].bounds.xy)/view.settings.x;out.bary=vec3(select(0.,1.,id%3u==0u),select(0.,1.,id%3u==1u),select(0.,1.,id%3u==2u));return out;
}
@fragment fn control_fs(v:Varying)->Fragment{
 if coveredByFiner(v.position.xz/view.offset.z,v.height){discard;}
 if any(v.uv<vec2(0.))||any(v.uv>vec2(1.)){discard;}
 var normal=normalize(cross(dpdx(v.position),dpdy(v.position)));if normal.y<0.{normal=-normal;}
 let light=normalize(vec3(-.55,1.,.65));
 let flat=.39+.70*light.y;
 let shade=clamp(flat+.70*(max(dot(normal,light),0.)-light.y)*view.settings.z,.08,1.3);
 let base=landcoverColor(v.position,mix(vec3(.41,.49,.33),vec3(.65,.62,.47),smoothstep(900.,3800.,v.height)));
 var color=clamp(base*select(shade,1.,landcover.settings.w>0.),vec3(0.),vec3(1.));
 let width=max(fwidth(v.bary),vec3(.0001));
 let edge=min(min(v.bary.x/width.x,v.bary.y/width.y),v.bary.z/width.z);
 if view.extra.y==0u{
  if edge>1.{discard;}color=vec3(.06,.12,.18);
 }else if view.flags.z>0u{
  if (view.flags.z==2u||view.flags.z==4u)&&edge>1.{discard;}
  color=mix(vec3(.035,.055,.04),color,smoothstep(.15,.45,edge));
 }
 return Fragment(vec4(linear(color),1.),vec4(v.position.xz/view.offset.z,shade,select(-1.,1.,view.extra.y!=0u && (view.flags.z==0u || edge>1.))*(v.height+10000.)));
}

fn starHash(p:vec2<f32>)->f32{return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453);}
fn space(ray:vec3<f32>,glow:f32)->vec3<f32>{
 let d=normalize(ray);let uv=vec2(atan2(d.x,d.z),asin(d.y));let grid=uv*vec2(180.,180.);let cell=floor(grid);
 let offset=vec2(starHash(cell+17.),starHash(cell+83.));let dist=length(fract(grid)-offset);
 let star=pow(max(1.-dist/.12,0.),3.)*select(0.,1.,starHash(cell)>.982);
 let band=exp(-earthSquare(dot(d,normalize(vec3(.2,.6,.77)))/.11));
 let dust=.5+.5*sin(uv.x*13.+sin(uv.y*7.))*sin(uv.y*19.+uv.x*9.);
 return vec3(.008,.013,.03)+band*(.006+.012*dust)*vec3(.65,.7,1.)+star*mix(vec3(.8,.9,1.),vec3(1.,.85,.65),starHash(cell+92.))+glow*vec3(.06,.24,.55);
}
@vertex fn globe_vs(@builtin(vertex_index)i:u32)->@builtin(position)vec4<f32>{
 let p=array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));return vec4(p[i],0.,1.);
}
struct GlobeFragment { @location(0) color:vec4<f32>, @location(1) world:vec4<f32>, @builtin(frag_depth) depth:f32 }
@fragment fn globe_fs(@builtin(position)pixel:vec4<f32>)->GlobeFragment{
 let back=normalize(view.eye.xyz-vec3(0.,view.eye.w,0.));let right=normalize(vec3(back.z,0.,-back.x));let up=cross(back,right);
 let ray=right*((pixel.x-view.frame.x*.5)*2.*view.frame.z/view.frame.y)-up*((pixel.y-view.frame.y*.5)*2.*view.frame.z/view.frame.y)-back;
 let origin=view.eye.xyz;let r=view.earth.z;let a=dot(ray,ray);let b=dot(origin,ray)+r*ray.y;
 let c=origin.x*origin.x+origin.z*origin.z+origin.y*(2.*r+origin.y);let disc=b*b-a*c;
 let closest=sqrt(max(dot(origin+vec3(0.,r,0.),origin+vec3(0.,r,0.))-b*b/a,0.))-r;
 let glow=select(0.,exp(-max(closest,0.)/60000.),b<0.);
 if disc<0.||b>=0. {return GlobeFragment(vec4(airDaylight(space(ray,glow),origin,ray,r),1.),vec4(0.),0.);}
 let t=c/(-b+sqrt(disc));if t<0.{return GlobeFragment(vec4(airDaylight(space(ray,glow),origin,ray,r),1.),vec4(0.),0.);}
 let p=origin+ray*t;let u=p.y+r;let s=view.earth.x;let co=view.earth.y;let z=u*co+p.z*s;let horizontal=length(vec2(p.x,z));
 var dh=horizontal-z;if z>0. {dh=p.x*p.x/max(horizontal+z,1e-10);}
 let db=earthAngle(-p.z-s*dh,u+co*dh);let ds=s*(-2.*earthSquare(earthSin(db*.5)))+co*earthSin(db);
 let ratio=clamp(ds/(co*co-s*ds),-.999999,.999999);
 var merc=.5*log((1.+ratio)/(1.-ratio));if abs(ratio)<.01 {merc=ratio*(1.+ratio*ratio*(1./3.+ratio*ratio/5.));}
 var dx=r*earthAngle(p.x,z);let circumference=r*6.28318530718;
 if dx>circumference-view.frame.w {dx-=circumference;} if dx < -view.frame.w {dx+=circumference;}
 let q=vec2(dx,-r*merc);
 let rim=pow(1.-clamp(dot(normalize(p+vec3(0.,r,0.)),normalize(-ray)),0.,1.),4.);
 let base=mix(vec3(.36,.43,.30),vec3(.11,.29,.48),rim*.5);
 let clip=view.matrix*vec4(p,1.);
 return GlobeFragment(vec4(base,1.),vec4(q,1.,10000.),clamp(clip.z/clip.w,0.,1.));
}
fn earthLocal(q:vec2<f32>,h:f32)->vec3<f32>{return earthPosition(q,h,view.earth,view.offset.z);}

// 描画と同じ曲面から地点を持ち上げ、実際の描画位置で遮蔽を判定する。
struct Sample {original:vec4<f32>,adjusted:vec4<f32>}
struct Record {bounds:vec4<f32>,data:vec4<u32>,neighbors:vec4<u32>,tile:vec4<f32>,tile_end:vec4<f32>,draw:vec4<u32>,transition:vec4<u32>,blend_data:vec4<u32>}
struct Patch {bounds:vec4<f32>,edges:vec4<f32>,origins:vec4<f32>,lines:vec4<f32>}
struct View {matrix:mat4x4<f32>,offset:vec4<f32>,settings:vec4<f32>,flags:vec4<u32>,extra:vec4<u32>,earth:vec4<f32>,eye:vec4<f32>,frame:vec4<f32>}
@group(0) @binding(0) var positions:texture_2d<f32>;
@group(0) @binding(1) var<storage,read> atlas:array<Sample>;
@group(0) @binding(2) var<storage,read> records:array<Record>;
@group(0) @binding(3) var<storage,read> links:array<u32>;
@group(0) @binding(4) var<uniform> view:View;
@group(0) @binding(5) var<storage,read> queries:array<vec4<f32>>;
@group(0) @binding(6) var<storage,read_write> result:array<vec4<f32>>;
@group(0) @binding(7) var<storage,read> patches:array<Patch>;
fn triangleHeight(q:vec2<f32>,a:vec3<f32>,b:vec3<f32>,c:vec3<f32>)->vec2<f32>{
 let u=b.xz-a.xz;let v=c.xz-a.xz;let d=q-a.xz;
 let det=u.x*v.y-u.y*v.x;
 if abs(det)<1e-12 {return vec2(0.);}
 let w=vec2(d.x*v.y-d.y*v.x,u.x*d.y-u.y*d.x)/det;
 if any(w<vec2(-.0001))||w.x+w.y>1.0001 {return vec2(0.);}
 return vec2(a.y+w.x*(b.y-a.y)+w.y*(c.y-a.y),1.);
}
// 数式上の曲面ではなく、実際に描いた三角形の高さへ地点を置く。
fn meshHeight(q:vec2<f32>,owner:u32)->vec2<f32>{
 if view.extra.y!=0u||view.flags.z==4u{return vec2(0.);}
 let r=records[owner];
 for(var j=0u;j<r.draw.y;j++) {
  let part=patches[r.draw.x+j];let origin=r.bounds.xy+part.bounds.xy;
  if any(q<origin-vec2(.01))||any(q>origin+vec2(part.bounds.z+.01)){continue;}
  let n=u32(part.bounds.w);
  let ij=min(vec2<u32>(max(vec2(0.),floor((q-origin)*f32(n)/part.bounds.z))),vec2(n-1u));
  let id=ij.y*(n+1u)+ij.x;
  let ids=array<u32,4>(id,id+1u,id+n+2u,id+n+1u);
  var vertices:array<vec3<f32>,4>;
  // 四頂点を同じ処理で評価し、接合処理を四重に展開させない。
  for(var k=0u;k<r.draw.z;k++){vertices[k]=meshVertex(ids[k],part,owner).position;}
  let a=vertices[0];let b=vertices[1];let c=vertices[2];let d=vertices[3];
  let point=q*view.offset.z;
  let first=triangleHeight(point,a,b,c);
  if first.y>0. {return first;}
  let second=triangleHeight(point,a,c,d);
  if second.y>0. {return second;}
 }
 return vec2(0.);
}
@compute @workgroup_size(64) fn query(@builtin(global_invocation_id)index:vec3<u32>){
 let id=index.x;if id>=arrayLength(&queries){return;}
 let q=queries[id];var owner=0xffffffffu;var z=0u;var side=1e30;
 for(var i=0u;i<view.extra.x;i++){
  let r=records[i];if r.draw.y>0u&&inside(q.xy,r)&&(r.data.z>z||(r.data.z==z&&r.bounds.z<side)){owner=i;z=r.data.z;side=r.bounds.z;}
 }
 var height=2.;
 if owner!=0xffffffffu{
  let mesh=meshHeight(q.xy,owner);
  height=select(surface(q.xy,owner).x,mesh.x,mesh.y>0.)+2.;
 }
 let clip=view.matrix*vec4(earthLocal(q.xy,height),1.);
 if clip.w<=0.||clip.z<0.||clip.z>clip.w{result[id]=vec4(0.);return;}
 let uv=vec2(clip.x/clip.w*.5+.5,.5-clip.y/clip.w*.5);
 if any(uv<vec2(0.))||any(uv>=vec2(1.)){result[id]=vec4(uv,height,0.);return;}
 let size=vec2<i32>(textureDimensions(positions));let center=vec2<i32>(uv*vec2<f32>(size));
 var visible=false;
 // 稜線のピクセル境界で山頂自身を誤って隠さないよう、隣接ピクセルも確認する。
 for(var y=-1;y<=1;y++){for(var x=-1;x<=1;x++){
  let pixel=clamp(center+vec2(x,y),vec2(0),size-vec2(1));let terrain=textureLoad(positions,pixel,0);
  if terrain.w==0.{visible=true;continue;}
  let terrainClip=view.matrix*vec4(earthLocal(terrain.xy,abs(terrain.w)-10000.),1.);
  if clip.w<=terrainClip.w+max(.75,clip.w*.00001){visible=true;}
 }}
 result[id]=vec4(uv,height,select(0.,1.,visible));
}
fn earthLocal(q:vec2<f32>,h:f32)->vec3<f32>{return earthPosition(q,h,view.earth,view.offset.z);}

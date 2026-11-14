// 描画と同じ曲面から地点を持ち上げ、実際の描画位置で遮蔽を判定する。
struct Sample {original:vec4<f32>,adjusted:vec4<f32>}
struct Record {bounds:vec4<f32>,data:vec4<u32>,neighbors:vec4<u32>}
struct View {matrix:mat4x4<f32>,offset:vec4<f32>,settings:vec4<f32>,flags:vec4<u32>,extra:vec4<u32>}
@group(0) @binding(0) var positions:texture_2d<f32>;
@group(0) @binding(1) var<storage,read> atlas:array<Sample>;
@group(0) @binding(2) var<storage,read> records:array<Record>;
@group(0) @binding(3) var<storage,read> links:array<u32>;
@group(0) @binding(4) var<uniform> view:View;
@group(0) @binding(5) var<storage,read> queries:array<vec4<f32>>;
@group(0) @binding(6) var<storage,read_write> result:array<vec4<f32>>;
@compute @workgroup_size(64) fn query(@builtin(global_invocation_id)index:vec3<u32>){
 let id=index.x;if id>=arrayLength(&queries){return;}
 let q=queries[id];var owner=0xffffffffu;var z=0u;var side=1e30;
 for(var i=0u;i<view.extra.x;i++){
  let r=records[i];if inside(q.xy,r)&&(r.data.z>z||(r.data.z==z&&r.bounds.z<side)){owner=i;z=r.data.z;side=r.bounds.z;}
 }
 var height=2.;
 if owner!=0xffffffffu{height=surface(q.xy,owner).x+2.;}
 let clip=view.matrix*vec4(q.x*view.offset.z,height,q.y*view.offset.z,1.);
 if clip.w<=0.||clip.z<0.||clip.z>clip.w{result[id]=vec4(0.);return;}
 let uv=vec2(clip.x/clip.w*.5+.5,.5-clip.y/clip.w*.5);
 if any(uv<vec2(0.))||any(uv>=vec2(1.)){result[id]=vec4(uv,height,0.);return;}
 let size=vec2<i32>(textureDimensions(positions));let center=vec2<i32>(uv*vec2<f32>(size));
 var visible=false;
 // 稜線のピクセル境界で山頂自身を誤って隠さないよう、隣接ピクセルも確認する。
 for(var y=-1;y<=1;y++){for(var x=-1;x<=1;x++){
  let pixel=clamp(center+vec2(x,y),vec2(0),size-vec2(1));let terrain=textureLoad(positions,pixel,0);
  if terrain.w==0.{visible=true;continue;}
  let terrainClip=view.matrix*vec4(terrain.x*view.offset.z,abs(terrain.w)-10000.,terrain.y*view.offset.z,1.);
  if clip.w<=terrainClip.w+max(.75,clip.w*.00001){visible=true;}
 }}
 result[id]=vec4(uv,height,select(0.,1.,visible));
}

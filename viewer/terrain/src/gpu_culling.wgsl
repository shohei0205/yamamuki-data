struct View {matrix:mat4x4<f32>,offset:vec4<f32>,settings:vec4<f32>,flags:vec4<u32>,extra:vec4<u32>,earth:vec4<f32>,eye:vec4<f32>,frame:vec4<f32>}
struct Patch {bounds:vec4<f32>,edges:vec4<f32>,origins:vec4<f32>,lines:vec4<f32>}
struct Record {bounds:vec4<f32>,data:vec4<u32>,neighbors:vec4<u32>,tile:vec4<f32>,tile_end:vec4<f32>,draw:vec4<u32>,transition:vec4<u32>,blend_data:vec4<u32>}
struct Arguments {index_count:u32,instance_count:atomic<u32>,first_index:u32,base_vertex:i32,first_instance:u32}
@group(0) @binding(0) var<uniform> view:View;
@group(0) @binding(1) var<storage,read> patches:array<Patch>;
@group(0) @binding(2) var<storage,read> records:array<Record>;
@group(0) @binding(3) var<storage,read> links:array<u32>;
@group(0) @binding(4) var<storage,read> heights:array<vec4<f32>>;
@group(0) @binding(5) var<storage,read_write> visible:array<Patch>;
@group(0) @binding(6) var<storage,read_write> arguments:Arguments;
@group(0) @binding(7) var<uniform> params:vec4<u32>;
fn heightBound(id:u32,lo:vec2<f32>,hi:vec2<f32>)->vec2<f32>{
 let r=records[id];let h=heights[id];
 let distance=max(max(r.bounds.xy-lo,hi-(r.bounds.xy+vec2(r.bounds.z))),vec2(0.));
 let pad=dot(h.zw,distance);
 return vec2(h.x-pad,h.y+pad);
}
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) invocation:vec3<u32>){
 let index=invocation.x;if index>=params.y{return;}
 let part=patches[params.x+index];
 // 計画器ですでに判定済み。間接命令の生成と転記だけを行う。
 if params.w!=0u{visible[index]=part;if index==0u{atomicStore(&arguments.instance_count,params.y);}return;}
 let own=records[view.flags.w];
 // 評価領域と接合端点の丸め差も包含し、境界の頂点を落とさない。
 let pad=max(.1,part.bounds.z*1e-5);
 let lo=own.bounds.xy+part.bounds.xy-vec2(pad);
 let hi=own.bounds.xy+part.bounds.xy+vec2(part.bounds.z+pad);
 var h=heightBound(view.flags.w,lo,hi);
 for(var j=0u;j<own.data.w;j++){
  let b=heightBound(links[own.neighbors.x+j],lo,hi);h=vec2(min(h.x,b.x),max(h.y,b.y));
 }
 let mid=(lo+hi)*.5;let height=(h.x+h.y)*.5;
 let center=earthPosition(mid,height,view.earth,view.offset.z);
 // Mercatorの水平長は球面上の弧長の上限。標高幅を加えた球で保守的に包む。
 let horizontal=length(hi-lo)*.5;
 let scale=select(view.offset.z,1.+max(abs(h.x),abs(h.y))/max(view.earth.z,1.),view.earth.w>0.);
 let radius=horizontal*scale+(h.y-h.x)*.5+1.;
 let m=transpose(view.matrix);
 let planes=array<vec4<f32>,6>(m[3]+m[0],m[3]-m[0],m[3]+m[1],m[3]-m[1],m[2],m[3]-m[2]);
 for(var i=0u;i<6u;i++){
  let p=planes[i];if dot(p.xyz,center)+p.w < -radius*length(p.xyz) {return;}
 }
 let slot=atomicAdd(&arguments.instance_count,1u);visible[slot]=part;
}

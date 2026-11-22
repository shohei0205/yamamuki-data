struct Sample { original:vec4<f32>, adjusted:vec4<f32> }
struct Record { bounds:vec4<f32>, data:vec4<u32>, neighbors:vec4<u32>, tile:vec4<f32>, tile_end:vec4<f32>, draw:vec4<u32>, transition:vec4<u32>, blend_data:vec4<u32> }
@group(0) @binding(0) var<storage,read> samples:array<Sample>;
@group(0) @binding(1) var<storage,read> records:array<Record>;
@group(0) @binding(2) var<storage,read_write> heights:array<vec4<f32>>;
var<workgroup> values:array<vec4<f32>,64>;
@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32){
 let record=records[group.x];let count=(record.data.y+1u)*(record.data.y+1u);
 var bound=vec4(1e30,-1e30,0.,0.);
 for(var i=lane;i<count;i+=64u){
  let s=samples[record.data.x+i];
  bound.x=min(bound.x,min(s.original.x,s.adjusted.x));
  bound.y=max(bound.y,max(s.original.x,s.adjusted.x));
  bound.z=max(bound.z,max(abs(s.original.y),abs(s.adjusted.y)));
  bound.w=max(bound.w,max(abs(s.original.z),abs(s.adjusted.z)));
 }
 values[lane]=bound;workgroupBarrier();
 for(var step=32u;step>0u;step/=2u){
  if lane<step {let b=values[lane+step];let a=values[lane];values[lane]=vec4(min(a.x,b.x),max(a.y,b.y),max(a.z,b.z),max(a.w,b.w));}
  workgroupBarrier();
 }
 if lane==0u {heights[group.x]=values[0];}
}

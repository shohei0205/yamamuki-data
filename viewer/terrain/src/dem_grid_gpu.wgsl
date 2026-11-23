// 作業用DEMの画素中心格子。欠損は描画の有効フラグを0にする。
struct Params{bounds:vec4<f32>,lookup:vec4<f32>,counts:vec4<u32>,settings:vec4<f32>}
struct Sample{original:vec4<f32>,adjusted:vec4<f32>}
@group(0) @binding(0) var<storage,read> grid:array<f32>;
@group(0) @binding(4) var<storage,read_write> output:array<Sample>;
@group(0) @binding(5) var<uniform> params:Params;
fn height(x:u32,y:u32)->vec2<f32>{let width=u32(params.lookup.z)+2u;let v=grid[y*width+x];if (bitcast<u32>(v)&0x7f800000u)==0x7f800000u{return vec2(0.);}return vec2(v,1.);}
fn sample(q:vec2<f32>)->vec2<f32>{
 let cells=params.lookup.z;let width=cells+2.;let p=clamp(q/params.settings.z*cells+vec2(0.5),vec2(0.),vec2(width-1.00001));let ij=min(vec2<u32>(floor(p)),vec2(u32(width)-2u));let f=p-vec2<f32>(ij);var h=0.;
 for(var i=0u;i<4u;i++){let dx=i%2u;let dy=i/2u;let w=select(1.-f.x,f.x,dx==1u)*select(1.-f.y,f.y,dy==1u);if w>1e-7{let v=height(ij.x+dx,ij.y+dy);if v.y==0.{return vec2(0.);}h+=v.x*w;}}return vec2(h,1.);
}
@compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) id:vec3<u32>){
 let n=params.counts.x;if id.x>n||id.y>n{return;}let q=mix(params.bounds.xy,params.bounds.zw,vec2<f32>(id.xy)/f32(n))*params.settings.x;let d=params.settings.z/params.lookup.z;
 let c=sample(q);let xp=sample(q+vec2(d,0.));let xm=sample(q-vec2(d,0.));let yp=sample(q+vec2(0.,d));let ym=sample(q-vec2(0.,d));
 let dx=(select(c.x,xp.x,xp.y>0.)-select(c.x,xm.x,xm.y>0.))/(2.*d);let dy=(select(c.x,yp.x,yp.y>0.)-select(c.x,ym.x,ym.y>0.))/(2.*d);let v=vec4(c.x,dx,dy,c.y);let index=id.y*(n+1u)+id.x;output[index].original=v;output[index].adjusted=v;
}

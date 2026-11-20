// ハイトマップから描画用の頂点標高・法線をGPUで生成する。
struct Params{bounds:vec4<f32>,lookup:vec4<f32>,counts:vec4<u32>,settings:vec4<f32>}
struct Sample{original:vec4<f32>,adjusted:vec4<f32>}
@group(0) @binding(0) var<storage,read> grid:array<u32>;
@group(0) @binding(4) var<storage,read_write> output:array<Sample>;
@group(0) @binding(5) var<uniform> params:Params;
fn height(x:u32,y:u32)->vec2<f32>{let width=u32(params.lookup.z+1.+2.*params.lookup.w);let index=y*width+x;let v=(grid[index/2u]>>((index%2u)*16u))&65535u;return vec2(select(params.lookup.x+f32(v)*params.lookup.y,0.,v==65535u),select(1.,0.,v==65535u));}
fn sample(q:vec2<f32>)->vec2<f32>{
 let cells=params.lookup.z;let width=cells+1.+2.*params.lookup.w;
 let p=clamp(q/params.settings.z*cells+vec2(params.lookup.w),vec2(0.),vec2(width-1.));let ij=min(vec2<u32>(floor(p)),vec2(u32(width)-2u));let f=p-vec2<f32>(ij);var h=0.;
 for(var i=0u;i<4u;i++){let dx=i%2u;let dy=i/2u;let w=select(1.-f.x,f.x,dx==1u)*select(1.-f.y,f.y,dy==1u);if w>1e-7{let v=height(ij.x+dx,ij.y+dy);if v.y==0.{return vec2(0.);}h+=v.x*w;}}
 return vec2(h,1.);
}
@compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) id:vec3<u32>){
 let n=params.counts.x;if id.x>n||id.y>n{return;}
 let q=mix(params.bounds.xy,params.bounds.zw,vec2<f32>(id.xy)/f32(n))*params.settings.x;let d=params.settings.z/params.lookup.z;
 let c=sample(q);let xp=sample(q+vec2(d,0.));let xm=sample(q-vec2(d,0.));let yp=sample(q+vec2(0.,d));let ym=sample(q-vec2(0.,d));
 let dx=(select(c.x,xp.x,xp.y>0.)-select(c.x,xm.x,xm.y>0.))/(2.*d);let dy=(select(c.x,yp.x,yp.y>0.)-select(c.x,ym.x,ym.y>0.))/(2.*d);
 let index=id.y*(n+1u)+id.x;let value=vec4(c.x,dx,dy,c.y);output[index].original=value;output[index].adjusted=value;
}

// 高さキャッシュも境界接合もGPU内で処理する。CPUの頂点配列は使わない。
struct Sample { original:vec4<f32>, adjusted:vec4<f32> }
struct Tile { core:vec4<f32>, field:vec4<f32>, data:vec4<u32>, misc:vec4<f32> }
struct Camera { pan:vec4<f32>, pose:vec4<f32>, viewport:vec4<f32>, flags:vec4<u32> }
@group(0) @binding(0) var<storage,read> samples:array<Sample>;
@group(0) @binding(1) var<storage,read> tiles:array<Tile>;
@group(0) @binding(2) var<storage,read> neighbors:array<u32>;
@group(0) @binding(3) var<uniform> camera:Camera;
fn evaluate(q:vec2<f32>,t:Tile)->vec3<f32>{
 let n=t.data.y;let uv=clamp((q-t.field.xy)/(t.field.zw-t.field.xy)*f32(n),vec2<f32>(0.),vec2<f32>(f32(n)));
 let ij=min(vec2<u32>(uv),vec2<u32>(n-1u));let f=uv-vec2<f32>(ij);let base=t.data.x+ij.y*(n+1u)+ij.x;
 let a=samples[base].original.xyz;let b=samples[base+1u].original.xyz;let c=samples[base+n+1u].original.xyz;let d=samples[base+n+2u].original.xyz;
 return mix(mix(a,b,f.x),mix(c,d,f.x),f.y);
}
fn validAt(q:vec2<f32>,t:Tile)->f32{let n=t.data.y;let uv=clamp((q-t.field.xy)/(t.field.zw-t.field.xy)*f32(n),vec2<f32>(0.),vec2<f32>(f32(n)));let ij=min(vec2<u32>(uv),vec2<u32>(n-1u));let base=t.data.x+ij.y*(n+1u)+ij.x;return min(min(samples[base].original.w,samples[base+1u].original.w),min(samples[base+n+1u].original.w,samples[base+n+2u].original.w));}
fn ramp(x:f32)->vec2<f32>{let u=clamp(x,0.,1.);return vec2<f32>(u*u*(3.-2.*u),select(0.,6.*u*(1.-u),x>0.&&x<1.));}
fn blended(q:vec2<f32>,owner:Tile)->vec3<f32>{
 if(camera.flags.x==0u){return evaluate(q,owner);}
 var sum=0.;var h=0.;var g=vec2<f32>(0.);var dw=vec2<f32>(0.);let band=250.;
 for(var i=0u;i<owner.data.w;i+=1u){let t=tiles[neighbors[owner.data.z+i]];
  if(any(q<t.core.xy-vec2<f32>(band))||any(q>t.core.zw+vec2<f32>(band))){continue;}
  let a=ramp((q.x-t.core.x+band)/(2.*band));let b=ramp((t.core.z-q.x+band)/(2.*band));let c=ramp((q.y-t.core.y+band)/(2.*band));let d=ramp((t.core.w-q.y+band)/(2.*band));
  let w=a.x*b.x*c.x*d.x;let gradient=vec2<f32>((a.y*b.x-a.x*b.y)*c.x*d.x,(c.y*d.x-c.x*d.y)*a.x*b.x)/(2.*band);
  if(w<=0.){continue;}let v=evaluate(q,t);sum+=w;h+=w*v.x;g+=w*v.yz+gradient*v.x;dw+=gradient;
 }
 if(sum<1e-10){return evaluate(q,owner);}let height=h/sum;return vec3<f32>(height,(g-height*dw)/sum);
}
fn stitched(q:vec2<f32>,owner:Tile)->vec3<f32>{
 if(camera.flags.x==0u){return evaluate(q,owner);}
 var step=(owner.core.z-owner.core.x)/owner.misc.y;var coarse=owner;var vertical=false;var start=0.;var found=false;
 for(var i=0u;i<owner.data.w;i+=1u){let t=tiles[neighbors[owner.data.z+i]];let s=(t.core.z-t.core.x)/t.misc.y;if(s<=step+.001){continue;}
  if((abs(q.x-t.core.x)<.02||abs(q.x-t.core.z)<.02)&&q.y>=t.core.y-.02&&q.y<=t.core.w+.02){step=s;coarse=t;vertical=true;start=t.core.y;found=true;}
  if((abs(q.y-t.core.y)<.02||abs(q.y-t.core.w)<.02)&&q.x>=t.core.x-.02&&q.x<=t.core.z+.02){step=s;coarse=t;vertical=false;start=t.core.x;found=true;}
 }
 if(found){let coordinate=select(q.x,q.y,vertical);let lo=start+floor((coordinate-start)/step)*step;let a=select(vec2<f32>(lo,q.y),vec2<f32>(q.x,lo),vertical);let b=select(vec2<f32>(lo+step,q.y),vec2<f32>(q.x,lo+step),vertical);return mix(blended(a,coarse),blended(b,coarse),(coordinate-lo)/step);}
 return blended(q,owner);
}
struct Varying { @builtin(position) position:vec4<f32>, @location(0) normal:vec3<f32>, @location(1) height:f32, @location(2) uv:vec2<f32>, @location(3) @interpolate(flat) error:f32, @location(4) valid:f32 }
@vertex fn vertex(@builtin(vertex_index) id:u32,@builtin(instance_index) tileIndex:u32)->Varying {
 let t=tiles[tileIndex];let n=u32(t.misc.y);let corners=array<vec2<u32>,6>(vec2<u32>(0u,0u),vec2<u32>(1u,0u),vec2<u32>(1u,1u),vec2<u32>(0u,0u),vec2<u32>(1u,1u),vec2<u32>(0u,1u));let cell=id/6u;let ij=vec2<u32>(cell%n,cell/n)+corners[id%6u];let uv=vec2<f32>(ij)/f32(n);let xy=mix(t.core.xy,t.core.zw,uv);let field=stitched(xy,t);let p=xy-camera.pan.xy;
 let x=camera.pose.x*p.x-camera.pose.y*p.y;let d=camera.pose.y*p.x+camera.pose.x*p.y;let z=camera.pan.w/1.15470054+camera.pose.w*d-camera.pose.z*(field.x-camera.pan.z);let y=camera.pose.z*d+camera.pose.w*(field.x-camera.pan.z);
 var out:Varying;out.position=vec4<f32>(x*1.7320508,y*camera.viewport.x*1.7320508,1.00001*z-10.0001,z);out.normal=normalize(vec3<f32>(-field.y,-field.z,1.));out.height=field.x;out.uv=uv;out.error=t.misc.x;out.valid=validAt(xy,t);return out;
}
@fragment fn fragment(v:Varying)->@location(0) vec4<f32>{
 if(v.valid<.999){discard;}
 let light=max(dot(normalize(v.normal),normalize(vec3<f32>(-.55,.65,1.))),0.);var base=mix(vec3<f32>(.41,.49,.33),vec3<f32>(.65,.62,.47),smoothstep(900.,3800.,v.height));
 if(camera.flags.z==1u){let t=log2(v.error/12.5)/6.;base=mix(vec3<f32>(.2,.5,.9),vec3<f32>(.95,.4,.15),t);}
 if(camera.flags.y==1u){let edge=min(v.uv,vec2<f32>(1.)-v.uv)/max(fwidth(v.uv),vec2<f32>(1e-5));if(min(edge.x,edge.y)<1.){base*=.25;}}
 return vec4<f32>(base*(.39+.70*light),1.);
}
// 検証用。通常描画ではこのバッファーの生成・読み戻しは行わない。
@group(0) @binding(4) var<storage,read> checks:array<vec4<f32>>;
@group(0) @binding(5) var<storage,read_write> checked:array<vec4<f32>>;
@compute @workgroup_size(64)
fn checkSeams(@builtin(global_invocation_id) id:vec3<u32>){
 if(id.x>=arrayLength(&checks)){return;}let q=checks[id.x];checked[id.x]=vec4<f32>(stitched(q.xy,tiles[u32(q.z)]),1.);
}

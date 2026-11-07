struct Sample { original: vec4<f32>, adjusted: vec4<f32> }
struct View { matrix: mat4x4<f32>, offset: vec4<f32>, settings: vec4<f32>, flags: vec4<u32>, extra:vec4<u32> }
struct Patch { bounds:vec4<f32>, edges:vec4<f32>, origins:vec4<f32> }
struct Record { bounds:vec4<f32>, data:vec4<u32>, neighbors:vec4<u32> }
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
   return textureSampleLevel(tex,landcoverSampler,uv,0.);
  }
 }
 return vec4(0.);
}

fn landcoverColor(p:vec3<f32>,fallback:vec3<f32>)->vec3<f32>{
 let q=p.xz/view.offset.z;
 let blend=landcover.settings.x;
 let next=landcoverSample(landcoverDetail,landcover.areas[1],q);
 if blend>=1. && next.a>=1.{ return next.rgb; }
 let overview=landcoverSample(landcoverOverview,landcover.areas[0],q);
 let base=fallback*(1.-overview.a)+overview.rgb;
 let current=base*(1.-next.a)+next.rgb;
 if blend>=1.{ return current; }
 let old=landcoverSample(landcoverPrevious,landcover.areas[2],q);
 return mix(base*(1.-old.a)+old.rgb,current,blend);
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
fn band(r:Record)->f32{
 return min(r.bounds.z/8.,max(1.,2.*r.bounds.z/f32(r.data.y)));
}
fn blend(q:vec2<f32>,owner:u32,old:bool)->vec3<f32>{
 let own=records[owner];
 if view.extra.z==0u{return sampleAt(q,owner);}
 var sum=vec3(0.);var weight=0.;
 for(var j=0u;j<own.data.w;j++){
  let id=links[own.neighbors.x+j];
  if (id>=view.extra.x)!=old{continue;}
  let r=records[id];
  // A resident root is a hole filler; never blend it into a loaded finer tile.
  if !old&&r.bounds.w>0.&&own.bounds.w==0.{continue;}
  let pad=band(r);
  let distance=max(max(r.bounds.x-q.x,q.x-r.bounds.x-r.bounds.z),max(r.bounds.y-q.y,q.y-r.bounds.y-r.bounds.z));
  if distance>=pad{continue;}
  let w=smoothstep(-pad,pad,-distance);
  sum+=sampleAt(q,id)*w;weight+=w;
 }
 if weight>1e-6{return sum/weight;}
 return sampleAt(q,owner);
}
fn surface(q:vec2<f32>,owner:u32)->vec3<f32>{
 let fresh=blend(q,owner,false);
 if view.settings.w>=1.{return fresh;}
 let own=records[owner];
 var prior=0xffffffffu;var finest=0u;
 for(var j=0u;j<own.data.w;j++){
  let id=links[own.neighbors.x+j];
  if id<view.extra.x{continue;}
  let r=records[id];
  if inside(q,r)&&r.data.z>=finest{prior=id;finest=r.data.z;}
 }
 // First loading has no preceding surface: show the new field immediately.
 if prior==0xffffffffu{return fresh;}
 return mix(blend(q,prior,true),fresh,view.settings.w);
}
@vertex fn vs(@builtin(vertex_index) id:u32,@builtin(instance_index) instance:u32)->Varying{
 let part=patches[instance];let n=u32(part.bounds.w);
 let ij=vec2(id%(n+1u),id/(n+1u));
 let corner=(ij.x+ij.y)%3u;
 var xy=part.bounds.xy+vec2<f32>(ij)*part.bounds.z/f32(n);
 var stepX=0.;var originX=0.;var stepY=0.;var originY=0.;
 for(var e=0u;e<4u;e++){
  let hit=select(select(ij.y==0u,ij.y==n,e==3u),select(ij.x==0u,ij.x==n,e==1u),e<2u);
  if hit{
   if e<2u{if part.edges[e]>stepY{stepY=part.edges[e];originY=part.origins[e];}}
   else{if part.edges[e]>stepX{stepX=part.edges[e];originX=part.origins[e];}}
  }
 }
 if stepX>0.{xy.x=originX+round((xy.x-originX)/stepX)*stepX;}
 if stepY>0.{xy.y=originY+round((xy.y-originY)/stepY)*stepY;}
 let q=records[view.flags.w].bounds.xy+xy;
 let field=surface(q,view.flags.w);
 let p=vec3(view.offset.x+xy.x*view.offset.z,field.x,view.offset.y+xy.y*view.offset.z);
 var out:Varying;out.clip=view.matrix*vec4(p,1.);out.position=p;
 out.normal=normalize(vec3(-field.y/view.offset.z,1.,-field.z/view.offset.z));
 out.height=field.x;out.uv=xy/view.settings.x;
 out.bary=vec3(select(0.,1.,corner%3u==0u),select(0.,1.,corner%3u==1u),select(0.,1.,corner%3u==2u));return out;
}
fn linear(c:vec3<f32>)->vec3<f32>{
 return select(pow((c+vec3(.055))/vec3(1.055),vec3(2.4)),c/vec3(12.92),c<=vec3(.04045));
}
@fragment fn fs(v:Varying)->@location(0) vec4<f32>{
 var normal=normalize(v.normal);
 if(view.flags.y==0u){normal=normalize(cross(dpdx(v.position),dpdy(v.position)));if(normal.y<0.){normal=-normal;}}
 let light=normalize(vec3(-.55,1.,.65));
 let flat=.39+.70*light.y;
 let shade=clamp(flat+.70*(max(dot(normal,light),0.)-light.y)*view.settings.z,.08,1.3);
 var base=landcoverColor(v.position,mix(vec3(.41,.49,.33),vec3(.65,.62,.47),smoothstep(900.,3800.,v.height)));
 if (view.extra.w&2u)!=0u{
  let errors=array<f32,8>(500.,400.,300.,250.,200.,125.,60.,25.);
  let z=log2(errors[records[view.flags.w].data.z-6u]/12.5)/6.;
  base=mix(vec3(.2,.5,.9),vec3(.95,.4,.15),z);
 }
 var color=clamp(base*shade,vec3(0.),vec3(1.));
 let width=max(fwidth(v.bary),vec3(.0001));
 let edge=min(min(v.bary.x/width.x,v.bary.y/width.y),v.bary.z/width.z);
 if view.flags.z>0u&&view.flags.z<=2u{
  if view.flags.z==2u&&edge>1.{discard;}
  color=mix(vec3(.035,.055,.04),color,smoothstep(.15,.45,edge));
 }
 if (view.extra.w&1u)!=0u{
  let d=min(v.uv,vec2(1.)-v.uv)/max(fwidth(v.uv),vec2(.000001));
  color*=mix(.25,1.,clamp(min(d.x,d.y),0.,1.));
 }
 return vec4(linear(color),1.);
}

@vertex fn control_vs(@builtin(vertex_index)id:u32)->Varying{
 let v=controls[triangles[id]];
 let p=vec3(view.offset.x+v.x*view.offset.z,v.z,view.offset.y+v.y*view.offset.z);
 var out:Varying;out.clip=view.matrix*vec4(p,1.);out.position=p;out.height=v.z;out.normal=vec3(0.,1.,0.);
 out.uv=v.xy/view.settings.x;out.bary=vec3(select(0.,1.,id%3u==0u),select(0.,1.,id%3u==1u),select(0.,1.,id%3u==2u));return out;
}
@fragment fn control_fs(v:Varying)->@location(0)vec4<f32>{
 if any(v.uv<vec2(0.))||any(v.uv>vec2(1.)){discard;}
 var normal=normalize(cross(dpdx(v.position),dpdy(v.position)));if normal.y<0.{normal=-normal;}
 let light=normalize(vec3(-.55,1.,.65));
 let flat=.39+.70*light.y;
 let shade=clamp(flat+.70*(max(dot(normal,light),0.)-light.y)*view.settings.z,.08,1.3);
 let base=landcoverColor(v.position,mix(vec3(.41,.49,.33),vec3(.65,.62,.47),smoothstep(900.,3800.,v.height)));
 var color=clamp(base*shade,vec3(0.),vec3(1.));
 let width=max(fwidth(v.bary),vec3(.0001));
 let edge=min(min(v.bary.x/width.x,v.bary.y/width.y),v.bary.z/width.z);
 if view.extra.y==0u{
  if edge>1.{discard;}color=vec3(.06,.12,.18);
 }else if view.flags.z>0u{
  if (view.flags.z==2u||view.flags.z==4u)&&edge>1.{discard;}
  color=mix(vec3(.035,.055,.04),color,smoothstep(.15,.45,edge));
 }
 return vec4(linear(color),1.);
}

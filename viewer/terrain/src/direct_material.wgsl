struct Params { area:vec4<f32>, uv0:vec4<f32>, uv1:vec4<f32>, tint:vec4<f32>, settings:vec4<f32>, earth:vec4<f32>, eye:vec4<f32>, gainUV0:vec4<f32>, gainUV1:vec4<f32>, visibility:vec4<f32> }
@group(0) @binding(0) var positions:texture_2d<f32>;
@group(0) @binding(1) var oldTile:texture_2d<f32>;
@group(0) @binding(2) var newTile:texture_2d<f32>;
@group(1) @binding(0) var linearSampler:sampler;
@group(0) @binding(4) var<uniform> params:Params;
@group(0) @binding(5) var lightingSource:texture_2d<f32>;
@group(0) @binding(6) var oldGain:texture_2d<f32>;
@group(0) @binding(7) var newGain:texture_2d<f32>;
// R32係数にはフィルター機能を要求せず、周辺4画素を補間する。
fn surface_gain(image:texture_2d<f32>,uv:vec2<f32>,nearest:bool)->f32 {
 let size=vec2<f32>(textureDimensions(image));let maximum=vec2<i32>(textureDimensions(image))-vec2(1);
 if nearest {return textureLoad(image,clamp(vec2<i32>(floor(uv*size)),vec2(0),maximum),0).r;}
 let p=uv*size-.5;let anchor=vec2<i32>(floor(p));let t=fract(p);
 let a=textureLoad(image,clamp(anchor,vec2(0),maximum),0).r;
 let b=textureLoad(image,clamp(anchor+vec2(1,0),vec2(0),maximum),0).r;
 let c=textureLoad(image,clamp(anchor+vec2(0,1),vec2(0),maximum),0).r;
 let d=textureLoad(image,clamp(anchor+vec2(1,1),vec2(0),maximum),0).r;
 return mix(mix(a,b,t.x),mix(c,d,t.x),t.y);
}
@vertex fn vs(@builtin(vertex_index)i:u32)->@builtin(position)vec4<f32>{
 let p=array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.)); return vec4(p[i],0.,1.);
}
@group(2) @binding(0) var<storage,read> materialRectangles:array<vec4<u32>>;
@vertex fn tile_vs(@builtin(vertex_index)i:u32,@builtin(instance_index)command:u32)->@builtin(position)vec4<f32>{
 if (u32(params.settings.w)&32u)==0u {
  let p=array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));return vec4(p[i],params.visibility.x,1.);
 }
 let rect=materialRectangles[command];
 if any(rect.zw<=rect.xy) {return vec4(-2.,-2.,0.,1.);}
 let corner=array<vec2<f32>,6>(vec2(0.,0.),vec2(1.,0.),vec2(0.,1.),vec2(0.,1.),vec2(1.,0.),vec2(1.,1.));
 let pixel=mix(vec2<f32>(rect.xy),vec2<f32>(rect.zw),corner[i]);
 let clip=pixel/vec2<f32>(textureDimensions(positions))*2.-1.;
 return vec4(clip.x,-clip.y,params.visibility.x,1.);
}
fn linear(c:vec3<f32>)->vec3<f32>{return select(pow((c+vec3(.055))/1.055,vec3(2.4)),c/12.92,c<=vec3(.04045));}
fn gamma(c:vec3<f32>)->vec3<f32>{return select(1.055*pow(max(c,vec3(0.)),vec3(1./2.4))-.055,c*12.92,c<vec3(.0031308));}
fn coordinate(p:vec4<f32>)->vec2<f32>{
 let pixel=clamp(vec2<i32>(p.xy),vec2(0),vec2<i32>(textureDimensions(positions))-vec2(1));
 let q=textureLoad(positions,pixel,0);
 if q.w<=0. {discard;}
 let uv=(q.xy-params.area.xy)/params.area.zw;
 if any(uv<vec2(0.))||any(uv>=vec2(1.)){discard;}
 return uv;
}
// 透明な隣接余白を読む可能性がある端3画素は遮蔽元にしない。
@fragment fn occlusion(@builtin(position)p:vec4<f32>)->@builtin(frag_depth)f32 {
 let uv=coordinate(p);
 let oldMargin=vec2(3.)/max(vec2<f32>(textureDimensions(oldTile))*params.uv0.zw,vec2(1.));
 let newMargin=vec2(3.)/max(vec2<f32>(textureDimensions(newTile))*params.uv1.zw,vec2(1.));
 let margin=max(oldMargin,newMargin);
 if any(uv<margin)||any(uv>vec2(1.)-margin){discard;}
 return params.visibility.x;
}
fn material_filter()->u32 {
 let flags=u32(params.settings.w);
 return ((flags>>3u)&3u)|(((flags>>6u)&3u)<<2u);
}
@fragment fn color(@builtin(position)p:vec4<f32>)->@location(0)vec4<f32>{
 let uv=coordinate(p);
 if (u32(params.settings.w)&4u)!=0u {
  let tileUV=params.uv0.xy+uv*params.uv0.zw;
  let edge=min(tileUV,vec2(1.)-tileUV)/max(fwidth(tileUV),vec2(1e-8));
  if min(edge.x,edge.y)>1.75 {discard;}
  return vec4(linear(params.tint.rgb),params.tint.a);
 }
 if (u32(params.settings.w)&1u)!=0u {return vec4(linear(params.tint.rgb),params.tint.a);}
 let oldUV=params.uv0.xy+uv*params.uv0.zw;
 let newUV=params.uv1.xy+uv*params.uv1.zw;
 let oldFootprint=edge_footprint(oldTile,oldUV);
 let newFootprint=edge_footprint(newTile,newUV);
 let pair=edge_resample_pair(oldTile,newTile,linearSampler,oldUV,newUV,material_filter(),oldFootprint,newFootprint,params.settings.x);
 let a=pair.old_color;let b=pair.new_color;
 if (u32(params.settings.w)&256u)!=0u {
  if params.tint.a<=0. {discard;}
  let flags=u32(params.settings.w);
  let ca=vec4(gamma(a.rgb),a.a)*params.settings.y;
  let cb=vec4(gamma(b.rgb),b.a)*params.settings.z;
  var ga=1.;var gb=1.;
  if (flags&512u)!=0u {ga=surface_gain(oldGain,params.gainUV0.xy+uv*params.gainUV0.zw,material_filter()==2u);}
  if (flags&1024u)!=0u {gb=surface_gain(newGain,params.gainUV1.xy+uv*params.gainUV1.zw,material_filter()==2u);}
  let foreground=mix(ca,cb,params.settings.x);
  let factor=mix((1.-ca.a)*ga,(1.-cb.a)*gb,params.settings.x);
  let background=textureLoad(lightingSource,vec2<i32>(p.xy),0);
  let rgb=clamp(foreground.rgb+factor*gamma(background.rgb),vec3(0.),vec3(1.));
  return vec4(linear(rgb),foreground.a+(1.-foreground.a)*background.a);
 }

 var c=mix(vec4(gamma(a.rgb),a.a)*params.settings.y,vec4(gamma(b.rgb),b.a)*params.settings.z,params.settings.x);
 c*=params.tint;
 return vec4(linear(c.rgb),c.a);
}
fn difference(p:vec4<f32>)->f32{
 let uv=coordinate(p);
 let oldUV=params.uv0.xy+uv*params.uv0.zw;
 let newUV=params.uv1.xy+uv*params.uv1.zw;
 let oldFootprint=edge_footprint(oldTile,oldUV);
 let newFootprint=edge_footprint(newTile,newUV);
 let pair=edge_resample_pair(oldTile,newTile,linearSampler,oldUV,newUV,material_filter(),oldFootprint,newFootprint,params.settings.x);
 let a=pair.old_color.rg*params.settings.y;let b=pair.new_color.rg*params.settings.z;
 let difference=mix(a.y-a.x,b.y-b.x,params.settings.x);
 if (u32(params.settings.w)&2u)!=0u {return pow(max(1.+difference,0.),2.2)-1.;}
 return difference;
}
@fragment fn shade(@builtin(position)p:vec4<f32>)->@location(0)vec4<f32>{return vec4(vec3(clamp(1.+difference(p),0.,2.)*.5),0.);}
@fragment fn lighting(@builtin(position)p:vec4<f32>)->@location(0)vec4<f32>{
 let pixel=vec2<i32>(p.xy);
 let q=textureLoad(positions,pixel,0); if q.w==0.{discard;}
 let color=textureLoad(lightingSource,pixel,0);
 var shaded=linear(clamp(gamma(color.rgb)*q.z,vec3(0.),vec3(1.)));
 if params.earth.w>0. {
  let point=earthPosition(q.xy,abs(q.w)-10000.,params.earth,params.eye.w);
  shaded=airHaze(shaded,params.eye.xyz,point,params.earth.z);
 }
 return vec4(shaded,color.a);
}

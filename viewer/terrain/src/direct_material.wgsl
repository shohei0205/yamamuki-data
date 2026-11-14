struct Params { area:vec4<f32>, uv0:vec4<f32>, uv1:vec4<f32>, tint:vec4<f32>, settings:vec4<f32> }
@group(0) @binding(0) var positions:texture_2d<f32>;
@group(0) @binding(1) var oldTile:texture_2d<f32>;
@group(0) @binding(2) var newTile:texture_2d<f32>;
@group(1) @binding(0) var linearSampler:sampler;
@group(0) @binding(4) var<uniform> params:Params;
@group(0) @binding(5) var lightingSource:texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index)i:u32)->@builtin(position)vec4<f32>{
 let p=array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.)); return vec4(p[i],0.,1.);
}
fn linear(c:vec3<f32>)->vec3<f32>{return select(pow((c+vec3(.055))/1.055,vec3(2.4)),c/12.92,c<=vec3(.04045));}
fn gamma(c:vec3<f32>)->vec3<f32>{return select(1.055*pow(max(c,vec3(0.)),vec3(1./2.4))-.055,c*12.92,c<vec3(.0031308));}
fn coordinate(p:vec4<f32>)->vec2<f32>{
 let q=textureLoad(positions,vec2<i32>(p.xy),0);
 if q.w<=0. {discard;}
 let uv=(q.xy-params.area.xy)/params.area.zw;
 if any(uv<vec2(0.))||any(uv>=vec2(1.)){discard;}
 return uv;
}
@fragment fn color(@builtin(position)p:vec4<f32>)->@location(0)vec4<f32>{
 let uv=coordinate(p);
 if (u32(params.settings.w)&1u)!=0u {return vec4(linear(params.tint.rgb),params.tint.a);}
 var a=textureSampleLevel(oldTile,linearSampler,params.uv0.xy+uv*params.uv0.zw,0.);
 var b=textureSampleLevel(newTile,linearSampler,params.uv1.xy+uv*params.uv1.zw,0.);
 a=vec4(gamma(a.rgb),a.a)*params.settings.y; b=vec4(gamma(b.rgb),b.a)*params.settings.z;
 let c=mix(a,b,params.settings.x)*params.tint;
 return vec4(linear(c.rgb),c.a);
}
fn difference(p:vec4<f32>)->f32{
 let uv=coordinate(p);
 let a=textureSampleLevel(oldTile,linearSampler,params.uv0.xy+uv*params.uv0.zw,0.).rg*params.settings.y;
 let b=textureSampleLevel(newTile,linearSampler,params.uv1.xy+uv*params.uv1.zw,0.).rg*params.settings.z;
 let difference=mix(a.y-a.x,b.y-b.x,params.settings.x);
 if (u32(params.settings.w)&2u)!=0u {return pow(max(1.+difference,0.),2.2)-1.;}
 return difference;
}
@fragment fn darken(@builtin(position)p:vec4<f32>)->@location(0)vec4<f32>{return vec4(vec3(max(-difference(p),0.)),0.);}
@fragment fn brighten(@builtin(position)p:vec4<f32>)->@location(0)vec4<f32>{return vec4(vec3(max(difference(p),0.)),0.);}
@fragment fn lighting(@builtin(position)p:vec4<f32>)->@location(0)vec4<f32>{
 let pixel=vec2<i32>(p.xy);
 let q=textureLoad(positions,pixel,0); if q.w==0.{discard;}
 let color=textureLoad(lightingSource,pixel,0);
 return vec4(linear(clamp(gamma(color.rgb)*q.z,vec3(0.),vec3(1.))),color.a);
}

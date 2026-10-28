// 頂点・indexバッファーなし。頂点IDから格子の三角形を生成する。
struct Sample { original:vec4<f32>, adjusted:vec4<f32> }
struct Camera { pan:vec4<f32>, pose:vec4<f32>, viewport:vec4<f32>, grid:vec4<u32>, bounds:vec4<f32> }
@group(0) @binding(0) var<storage,read> data:array<Sample>;
@group(0) @binding(1) var<uniform> camera:Camera;
struct Varying { @builtin(position) position:vec4<f32>, @location(0) normal:vec3<f32>, @location(1) height:f32, @location(2) valid:f32 }
@vertex fn vertex(@builtin(vertex_index) id:u32)->Varying {
 let n=camera.grid.x;let corners=array<vec2<u32>,6>(vec2<u32>(0u,0u),vec2<u32>(1u,0u),vec2<u32>(1u,1u),vec2<u32>(0u,0u),vec2<u32>(1u,1u),vec2<u32>(0u,1u));let cell=id/6u;let ij=vec2<u32>(cell%n,cell/n)+corners[id%6u];let record=data[ij.y*(n+1u)+ij.x];let field=select(record.original,record.adjusted,camera.grid.y==1u);let xy=mix(camera.bounds.xy,camera.bounds.zw,vec2<f32>(ij)/f32(n))*camera.viewport.z;let p=xy-camera.pan.xy;
 let x=camera.pose.x*p.x-camera.pose.y*p.y;let d=camera.pose.y*p.x+camera.pose.x*p.y;let z=camera.pan.w/1.15470054+camera.pose.w*d-camera.pose.z*(field.x-camera.pan.z);let y=camera.pose.z*d+camera.pose.w*(field.x-camera.pan.z);
 var out:Varying;out.position=vec4<f32>(x*1.7320508,y*camera.viewport.x*1.7320508,1.00001*z-10.0001,z);out.normal=normalize(vec3<f32>(-field.y,-field.z,1.));out.height=field.x;out.valid=field.w;return out;
}
@fragment fn fragment(v:Varying)->@location(0) vec4<f32>{
 if(v.valid<.999){return vec4<f32>(1.,0.,1.,1.);}
 let light=max(dot(normalize(v.normal),normalize(vec3<f32>(-.55,.65,1.))),0.);let base=mix(vec3<f32>(.41,.49,.33),vec3<f32>(.65,.62,.47),smoothstep(900.,3800.,v.height));return vec4<f32>(base*(.39+.70*light),1.);
}

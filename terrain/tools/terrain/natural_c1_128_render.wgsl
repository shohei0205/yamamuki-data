// 高さキャッシュも境界接合もGPU内で処理する。CPUの頂点配列は使わない。
struct Sample { original:vec4<f32>, adjusted:vec4<f32> }
struct Tile { core:vec4<f32>, field:vec4<f32>, data:vec4<u32>, misc:vec4<f32> }
struct Patch { bounds:vec4<f32>, data:vec4<u32>, edges:vec4<f32> }
@group(0) @binding(6) var<storage,read> patches:array<Patch>;
struct Camera { pan:vec4<f32>, pose:vec4<f32>, viewport:vec4<f32>, flags:vec4<u32> }
@group(0) @binding(0) var<storage,read> samples:array<Sample>;
@group(0) @binding(1) var<storage,read> tiles:array<Tile>;
@group(0) @binding(2) var<storage,read> neighbors:array<u32>;
@group(0) @binding(3) var<uniform> camera:Camera;
@group(0) @binding(7) var<storage,read> morphSamples:array<Sample>;
fn emptyTile(t:Tile)->bool{return (u32(t.misc.w)&1u)!=0u;}
fn provisionalTile(t:Tile)->bool{return (u32(t.misc.w)&2u)!=0u;}
fn trustedTile(t:Tile)->bool{return (u32(t.misc.w)&4u)!=0u;}
fn evaluate(q:vec2<f32>,t:Tile)->vec3<f32>{
 if(emptyTile(t)){return vec3<f32>(0.);}
 let n=t.data.y;let uv=clamp((q-t.field.xy)/(t.field.zw-t.field.xy)*f32(n),vec2<f32>(0.),vec2<f32>(f32(n)));
 let ij=min(vec2<u32>(uv),vec2<u32>(n-1u));let f=uv-vec2<f32>(ij);let base=t.data.x+ij.y*(n+1u)+ij.x;
 let a=mix(samples[base].original.xyz,samples[base].adjusted.xyz,camera.viewport.y);let b=mix(samples[base+1u].original.xyz,samples[base+1u].adjusted.xyz,camera.viewport.y);let c=mix(samples[base+n+1u].original.xyz,samples[base+n+1u].adjusted.xyz,camera.viewport.y);let d=mix(samples[base+n+2u].original.xyz,samples[base+n+2u].adjusted.xyz,camera.viewport.y);
 return mix(mix(a,b,f.x),mix(c,d,f.x),f.y);
}
fn oldSample(q:vec2<f32>,t:Tile)->vec3<f32>{
 let n=t.data.y;let uv=clamp((q-t.field.xy)/(t.field.zw-t.field.xy)*f32(n),vec2<f32>(0.),vec2<f32>(f32(n)));let ij=min(vec2<u32>(uv),vec2<u32>(n-1u));let f=uv-vec2<f32>(ij);let base=u32(t.misc.z)+ij.y*(n+1u)+ij.x;
 return mix(mix(morphSamples[base].original.xyz,morphSamples[base+1u].original.xyz,f.x),mix(morphSamples[base+n+1u].original.xyz,morphSamples[base+n+2u].original.xyz,f.x),f.y);
}
fn ramp(x:f32)->vec2<f32>{let u=clamp(x,0.,1.);return vec2<f32>(u*u*(3.-2.*u),select(0.,6.*u*(1.-u),x>0.&&x<1.));}
fn tileBlendBand(t:Tile)->f32{
 let padding=min(min(t.core.x-t.field.x,t.core.y-t.field.y),min(t.field.z-t.core.z,t.field.w-t.core.w));
 let step=(t.core.z-t.core.x)/max(t.misc.y,1.);
 return min(max(250.,2.*step),max(1.,min(padding,(t.core.z-t.core.x)*.125)));
}
fn fieldAt(q:vec2<f32>,t:Tile,old:bool)->vec3<f32>{if(old){return oldSample(q,t);}return evaluate(q,t);}
fn blendFields(q:vec2<f32>,owner:Tile,old:bool)->vec3<f32>{
 if(camera.flags.x==0u || owner.data.w<=1u){return fieldAt(q,owner,old);}
 // 仮表示は詳細地形の内側を引き下げない。詳細側の外縁でのみ
 // 仮表示の重みを滑らかに戻し、高さとその勾配を同じ式で接合する。
 var sum=0.;var h=0.;var g=vec2<f32>(0.);var dw=vec2<f32>(0.);
 var fallbackSum=0.;var fallbackH=0.;var fallbackG=vec2<f32>(0.);var fallbackDW=vec2<f32>(0.);
 var remain=1.;var remainGradient=vec2<f32>(0.);
 for(var i=0u;i<owner.data.w;i+=1u){let t=tiles[neighbors[owner.data.z+i]];let band=tileBlendBand(t);
  if(any(q<t.core.xy-vec2<f32>(band))||any(q>t.core.zw+vec2<f32>(band))){continue;}
  let a=ramp((q.x-t.core.x+band)/(2.*band));let b=ramp((t.core.z-q.x+band)/(2.*band));let c=ramp((q.y-t.core.y+band)/(2.*band));let d=ramp((t.core.w-q.y+band)/(2.*band));
  let w=a.x*b.x*c.x*d.x;let gradient=vec2<f32>((a.y*b.x-a.x*b.y)*c.x*d.x,(c.y*d.x-c.x*d.y)*a.x*b.x)/(2.*band);
  if(w<=0.){continue;}let v=fieldAt(q,t,old);
  if(provisionalTile(t)){fallbackSum+=w;fallbackH+=w*v.x;fallbackG+=w*v.yz+gradient*v.x;fallbackDW+=gradient;}
  else{
   sum+=w;h+=w*v.x;g+=w*v.yz+gradient*v.x;dw+=gradient;
   if((u32(owner.misc.w)&8u)!=0u){
   let ca=ramp((q.x-t.core.x+band)/band);let cb=ramp((t.core.z-q.x+band)/band);let cc=ramp((q.y-t.core.y+band)/band);let cd=ramp((t.core.w-q.y+band)/band);
   let cover=ca.x*cb.x*cc.x*cd.x;let coverGradient=vec2<f32>((ca.y*cb.x-ca.x*cb.y)*cc.x*cd.x,(cc.y*cd.x-cc.x*cd.y)*ca.x*cb.x)/band;
   remainGradient=remainGradient*(1.-cover)-remain*coverGradient;remain*=1.-cover;
   }
  }
 }
 g+=remain*fallbackG+remainGradient*fallbackH;dw+=remain*fallbackDW+remainGradient*fallbackSum;h+=remain*fallbackH;sum+=remain*fallbackSum;
 if(sum<1e-10){return fieldAt(q,owner,old);}let height=h/sum;return vec3<f32>(height,(g-height*dw)/sum);
}
fn blended(q:vec2<f32>,owner:Tile)->vec3<f32>{return blendFields(q,owner,false);}
fn movingSurface(q:vec2<f32>,owner:Tile)->vec3<f32>{let goal=blended(q,owner);if(camera.viewport.w>=1.){return goal;}return mix(blendFields(q,owner,true),goal,camera.viewport.w);}
fn stitched(q:vec2<f32>,owner:Tile)->vec3<f32>{
 if(camera.flags.x==0u || owner.data.w<=1u){return evaluate(q,owner);}
 var step=(owner.core.z-owner.core.x)/owner.misc.y;var coarse=owner;var vertical=false;var start=0.;var found=false;
 for(var i=0u;i<owner.data.w;i+=1u){let t=tiles[neighbors[owner.data.z+i]];let s=(t.core.z-t.core.x)/t.misc.y;if(s<=step+.001){continue;}
  if((abs(q.x-t.core.x)<.02||abs(q.x-t.core.z)<.02)&&q.y>=t.core.y-.02&&q.y<=t.core.w+.02){step=s;coarse=t;vertical=true;start=t.core.y;found=true;}
  if((abs(q.y-t.core.y)<.02||abs(q.y-t.core.w)<.02)&&q.x>=t.core.x-.02&&q.x<=t.core.z+.02){step=s;coarse=t;vertical=false;start=t.core.x;found=true;}
 }
 if(found){let coordinate=select(q.x,q.y,vertical);let lo=start+floor((coordinate-start)/step)*step;let a=select(vec2<f32>(lo,q.y),vec2<f32>(q.x,lo),vertical);let b=select(vec2<f32>(lo+step,q.y),vec2<f32>(q.x,lo+step),vertical);return mix(blended(a,coarse),blended(b,coarse),(coordinate-lo)/step);}
 return blended(q,owner);
}
fn patchPosition(xy:vec2<f32>,part:Patch)->vec2<f32>{
 // 細かい側の辺を粗い側の辺の直線へ合わせ、T字接続の隙間を防ぐ。
 var edgeStep=0.;var vertical=false;
 if(abs(xy.x-part.bounds.x)<.02){edgeStep=part.edges.x;vertical=true;}
 if(abs(xy.x-part.bounds.z)<.02 && part.edges.y>edgeStep){edgeStep=part.edges.y;vertical=true;}
 if(abs(xy.y-part.bounds.y)<.02 && part.edges.z>edgeStep){edgeStep=part.edges.z;vertical=false;}
 if(abs(xy.y-part.bounds.w)<.02 && part.edges.w>edgeStep){edgeStep=part.edges.w;vertical=false;}
 if(edgeStep<=0.){return xy;}
 let coordinate=select(xy.x,xy.y,vertical);let snapped=round(coordinate/edgeStep)*edgeStep;
 return select(vec2<f32>(snapped,xy.y),vec2<f32>(xy.x,snapped),vertical);
}
fn patchSurface(xy:vec2<f32>,part:Patch)->vec3<f32>{return movingSurface(patchPosition(xy,part),tiles[part.data.x]);}
struct Varying { @builtin(position) position:vec4<f32>, @location(0) normal:vec3<f32>, @location(1) height:f32, @location(2) uv:vec2<f32>, @location(3) @interpolate(flat) error:f32, @location(4) valid:f32, @location(5) @interpolate(linear) bary:vec3<f32> }
@vertex fn vertex(@builtin(vertex_index) id:u32,@builtin(instance_index) tileIndex:u32)->Varying {
 let part=patches[tileIndex];let t=tiles[part.data.x];let n=part.data.y;let corners=array<vec2<u32>,6>(vec2<u32>(0u,0u),vec2<u32>(1u,0u),vec2<u32>(1u,1u),vec2<u32>(0u,0u),vec2<u32>(1u,1u),vec2<u32>(0u,1u));let cell=id/6u;let ij=vec2<u32>(cell%n,cell/n)+corners[id%6u];let patchUV=vec2<f32>(ij)/f32(n);let xy=patchPosition(mix(part.bounds.xy,part.bounds.zw,patchUV),part);let uv=(xy-t.core.xy)/(t.core.zw-t.core.xy);var field=vec3<f32>(0.);
 field=patchSurface(xy,part);
  let p=xy-camera.pan.xy;
 let x=camera.pose.x*p.x+camera.pose.y*p.y;let d=camera.pose.y*p.x-camera.pose.x*p.y;let z=camera.pan.w/1.15470054+camera.pose.w*d-camera.pose.z*(field.x-camera.pan.z);let y=camera.pose.z*d+camera.pose.w*(field.x-camera.pan.z);
 var out:Varying;out.position=vec4<f32>(x*1.7320508,y*camera.viewport.x*1.7320508,1.00001*z-10.0001,z);out.normal=normalize(vec3<f32>(-field.y,-field.z,1.));out.height=field.x;out.uv=uv;out.error=t.misc.x;out.valid=select(1.,0.,emptyTile(t));let bary=array<vec3<f32>,3>(vec3<f32>(1.,0.,0.),vec3<f32>(0.,1.,0.),vec3<f32>(0.,0.,1.));out.bary=bary[id%3u];return out;
}
@fragment fn fragment(v:Varying)->@location(0) vec4<f32>{
 let light=max(dot(normalize(v.normal),normalize(vec3<f32>(-.55,.65,1.))),0.);var base=mix(vec3<f32>(.41,.49,.33),vec3<f32>(.65,.62,.47),smoothstep(900.,3800.,v.height));
 if(camera.flags.z==1u){let t=log2(v.error/12.5)/6.;base=mix(vec3<f32>(.2,.5,.9),vec3<f32>(.95,.4,.15),t);}
 if(camera.flags.y==1u){let edge=min(v.uv,vec2<f32>(1.)-v.uv)/max(fwidth(v.uv),vec2<f32>(1e-5));if(min(edge.x,edge.y)<1.){base*=.25;}}
 var shade=base*(.39+.70*light);let d=v.bary/max(fwidth(v.bary),vec3<f32>(1e-5));let edge=1.-smoothstep(.15,.45,min(d.x,min(d.y,d.z)));
 if(camera.viewport.z==1.){shade=mix(shade,vec3<f32>(.035,.055,.04),edge);}
 if(camera.viewport.z==2.){if(edge<.02){discard;}shade=mix(vec3<f32>(.9,.945,.98),vec3<f32>(.035,.055,.04),edge);}
 if(v.valid<.5){return vec4<f32>(.38,.59,.72,1.);}
 return vec4<f32>(shade,1.);
}
// 検証用。通常描画ではこのバッファーの生成・読み戻しは行わない。
@group(0) @binding(4) var<storage,read> checks:array<vec4<f32>>;
@group(0) @binding(5) var<storage,read_write> checked:array<vec4<f32>>;
@compute @workgroup_size(64)
fn checkSeams(@builtin(global_invocation_id) id:vec3<u32>){
 if(id.x>=arrayLength(&checks)){return;}let q=checks[id.x];checked[id.x]=vec4<f32>(stitched(q.xy,tiles[u32(q.z)]),1.);
}

struct WireOutput { @builtin(position) position:vec4<f32>, @location(0) xy:vec2<f32>, @location(1) @interpolate(flat) owner:u32, @location(2) height:f32 }
@vertex fn wireVertex(@location(0) point:vec3<f32>,@builtin(instance_index) tileIndex:u32)->WireOutput {
 let p=point.xy-camera.pan.xy;let x=camera.pose.x*p.x+camera.pose.y*p.y;let d=camera.pose.y*p.x-camera.pose.x*p.y;let z=camera.pan.w/1.15470054+camera.pose.w*d-camera.pose.z*(point.z-camera.pan.z);let y=camera.pose.z*d+camera.pose.w*(point.z-camera.pan.z);
 var out:WireOutput;out.position=vec4<f32>(x*1.7320508,y*camera.viewport.x*1.7320508,1.00001*z-10.0001,z);out.xy=point.xy;out.owner=tileIndex;out.height=point.z;return out;
}
@fragment fn wireFragment(v:WireOutput)->@location(0) vec4<f32>{let t=tiles[v.owner];if(any(v.xy<t.core.xy)||any(v.xy>t.core.zw)){discard;}return vec4<f32>(.025,.045,.03,1.);}

@fragment fn controlMeshFragment(v:WireOutput)->@location(0) vec4<f32>{
 let t=tiles[v.owner];let p=vec3<f32>(v.xy,v.height);var n=normalize(cross(dpdx(p),dpdy(p)));if(n.z<0.){n=-n;}
 if(any(v.xy<t.core.xy)||any(v.xy>t.core.zw)){discard;}
 let light=max(dot(n,normalize(vec3<f32>(-.55,.65,1.))),0.);var base=mix(vec3<f32>(.41,.49,.33),vec3<f32>(.65,.62,.47),smoothstep(900.,3800.,v.height));
 if(camera.flags.z==1u){base=mix(vec3<f32>(.2,.5,.9),vec3<f32>(.95,.4,.15),log2(t.misc.x/12.5)/6.);}
 return vec4<f32>(base*(.39+.70*light),1.);
}

@compute @workgroup_size(64) fn checkPatchSeams(@builtin(global_invocation_id) id:vec3<u32>){if(id.x>=arrayLength(&checks)){return;}let q=checks[id.x];let part=patches[u32(q.z)];let xy=patchPosition(q.xy,part);var v=movingSurface(xy,tiles[part.data.x]);if(q.w==2.){v=evaluate(xy,tiles[part.data.x]);}checked[id.x]=vec4<f32>(v,select(xy.x,xy.y,q.w==1.));}

// 元データ全画素の監査で標高なしと確認した完成タイルだけを埋める。
@vertex fn emptyVertex(@builtin(vertex_index) id:u32,@builtin(instance_index) tileIndex:u32)->WireOutput{
 let t=tiles[tileIndex];let corners=array<vec2<f32>,6>(vec2<f32>(0.,0.),vec2<f32>(1.,0.),vec2<f32>(1.,1.),vec2<f32>(0.,0.),vec2<f32>(1.,1.),vec2<f32>(0.,1.));let xy=mix(t.core.xy,t.core.zw,corners[id]);let p=xy-camera.pan.xy;let x=camera.pose.x*p.x+camera.pose.y*p.y;let d=camera.pose.y*p.x-camera.pose.x*p.y;let h=-camera.pan.z;let z=camera.pan.w/1.15470054+camera.pose.w*d-camera.pose.z*h;let y=camera.pose.z*d+camera.pose.w*h;
 var out:WireOutput;out.position=vec4<f32>(x*1.7320508,y*camera.viewport.x*1.7320508,1.00001*z-10.0001,z);out.xy=xy;out.owner=tileIndex;out.height=0.;return out;
}
@fragment fn emptyFragment(v:WireOutput)->@location(0) vec4<f32>{return vec4<f32>(.38,.59,.72,1.);}

// 現在表示している曲面を次のタイル格子へGPU内で保存。途中の再目標化も同じ経路。
@group(0) @binding(8) var<storage,read> snapshotOwners:array<i32>;
@group(0) @binding(9) var<storage,read> snapshotTargets:array<Tile>;
@group(0) @binding(10) var<storage,read_write> snapshotOutput:array<Sample>;
@compute @workgroup_size(8,8) fn snapshotSurface(@builtin(global_invocation_id) id:vec3<u32>){
 if(id.z>=arrayLength(&snapshotTargets)){return;}let t=snapshotTargets[id.z];let n=t.data.y;if(id.x>n||id.y>n){return;}
 let q=mix(t.field.xy,t.field.zw,vec2<f32>(id.xy)/f32(n));var value=evaluate(q,t);let cell=vec2<i32>(floor(q/1000.));
 if(all(cell>=vec2<i32>(0))&&all(cell<vec2<i32>(128))){let index=snapshotOwners[u32(cell.y*128+cell.x)];if(index>=0){let prior=tiles[u32(index)];if(!provisionalTile(prior)||trustedTile(prior)){value=movingSurface(q,prior);}}}
 // 同じ制御点キャッシュは接合前の場を保存する。接合済みの曲面を
 // 再び接合すると、取得通知のたび境界帯の高さ・傾きが拡散する。
 if(t.data.z>0u){let prior=tiles[t.data.z-1u];value=evaluate(q,prior);if(camera.viewport.w<1.){value=mix(oldSample(q,prior),value,camera.viewport.w);}}
 let at=u32(t.misc.z)+id.y*(n+1u)+id.x;snapshotOutput[at]=Sample(vec4<f32>(value,1.),vec4<f32>(value,1.));
}

// 高度で薄くなる大気の、視点から地表までの光路を積分する。
fn airAltitude(p:vec3<f32>,r:f32)->f32{
 let c=dot(p.xz,p.xz)+p.y*(2.*r+p.y);
 return c/(sqrt(max(r*r+c,1.))+r);
}
fn airLinear(c:vec3<f32>)->vec3<f32>{return select(pow((c+vec3(.055))/1.055,vec3(2.4)),c/12.92,c<=vec3(.04045));}
fn airSky(eye:vec3<f32>,ray:vec3<f32>,r:f32)->vec3<f32>{
 let up=normalize(eye+vec3(0.,r,0.));let d=normalize(ray);let horizon=exp(-abs(dot(d,up))*4.);
 let color=mix(vec3(.22,.47,.78),vec3(.78,.85,.92),horizon);
 let glow=pow(max(dot(d,normalize(vec3(-.55,1.,.65))),0.),128.);
 return airLinear(min(color+vec3(.16,.13,.08)*glow,vec3(1.)));
}
fn airDaylight(space:vec3<f32>,eye:vec3<f32>,ray:vec3<f32>,r:f32)->vec3<f32>{
 let day=1.-smoothstep(20000.,100000.,max(airAltitude(eye,r),0.));
 return mix(space,airSky(eye,ray,r),day);
}
fn airDepth(eye:vec3<f32>,point:vec3<f32>,r:f32)->vec2<f32>{
 let distance=length(point-eye);if distance<1.{return vec2(0.);}
 let direction=(point-eye)/distance;let b=dot(eye,direction)+r*direction.y;
 let c=dot(eye.xz,eye.xz)+(eye.y-100000.)*(2.*r+eye.y+100000.);let disc=b*b-c;
 if disc<0.{return vec2(0.);}
 let start=max(0.,-b-sqrt(disc));let end=min(distance,-b+sqrt(disc));if end<=start{return vec2(0.);}
 let step=(end-start)/8.;var density=vec2(0.);
 for(var i=0u;i<8u;i++){
  let p=eye+direction*(start+(f32(i)+.5)*step);let h=max(airAltitude(p,r),0.);
  density+=exp(-h/vec2(8000.,1200.));
 }
 return density*step;
}
fn airHaze(color:vec3<f32>,eye:vec3<f32>,point:vec3<f32>,r:f32)->vec3<f32>{
 if distance(eye,point)<1.{return color;}
 let depth=airDepth(eye,point,r);let amount=1.-exp(-depth.x/130000.-depth.y/70000.);
 return mix(color,airSky(eye,point-eye,r),clamp(amount,0.,1.));
}

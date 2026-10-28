'use strict';
(function(root){
const common=`
float heightAt(vec3 p){
 float a=p.x,b=p.y,c=p.z;
 return h0*a*a*a+h1*b*b*b+h2*c*c*c+3.*h3*a*a*b+3.*h4*a*b*b+3.*h5*a*a*c+3.*h6*a*c*c+3.*h7*b*b*c+3.*h8*b*c*c+6.*h9*a*b*c;
}
vec3 gradientAt(vec3 p){
 float a=p.x,b=p.y,c=p.z;
 return vec3(3.*h0*a*a+6.*h3*a*b+3.*h4*b*b+6.*h5*a*c+3.*h6*c*c+6.*h9*b*c,
 3.*h1*b*b+3.*h3*a*a+6.*h4*a*b+6.*h7*b*c+3.*h8*c*c+6.*h9*a*c,
 3.*h2*c*c+3.*h5*a*a+6.*h6*a*c+3.*h7*b*b+6.*h8*b*c+6.*h9*a*b);
}
`;
const vs=`#version 300 es
precision highp float;precision highp int;
layout(location=1) in vec4 r0;layout(location=2) in vec4 r1;layout(location=3) in vec4 r2;layout(location=4) in vec4 r3;
uniform mat4 matrix;
flat out vec4 p0;flat out vec4 p1;flat out vec4 p2;flat out vec4 p3;
const int indices[24]=int[24](0,1,2,3,5,4,0,3,1,1,3,4,1,4,2,2,4,5,2,5,0,0,5,3);
void main(){
 p0=r0;p1=r1;p2=r2;p3=r3;
 float lo=min(min(r1.z,r1.w),min(min(min(r2.x,r2.y),min(r2.z,r2.w)),min(min(r3.x,r3.y),min(r3.z,r3.w))))-.05;
 float hi=max(max(r1.z,r1.w),max(max(max(r2.x,r2.y),max(r2.z,r2.w)),max(max(r3.x,r3.y),max(r3.z,r3.w))))+.05;
 int id=indices[gl_VertexID];int corner=id%3;vec2 xy=corner==0?r0.xy:(corner==1?r0.zw:r1.xy);
 vec2 ab=r0.zw-r0.xy,ac=r1.xy-r0.xy;float altitude=abs(ab.x*ac.y-ab.y*ac.x)/max(max(length(ab),length(ac)),length(ac-ab));
 vec2 center=(r0.xy+r0.zw+r1.xy)/3.;xy=center+(xy-center)*(1.+min(.05,3./max(altitude,.1)));
 gl_Position=matrix*vec4(xy.x,id<3?lo:hi,xy.y,1.);
}`;
const fs=`#version 300 es
precision highp float;precision highp int;
flat in vec4 p0;flat in vec4 p1;flat in vec4 p2;flat in vec4 p3;
uniform mat4 matrix;uniform vec3 eye;uniform vec3 back;uniform vec3 right;uniform vec3 up;
uniform vec4 viewport;uniform vec2 rayScale;uniform float nearPlane;uniform int wire;
out vec4 color;
float h0,h1,h2,h3,h4,h5,h6,h7,h8,h9;float edgeTolerance;
${common}
float polynomial(vec4 q,float t){return ((q.x*t+q.y)*t+q.z)*t+q.w;}
bool clipHalf(float value,float slope,inout float lo,inout float hi){
 if(abs(slope)<1e-12)return value>=-edgeTolerance;
 float t=(-edgeTolerance-value)/slope;
 if(slope>0.)lo=max(lo,t);else hi=min(hi,t);
 return lo<=hi;
}
void main(){
 float base=p1.z;h0=0.;h1=p1.w-base;h2=p2.x-base;h3=p2.y-base;h4=p2.z-base;h5=p2.w-base;h6=p3.x-base;h7=p3.y-base;h8=p3.z-base;h9=p3.w-base;
 vec2 a=p0.xy,ab=p0.zw-a,ac=p1.xy-a;float determinant=ab.x*ac.y-ab.y*ac.x;if(abs(determinant)<1e-10)discard;
 float altitude=abs(determinant)/max(max(length(ab),length(ac)),length(ac-ab));edgeTolerance=clamp(.02/max(altitude,.1),1e-6,.01);
 vec2 ndc=(gl_FragCoord.xy-viewport.xy)/viewport.zw*2.-1.;
 vec3 direction=normalize(-back+right*ndc.x*rayScale.x+up*ndc.y*rayScale.y);
 vec2 xy=eye.xz-a;
 float v=(xy.x*ac.y-xy.y*ac.x)/determinant,w=(ab.x*xy.y-ab.y*xy.x)/determinant;
 float dv=(direction.x*ac.y-direction.z*ac.x)/determinant,dw=(ab.x*direction.z-ab.y*direction.x)/determinant;
 float lo=nearPlane/max(-dot(direction,back),1e-6),hi=2000000.;
 if(!clipHalf(v,dv,lo,hi)||!clipHalf(w,dw,lo,hi)||!clipHalf(1.-v-w,-dv-dw,lo,hi))discard;
 float low=min(min(h0,h1),min(min(min(h2,h3),min(h4,h5)),min(min(h6,h7),min(h8,h9))))+base-.05;
 float high=max(max(h0,h1),max(max(max(h2,h3),max(h4,h5)),max(max(h6,h7),max(h8,h9))))+base+.05;
 if(abs(direction.y)<1e-10){if(eye.y<low||eye.y>high)discard;}
 else{float t0=(low-eye.y)/direction.y,t1=(high-eye.y)/direction.y;lo=max(lo,min(t0,t1));hi=min(hi,max(t0,t1));}
 if(lo>hi||hi<0.)discard;
 // 交差区間を0～1へ移し、大きな世界座標での3次式の桁落ちを避ける。
 vec3 origin=eye+direction*lo,span=direction*(hi-lo);vec2 local=origin.xz-a;
 v=(local.x*ac.y-local.y*ac.x)/determinant;w=(ab.x*local.y-ab.y*local.x)/determinant;
 vec3 start=vec3(1.-v-w,v,w),step=vec3(-dv-dw,dv,dw)*(hi-lo);
 float y0=origin.y-base;
 float f0=heightAt(start)-y0;
 float f1=heightAt(start+step/3.)-(y0+span.y/3.);
 float f2=heightAt(start+step*(2./3.))-(y0+span.y*(2./3.));
 float f3=heightAt(start+step)-(y0+span.y);
 float d1=f1-f0,d2=f2-2.*f1+f0,d3=f3-3.*f2+3.*f1-f0;
 vec4 q=vec4(4.5*d3,4.5*(d2-d3),3.*d1-1.5*d2+d3,f0);
 // 極値で区切ることで、3交点や接するだけの交点を取りこぼさない。
 float roots[2];int nr=0;float aa=3.*q.x,bb=2.*q.y,cc=q.z;
 float eps=1e-6*max(1.,max(abs(aa),max(abs(bb),abs(cc))));
 if(abs(aa)<eps){if(abs(bb)>eps){float r=-cc/bb;if(r>0.&&r<1.){roots[nr]=r;nr++;}}}
 else{float disc=bb*bb-4.*aa*cc;if(disc>=0.){
  float rdisc=sqrt(disc);float z=-.5*(bb+(bb>=0.?rdisc:-rdisc));
  float r0=abs(z)>eps?z/aa:-bb/(2.*aa),r1=abs(z)>eps?cc/z:r0;
  if(r0>r1){float temp=r0;r0=r1;r1=temp;}
  if(r0>0.&&r0<1.){roots[nr]=r0;nr++;}if(r1>0.&&r1<1.&&abs(r1-r0)>1e-6){roots[nr]=r1;nr++;}
 }}
 float left=0.,hit=-1.;
 for(int part=0;part<3;part++){
  if(part>nr)break;float finish=part<nr?roots[part]:1.;float fl=polynomial(q,left),fr=polynomial(q,finish);
  if(abs(fl)<.005){hit=left;break;}
  if(fl*fr<0.){float l=left,r=finish;for(int j=0;j<24;j++){float mid=(l+r)*.5,fm=polynomial(q,mid);if((fl>=0.)==(fm>=0.)){l=mid;fl=fm;}else r=mid;}hit=(l+r)*.5;break;}
  if(abs(fr)<.005){hit=finish;break;}left=finish;
 }
 if(hit<0.)discard;
 // 最後は元のBézier式で補正する。近似した平面の交点は使わない。
 for(int i=0;i<2;i++){
  vec3 b=start+step*hit;float residual=heightAt(b)-(y0+span.y*hit);
  float derivative=dot(gradientAt(b),step)-span.y;
  if(abs(derivative)>1e-7)hit=clamp(hit-residual/derivative,0.,1.);
 }
 vec3 bary=start+step*hit;if(min(bary.x,min(bary.y,bary.z))< -edgeTolerance*2.)discard;
 vec3 position=origin+span*hit;
 if(abs(heightAt(bary)-(position.y-base))>.05)discard;
 vec4 clip=matrix*vec4(position,1.);if(clip.w<=0.)discard;float depth=clip.z/clip.w*.5+.5;if(depth<0.||depth>1.)discard;
 gl_FragDepth=depth;
 vec3 g=gradientAt(bary);float hs=g.y-g.x,ht=g.z-g.x;
 float hx=(hs*ac.y-ht*ab.y)/determinant,hy=(ab.x*ht-ac.x*hs)/determinant;
 vec3 n=normalize(vec3(-hx,1.,-hy)),light=normalize(vec3(-.5,1.,-.6));
 float shade=clamp(.82+dot(n,light)-light.y,.15,1.2);
 vec3 tint=mix(vec3(.40,.49,.32),vec3(.69,.65,.51),clamp(position.y/3800.,0.,1.));
 if(wire==1){vec3 edge=smoothstep(vec3(0.),fwidth(bary)*1.1,bary);shade*=mix(.25,1.,min(edge.x,min(edge.y,edge.z)));}
 color=vec4(clamp(tint*shade,0.,1.),1.);
}`;

function debugShader(){
 const colors=['1.,0.,0.','0.,1.,0.','0.,0.,1.','1.,1.,0.','1.,0.,1.','0.,1.,1.','1.,.5,0.','1.,1.,1.','.5,.5,.5'];let index=0;
 return fs.replace(/discard;/g,()=>`{color=vec4(${colors[index++]||'0.,0.,0.'},1.);gl_FragDepth=0.;return;}`);
}

function distanceShader(){
 return fs.replace('color=vec4(clamp(tint*shade,0.,1.),1.);',
 'uint bits=floatBitsToUint(length(position-eye));color=vec4(float(bits&255u),float((bits>>8u)&255u),float((bits>>16u)&255u),float((bits>>24u)&255u))/255.;');
}

function create(gl,debug=false){
 function compile(type,source){const s=gl.createShader(type);gl.shaderSource(s,source);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;}
 const program=gl.createProgram();gl.attachShader(program,compile(gl.VERTEX_SHADER,vs));gl.attachShader(program,compile(gl.FRAGMENT_SHADER,debug===true?debugShader():(debug==='distance'?distanceShader():fs)));gl.linkProgram(program);if(!gl.getProgramParameter(program,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(program));
 const names=['matrix','eye','back','right','up','viewport','rayScale','nearPlane','wire'],uniforms=Object.fromEntries(names.map(k=>[k,gl.getUniformLocation(program,k)]));
 return {draw(matrix,camera,viewport,vao,count,wire){
  const back=[Math.cos(camera.pitch)*Math.sin(camera.yaw),Math.sin(camera.pitch),Math.cos(camera.pitch)*Math.cos(camera.yaw)],right=[Math.cos(camera.yaw),0,-Math.sin(camera.yaw)],up=[back[1]*right[2]-back[2]*right[1],back[2]*right[0]-back[0]*right[2],back[0]*right[1]-back[1]*right[0]];
  const eye=[camera.x+back[0]*camera.distance,camera.height+back[1]*camera.distance,camera.y+back[2]*camera.distance],tan=Math.tan(Math.PI/8);
  gl.useProgram(program);gl.uniformMatrix4fv(uniforms.matrix,false,matrix);
  for(const [key,value]of Object.entries({eye,back,right,up}))gl.uniform3fv(uniforms[key],value);
  gl.uniform4fv(uniforms.viewport,viewport);gl.uniform2f(uniforms.rayScale,tan*viewport[2]/viewport[3],tan);gl.uniform1f(uniforms.nearPlane,Math.max(1,camera.distance/10000));gl.uniform1i(uniforms.wire,wire?1:0);
  // 外向きの裏面を使うため、カメラが補助プリズム内にあっても描ける。
  gl.enable(gl.CULL_FACE);gl.cullFace(gl.FRONT);gl.bindVertexArray(vao);gl.drawArraysInstanced(gl.TRIANGLES,0,24,count);gl.disable(gl.CULL_FACE);gl.cullFace(gl.BACK);
 },program};
}

// CPU照合用。GPUと同じく極値で単調区間に分け、最も手前の解を返す。
function cubicRoot(q,low=0,high=1){
 const value=t=>((q[0]*t+q[1])*t+q[2])*t+q[3],a=3*q[0],b=2*q[1],c=q[2],cuts=[low,high];
 const eps=1e-12*Math.max(1,Math.abs(a),Math.abs(b),Math.abs(c));
 if(Math.abs(a)<eps){if(Math.abs(b)>eps)cuts.push(-c/b);}
 else{const d=b*b-4*a*c;if(d>=0){const z=-.5*(b+(b>=0?1:-1)*Math.sqrt(d));cuts.push(Math.abs(z)>eps?z/a:-b/(2*a),Math.abs(z)>eps?c/z:-b/(2*a));}}
 const sorted=[...new Set(cuts.filter(v=>v>=low&&v<=high))].sort((a,b)=>a-b);
 for(let i=0;i<sorted.length-1;i++){let l=sorted[i],r=sorted[i+1],fl=value(l),fr=value(r);if(Math.abs(fl)<1e-10)return l;if(fl*fr<0){for(let k=0;k<50;k++){const m=(l+r)/2,f=value(m);if((fl>=0)===(f>=0)){l=m;fl=f}else r=m;}return(l+r)/2;}if(Math.abs(fr)<1e-10)return r;}
 return null;
}
const api={create,cubicRoot,vertexShader:vs,fragmentShader:fs};if(typeof module!=='undefined')module.exports=api;else root.LpfCurveRaycast=api;
})(typeof window!=='undefined'?window:globalThis);

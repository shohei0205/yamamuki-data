'use strict';
// 自由配置のGaussian曲面を合成し、読み込み時に描画用メッシュをGPUで生成する。
const FeatureSurface=(()=>{
const functions=`
vec3 field(vec2 xy){
 vec3 sum=vec3(base.x+dot(base.yz,xy/span),base.y/span,base.z/span),compensation=vec3(0.);
 for(int k=0;k<512;k++){if(k>=featureCount)break;vec4 a=texelFetch(features,ivec2(0,k),0),b=texelFetch(features,ivec2(1,k),0);vec2 d=xy-a.xy;float u=(d.x*b.x+d.y*b.y)*a.z,v=(-d.x*b.y+d.y*b.x)*a.w;float g=b.z*exp(-.5*(u*u+v*v));vec3 term=g*vec3(1.,-u*b.x*a.z+v*b.y*a.w,-u*b.y*a.z-v*b.x*a.w);
  vec3 value=term-compensation,next=sum+value;compensation=(next-sum)-value;sum=next;
 }return sum;
}`;
const generationVertex=`#version 300 es
precision highp float;precision highp int;
layout(location=0) in vec2 xy;
uniform highp sampler2D features;uniform int featureCount;uniform vec3 base;uniform float span;
out vec3 generatedPosition;out vec3 generatedNormal;
${functions}
void main(){vec3 v=field(xy);generatedPosition=vec3(xy.x,v.x,xy.y);generatedNormal=vec3(-v.y,1.,-v.z);gl_Position=vec4(0.,0.,0.,1.);}`;
const vertex=`#version 300 es
precision highp float;
layout(location=0) in vec3 point;layout(location=1) in vec3 gradient;
uniform mat4 matrix;uniform int markers;
out vec3 position;out vec3 normal;
void main(){position=point;normal=gradient;vec3 p=point;if(markers==1)p.y+=15.;gl_Position=matrix*vec4(p,1.);gl_PointSize=8.;}`;
const fragment=`#version 300 es
precision highp float;precision highp int;
in vec3 position;in vec3 normal;
uniform int smoothMode,wire,markers,segments;uniform float span;
out vec4 color;
void main(){
 if(markers==1){float r=length(gl_PointCoord-.5);if(r>.5)discard;color=vec4(r>.32?vec3(.05):vec3(1.,.8,.2),1.);return;}
 vec3 n=smoothMode==1?normalize(normal):normalize(cross(dFdx(position),dFdy(position)));if(n.y<0.)n=-n;
 vec3 light=normalize(vec3(-.5,1.,-.6));float shade=clamp(.82+dot(n,light)-light.y,.15,1.2);vec3 tint=mix(vec3(.40,.49,.32),vec3(.69,.65,.51),clamp(position.y/3800.,0.,1.));
 if(wire==1){vec2 q=fract((position.xz+span*.5)/span*float(segments));vec3 b=q.x+q.y<=1.?vec3(1.-q.x-q.y,q.x,q.y):vec3(q.x+q.y-1.,1.-q.x,1.-q.y);vec3 edge=smoothstep(vec3(0.),max(fwidth(b),vec3(.00001)),b);shade*=mix(.3,1.,min(edge.x,min(edge.y,edge.z)));}
 color=vec4(clamp(tint*shade,0.,1.),1.);
}`;
function create(gl){
 const compile=(type,text)=>{const s=gl.createShader(type);gl.shaderSource(s,text);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;};
 function program(v,f,varyings){const p=gl.createProgram();gl.attachShader(p,compile(gl.VERTEX_SHADER,v));gl.attachShader(p,compile(gl.FRAGMENT_SHADER,f));if(varyings)gl.transformFeedbackVaryings(p,varyings,gl.INTERLEAVED_ATTRIBS);gl.linkProgram(p);if(!gl.getProgramParameter(p,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(p));return p;}
 const generation=program(generationVertex,'#version 300 es\nprecision highp float;out vec4 color;void main(){color=vec4(0.);}', ['generatedPosition','generatedNormal']),drawing=program(vertex,fragment);
 const gu=Object.fromEntries(['features','featureCount','base','span'].map(k=>[k,gl.getUniformLocation(generation,k)])),du=Object.fromEntries(['matrix','markers','smoothMode','wire','span','segments'].map(k=>[k,gl.getUniformLocation(drawing,k)]));
 const texture=gl.createTexture(),input=gl.createBuffer(),geometry=gl.createBuffer(),index=gl.createBuffer(),centers=gl.createBuffer(),inputVAO=gl.createVertexArray(),meshVAO=gl.createVertexArray(),centerVAO=gl.createVertexArray();
 let model=null,requested=4,actual=0,segments=0,indexCount=0,showCenters=false,generationMs=0;
 function configure(vao,buffer){gl.bindVertexArray(vao);gl.bindBuffer(gl.ARRAY_BUFFER,buffer);gl.enableVertexAttribArray(0);gl.vertexAttribPointer(0,3,gl.FLOAT,false,24,0);gl.enableVertexAttribArray(1);gl.vertexAttribPointer(1,3,gl.FLOAT,false,24,12);}
 function generatePoints(xy,output){
  gl.useProgram(generation);gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,texture);gl.uniform1i(gu.features,0);gl.uniform1i(gu.featureCount,model.features.length);gl.uniform3fv(gu.base,model.base);gl.uniform1f(gu.span,model.span);
  gl.bindVertexArray(inputVAO);gl.bindBuffer(gl.ARRAY_BUFFER,input);gl.bufferData(gl.ARRAY_BUFFER,xy,gl.STATIC_DRAW);gl.enableVertexAttribArray(0);gl.vertexAttribPointer(0,2,gl.FLOAT,false,8,0);gl.bindBuffer(gl.TRANSFORM_FEEDBACK_BUFFER,output);gl.bufferData(gl.TRANSFORM_FEEDBACK_BUFFER,xy.length/2*24,gl.STATIC_DRAW);gl.bindBufferBase(gl.TRANSFORM_FEEDBACK_BUFFER,0,output);gl.enable(gl.RASTERIZER_DISCARD);gl.beginTransformFeedback(gl.POINTS);gl.drawArrays(gl.POINTS,0,xy.length/2);gl.endTransformFeedback();gl.disable(gl.RASTERIZER_DISCARD);gl.bindBufferBase(gl.TRANSFORM_FEEDBACK_BUFFER,0,null);gl.bindVertexArray(null);
 }
 function build(force=false){if(!model)return;const d=Math.min(requested,16);if(!force&&d===actual)return;actual=d;segments=64*d;const start=performance.now(),xy=new Float32Array((segments+1)**2*2),ix=new Uint32Array(segments*segments*6);let at=0;
  for(let y=0;y<=segments;y++)for(let x=0;x<=segments;x++){xy[at++]=x/segments*model.span-model.span/2;xy[at++]=y/segments*model.span-model.span/2;}at=0;
  for(let y=0;y<segments;y++)for(let x=0;x<segments;x++){const a=y*(segments+1)+x,b=a+1,c=a+segments+1,e=c+1;ix.set([a,b,c,b,e,c],at);at+=6;}
  generatePoints(xy,geometry);configure(meshVAO,geometry);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,index);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,ix,gl.STATIC_DRAW);gl.bindVertexArray(null);indexCount=ix.length;generationMs=performance.now()-start;
 }
 return {
 upload(value){model=value;const data=new Float32Array(model.features.length*8);model.features.forEach((p,i)=>data.set([p[0],p[1],1/p[2],1/p[3],Math.cos(p[4]),Math.sin(p[4]),p[5],0],i*8));gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,texture);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA32F,2,model.features.length,0,gl.RGBA,gl.FLOAT,data);build(true);generatePoints(new Float32Array(model.features.flatMap(p=>p.slice(0,2))),centers);configure(centerVAO,centers);gl.bindVertexArray(null);},
 setDivisions(d){if(![1,2,4,8,16].includes(d))throw Error('分割数が不正です');requested=d;build();},setCenters(value){showCenters=value;},
 draw(matrix,camera,viewport,wire,smooth=true){if(!model)return;gl.useProgram(drawing);gl.uniformMatrix4fv(du.matrix,false,matrix);gl.uniform1i(du.markers,0);gl.uniform1i(du.smoothMode,smooth?1:0);gl.uniform1i(du.wire,wire?1:0);gl.uniform1f(du.span,model.span);gl.uniform1i(du.segments,segments);gl.bindVertexArray(meshVAO);gl.disable(gl.CULL_FACE);gl.drawElements(gl.TRIANGLES,indexCount,gl.UNSIGNED_INT,0);if(showCenters){gl.uniform1i(du.markers,1);gl.bindVertexArray(centerVAO);gl.drawArrays(gl.POINTS,0,model.features.length);}},
 profile(xs){if(!model)return xs.map(()=>0);return xs.map(x=>{let h=model.base[0]+model.base[1]*x/model.span;for(const p of model.features){const dx=x-p[0],dy=-p[1],cs=Math.cos(p[4]),sn=Math.sin(p[4]),u=(dx*cs+dy*sn)/p[2],v=(-dx*sn+dy*cs)/p[3];h+=p[5]*Math.exp(-.5*(u*u+v*v));}return h;});},
 get info(){return model?.info;},get stats(){return {triangles:2*segments**2,vertices:(segments+1)**2,segments,division:actual,gpuMeshBytes:(segments+1)**2*24+indexCount*4,generationSubmissionMs:generationMs};},
 readGeometry(){const points=new Float32Array((segments+1)**2*6);gl.bindBuffer(gl.ARRAY_BUFFER,geometry);gl.getBufferSubData(gl.ARRAY_BUFFER,0,points);return points;}
 };
}
return {create,generationVertexShader:generationVertex};
})();

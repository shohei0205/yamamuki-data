'use strict';
const AdaptiveSurface=(()=>{
const functions=`
vec3 rawField(vec2 xy){
 if(kernelMode==2||kernelMode==3){
  ivec2 bin=clamp(ivec2(floor((xy/span+.5)*float(binN))),ivec2(0),ivec2(binN-1));uvec2 row=texelFetch(lookup,bin,0).rg;
  float sw=0.,sh=0.;vec2 dwSum=vec2(0.),dn=vec2(0.);
  for(int j=0;j<1024;j++){if(j>=int(row.y))break;int index=int(row.x)+j,id=int(texelFetch(indices,ivec2(index%indexWidth,index/indexWidth),0).r)*3;
   vec4 a=texelFetch(patches,ivec2(id%1024,id/1024),0),b=texelFetch(patches,ivec2((id+1)%1024,(id+1)/1024),0),c=texelFetch(patches,ivec2((id+2)%1024,(id+2)/1024),0);
   vec2 q=(xy-a.xy)/a.z;float r=length(q);if(r>=1.)continue;float s=1.-r,confidence=kernelMode==3?c.y:1.,w=confidence*s*s*s*s*(4.*r+1.);
   float h=a.w+b.x*q.x+b.y*q.y+b.z*q.x*q.x+b.w*q.x*q.y+c.x*q.y*q.y;
   vec2 dh=vec2(b.x+2.*b.z*q.x+b.w*q.y,b.y+b.w*q.x+2.*c.x*q.y)/a.z,dw=-20.*confidence*s*s*s*q/a.z;
   sw+=w;sh+=w*h;dwSum+=dw;dn+=dw*h+w*dh;
  }float h=sh/max(sw,1e-20);return vec3(h,(dn-h*dwSum)/max(sw,1e-20));
 }
 if(kernelMode==1||kernelMode==4){
  vec2 q=clamp((xy/span+.5)*float(fieldN),vec2(0.),vec2(float(fieldN)));ivec2 lo=min(ivec2(floor(q)),ivec2(fieldN-1));vec2 f=q-vec2(lo);
  return mix(mix(texelFetch(fieldGrid,lo,0).xyz,texelFetch(fieldGrid,lo+ivec2(1,0),0).xyz,f.x),mix(texelFetch(fieldGrid,lo+ivec2(0,1),0).xyz,texelFetch(fieldGrid,lo+ivec2(1,1),0).xyz,f.x),f.y);
 }

 ivec2 bin=clamp(ivec2(floor((xy/span+.5)*float(binN))),ivec2(0),ivec2(binN-1));uvec2 row=texelFetch(lookup,bin,0).rg;
 for(int j=0;j<1024;j++){if(j>=int(row.y))break;int index=int(row.x)+j,id=int(texelFetch(indices,ivec2(index%indexWidth,index/indexWidth),0).r)*4;vec4 a=texelFetch(patches,ivec2(id%1024,id/1024),0),b=texelFetch(patches,ivec2((id+1)%1024,(id+1)/1024),0),c=texelFetch(patches,ivec2((id+2)%1024,(id+2)/1024),0),d=texelFetch(patches,ivec2((id+3)%1024,(id+3)/1024),0);vec2 e=a.xy-b.xy,f=a.zw-b.xy,q=xy-b.xy;float det=e.x*f.y-e.y*f.x;vec2 uv=vec2(q.x*f.y-q.y*f.x,e.x*q.y-e.y*q.x)/det;float u=uv.x,v=uv.y;if(min(u,min(v,1.-u-v))<-.00002)continue;
 float h=b.z+b.w*u+c.x*v+c.y*u*u+c.z*u*v+c.w*v*v+d.x*u*u*u+d.y*u*u*v+d.z*u*v*v+d.w*v*v*v;
 float du=b.w+2.*c.y*u+c.z*v+3.*d.x*u*u+2.*d.y*u*v+d.z*v*v,dv=c.x+c.z*u+2.*c.w*v+d.y*u*u+2.*d.z*u*v+3.*d.w*v*v;
 return vec3(h,(du*f.y-dv*e.y)/det,(-du*f.x+dv*e.x)/det);
 }return vec3(0.);
}
vec3 boundedField(vec2 xy){vec3 v=rawField(clamp(xy,vec2(-span*.5),vec2(span*.5)));if(abs(xy.x)>span*.5)v.y=0.;if(abs(xy.y)>span*.5)v.z=0.;return v;}
vec3 field(vec2 xy){
 if(fairing<=0.)return rawField(xy);
 vec3 total=rawField(xy)*.2;
 for(int i=0;i<4;i++){
  vec2 d=i==0?vec2(1.,0.):i==1?vec2(-1.,0.):i==2?vec2(0.,1.):vec2(0.,-1.);
  total+=boundedField(xy+d*fairing)*.1;
  total+=boundedField(xy+d*fairing*2.)*.05;
  vec2 diag=i==0?vec2(1.,1.):i==1?vec2(-1.,1.):i==2?vec2(-1.,-1.):vec2(1.,-1.);
  total+=boundedField(xy+diag*fairing)*.05;
 }return total;
}
`;
const declarations=`uniform highp sampler2D features,patches,fieldGrid;uniform highp usampler2D lookup,indices;uniform int binN,featureWidth,indexWidth,kernelMode,controlCount,fieldN;uniform vec3 base;uniform float span,fairing;`;
const generationVertex=`#version 300 es
precision highp float;precision highp int;layout(location=0) in vec2 xy;${declarations}
out vec3 generatedPosition;out vec3 generatedNormal;${functions}
void main(){vec3 v=field(xy);generatedPosition=vec3(xy.x,v.x,xy.y);generatedNormal=vec3(-v.y,1.,-v.z);gl_Position=vec4(0.,0.,0.,1.);}`;
const vertex=`#version 300 es
precision highp float;layout(location=0) in vec3 point;layout(location=1) in vec3 normalIn;uniform mat4 matrix;out vec3 position;out vec3 surfaceNormal;void main(){position=point;surfaceNormal=normalIn;gl_Position=matrix*vec4(point,1.);gl_PointSize=5.;}`;
const fragment=`#version 300 es
precision highp float;precision highp int;${declarations}
in vec3 position;in vec3 surfaceNormal;uniform int smoothMode,wire,markers,segments;out vec4 color;${functions}
void main(){if(markers>0){if(length(gl_PointCoord-.5)>.5)discard;color=vec4(markers==1?vec3(.1,.85,1.):vec3(1.,.35,.05),1.);return;}vec3 n=smoothMode==1?normalize(surfaceNormal):normalize(cross(dFdx(position),dFdy(position)));if(n.y<0.)n=-n;vec3 light=normalize(vec3(-.5,1.,-.6));float shade=clamp(.82+dot(n,light)-light.y,.15,1.2);vec3 tint=mix(vec3(.40,.49,.32),vec3(.69,.65,.51),clamp(position.y/3800.,0.,1.));if(wire==1){vec2 q=fract((position.xz/span+.5)*float(segments));vec3 b=q.x+q.y<=1.?vec3(1.-q.x-q.y,q.x,q.y):vec3(q.x+q.y-1.,1.-q.x,1.-q.y);vec3 edge=smoothstep(vec3(0.),max(fwidth(b),vec3(.00001)),b);shade*=mix(.3,1.,min(edge.x,min(edge.y,edge.z)));}color=vec4(clamp(tint*shade,0.,1.),1.);}`;
function create(gl){
 const compile=(kind,text)=>{const s=gl.createShader(kind);gl.shaderSource(s,text);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;};
 function program(v,f,varyings){const p=gl.createProgram();gl.attachShader(p,compile(gl.VERTEX_SHADER,v));gl.attachShader(p,compile(gl.FRAGMENT_SHADER,f));if(varyings)gl.transformFeedbackVaryings(p,varyings,gl.INTERLEAVED_ATTRIBS);gl.linkProgram(p);if(!gl.getProgramParameter(p,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(p));return p;}
 const generation=program(generationVertex,'#version 300 es\nprecision highp float;out vec4 color;void main(){color=vec4(0.);}', ['generatedPosition','generatedNormal']),drawing=program(vertex,fragment);
 const keys=['patches','features','lookup','indices','binN','featureWidth','indexWidth','base','span','fairing','kernelMode','controlCount','fieldGrid','fieldN'];
 const gu=Object.fromEntries(keys.map(k=>[k,gl.getUniformLocation(generation,k)])),du=Object.fromEntries([...keys,'matrix','smoothMode','wire','markers','segments'].map(k=>[k,gl.getUniformLocation(drawing,k)]));
 const input=gl.createBuffer(),geometry=gl.createBuffer(),elements=gl.createBuffer(),centers=gl.createBuffer(),sourcePoints=gl.createBuffer(),inputVAO=gl.createVertexArray(),meshVAO=gl.createVertexArray(),centerVAO=gl.createVertexArray(),sourceVAO=gl.createVertexArray();
 let model=null,fieldGrid=null,features=null,lookup=null,indices=null,triangles=null,textures=[],widths=[],fairing=40,requested=4,actual=0,segments=0,indexCount=0,inputCount=0,options={controls:true,inputs:false,xray:true};
 const decode=(text,T)=>new T(Uint8Array.from(atob(text),c=>c.charCodeAt(0)).buffer);
 function texture(unit,array,components,integer=false,fixedWidth=0){const t=gl.createTexture();textures.push(t);const width=fixedWidth||Math.min(1024,Math.max(1,array.length/components)),height=Math.ceil(array.length/components/width),data=new(integer?Uint32Array:Float32Array)(width*height*components);data.set(array);gl.activeTexture(gl.TEXTURE0+unit);gl.bindTexture(gl.TEXTURE_2D,t);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);gl.texImage2D(gl.TEXTURE_2D,0,integer?(components===2?gl.RG32UI:gl.R32UI):gl.RGBA32F,width,height,0,integer?(components===2?gl.RG_INTEGER:gl.RED_INTEGER):gl.RGBA,integer?gl.UNSIGNED_INT:gl.FLOAT,data);return width;}
 function bind(p,u){gl.useProgram(p);for(let k=0;k<textures.length;k++){gl.activeTexture(gl.TEXTURE0+k);gl.bindTexture(gl.TEXTURE_2D,textures[k]);}gl.uniform1i(u.fieldGrid,4);gl.uniform1i(u.fieldN,model.fieldN||0);gl.uniform1i(u.patches,3);gl.uniform1i(u.features,0);gl.uniform1i(u.lookup,1);gl.uniform1i(u.indices,2);gl.uniform1i(u.featureWidth,widths[0]);gl.uniform1i(u.indexWidth,widths[2]);gl.uniform1i(u.binN,model.binN);gl.uniform3fv(u.base,model.base);gl.uniform1f(u.span,model.span);gl.uniform1f(u.fairing,fairing);gl.uniform1i(u.kernelMode,model.kernelMode||0);gl.uniform1i(u.controlCount,model.info.controls);}
 function configure(vao,buffer){gl.bindVertexArray(vao);gl.bindBuffer(gl.ARRAY_BUFFER,buffer);gl.enableVertexAttribArray(0);gl.vertexAttribPointer(0,3,gl.FLOAT,false,24,0);gl.enableVertexAttribArray(1);gl.vertexAttribPointer(1,3,gl.FLOAT,false,24,12);}
 function generatePoints(xy,output){bind(generation,gu);gl.bindVertexArray(inputVAO);gl.bindBuffer(gl.ARRAY_BUFFER,input);gl.bufferData(gl.ARRAY_BUFFER,xy,gl.STATIC_DRAW);gl.enableVertexAttribArray(0);gl.vertexAttribPointer(0,2,gl.FLOAT,false,8,0);gl.bindBuffer(gl.TRANSFORM_FEEDBACK_BUFFER,output);gl.bufferData(gl.TRANSFORM_FEEDBACK_BUFFER,xy.length/2*24,gl.STATIC_DRAW);gl.bindBufferBase(gl.TRANSFORM_FEEDBACK_BUFFER,0,output);gl.enable(gl.RASTERIZER_DISCARD);gl.beginTransformFeedback(gl.POINTS);gl.drawArrays(gl.POINTS,0,xy.length/2);gl.endTransformFeedback();gl.disable(gl.RASTERIZER_DISCARD);gl.bindBufferBase(gl.TRANSFORM_FEEDBACK_BUFFER,0,null);}
 function build(force=false){if(!model)return;const d=Math.min(requested,8);if(d===actual&&!force)return;actual=d;segments=128*d;let at=0;const xy=new Float32Array((segments+1)**2*2),ix=new Uint32Array(segments*segments*6);for(let y=0;y<=segments;y++)for(let x=0;x<=segments;x++){xy[at++]=x/segments*model.span-model.span/2;xy[at++]=y/segments*model.span-model.span/2;}at=0;for(let y=0;y<segments;y++)for(let x=0;x<segments;x++){const a=y*(segments+1)+x;ix.set([a,a+1,a+segments+1,a+1,a+segments+2,a+segments+1],at);at+=6;}generatePoints(xy,geometry);configure(meshVAO,geometry);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,elements);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,ix,gl.STATIC_DRAW);gl.bindVertexArray(null);indexCount=ix.length;}
 function rawValueAt(x,y){if(model.kernelMode===4){const n=model.fieldN,qx=Math.max(0,Math.min(n,(x/model.span+.5)*n)),qy=Math.max(0,Math.min(n,(y/model.span+.5)*n)),ix=Math.min(n-1,Math.floor(qx)),iy=Math.min(n-1,Math.floor(qy)),fx=qx-ix,fy=qy-iy,k=(iy*(n+1)+ix)*4;return (fieldGrid[k]*(1-fx)+fieldGrid[k+4]*fx)*(1-fy)+(fieldGrid[k+(n+1)*4]*(1-fx)+fieldGrid[k+(n+2)*4]*fx)*fy;}if(model.kernelMode===2||model.kernelMode===3){let sw=0,sh=0;const ts=triangles;for(let k=0;k<ts.length;k+=12){const u=(x-ts[k])/ts[k+2],v=(y-ts[k+1])/ts[k+2],r=Math.hypot(u,v);if(r>=1)continue;const w=(model.kernelMode===3?ts[k+9]:1)*(1-r)**4*(4*r+1),h=ts[k+3]+ts[k+4]*u+ts[k+5]*v+ts[k+6]*u*u+ts[k+7]*u*v+ts[k+8]*v*v;sw+=w;sh+=w*h;}return sh/sw;}if(model.kernelMode===1){let h=model.base[0]+(model.base[1]*x+model.base[2]*y)/model.span;for(let k=0;k<features.length;k+=4){const dx=(x-features[k])/model.span,dy=(y-features[k+1])/model.span,r2=Math.max(dx*dx+dy*dy,1e-30);h+=features[k+3]*.5*r2*Math.log(r2);}return h;}const b=Math.max(0,Math.min(model.binN-1,Math.floor((y/model.span+.5)*model.binN)))*model.binN+Math.max(0,Math.min(model.binN-1,Math.floor((x/model.span+.5)*model.binN))),at=b*2,ts=triangles;for(let j=0;j<lookup[at+1];j++){const k=indices[lookup[at]+j]*16,e=[ts[k]-ts[k+4],ts[k+1]-ts[k+5]],f=[ts[k+2]-ts[k+4],ts[k+3]-ts[k+5]],q=[x-ts[k+4],y-ts[k+5]],det=e[0]*f[1]-e[1]*f[0],u=(q[0]*f[1]-q[1]*f[0])/det,v=(e[0]*q[1]-e[1]*q[0])/det;if(Math.min(u,v,1-u-v)<-.00002)continue;return ts[k+6]+ts[k+7]*u+ts[k+8]*v+ts[k+9]*u*u+ts[k+10]*u*v+ts[k+11]*v*v+ts[k+12]*u**3+ts[k+13]*u*u*v+ts[k+14]*u*v*v+ts[k+15]*v**3;}return 0;}

 function valueAt(x,y){if(fairing<=0)return rawValueAt(x,y);const clamp=v=>Math.max(-model.span/2,Math.min(model.span/2,v));let h=.2*rawValueAt(x,y);for(const [dx,dy] of [[1,0],[-1,0],[0,1],[0,-1]]){h+=.1*rawValueAt(clamp(x+dx*fairing),clamp(y+dy*fairing));h+=.05*rawValueAt(clamp(x+dx*2*fairing),clamp(y+dy*2*fairing));}for(const [dx,dy] of [[1,1],[-1,1],[-1,-1],[1,-1]])h+=.05*rawValueAt(clamp(x+dx*fairing),clamp(y+dy*fairing));return h;}
 function rebuildCenters(){if(!model)return;const points=new Float32Array(model.info.controls*2);for(let i=0;i<model.info.controls;i++)points.set(features.subarray(i*4,i*4+2),i*2);generatePoints(points,centers);configure(centerVAO,centers);gl.bindVertexArray(null);}

 return {
  upload(value){model=value;inputCount=0;features=decode(value.features,Float32Array);lookup=decode(value.lookup,Uint32Array);indices=decode(value.indices,Uint32Array);triangles=decode(value.triangles,Float32Array);fieldGrid=value.fieldGrid?decode(value.fieldGrid,Float32Array):new Float32Array(4);for(const t of textures)gl.deleteTexture(t);textures=[];widths=[texture(0,features,4),texture(1,lookup,2,true,model.binN),texture(2,indices,1,true),texture(3,triangles,4,false,1024),texture(4,fieldGrid,4,false,(model.fieldN||0)+1)];build(true);const points=new Float32Array(model.info.controls*2);for(let i=0;i<model.info.controls;i++)points.set(features.subarray(i*4,i*4+2),i*2);generatePoints(points,centers);configure(centerVAO,centers);gl.bindVertexArray(null);},
  setFairing(value){const next=Math.max(0,Math.min(240,Number(value)||0));if(next===fairing)return;fairing=next;build(true);rebuildCenters();},setDivisions(d){requested=d;build();},setPointDisplay(value){options={...options,...value};},
  setInputPoints(value){inputCount=value.length/3;const data=new Float32Array(inputCount*6);for(let i=0;i<inputCount;i++)data.set([value[i*3],value[i*3+2],value[i*3+1],0,1,0],i*6);gl.bindBuffer(gl.ARRAY_BUFFER,sourcePoints);gl.bufferData(gl.ARRAY_BUFFER,data,gl.STATIC_DRAW);configure(sourceVAO,sourcePoints);gl.bindVertexArray(null);},
  draw(matrix,camera,viewport,wire,smooth=true){if(!model)return;bind(drawing,du);gl.uniformMatrix4fv(du.matrix,false,matrix);gl.uniform1i(du.smoothMode,smooth?1:0);gl.uniform1i(du.wire,wire?1:0);gl.uniform1i(du.markers,0);gl.uniform1i(du.segments,segments);gl.bindVertexArray(meshVAO);gl.disable(gl.CULL_FACE);gl.drawElements(gl.TRIANGLES,indexCount,gl.UNSIGNED_INT,0);if(options.controls||(options.inputs&&inputCount)){const depth=gl.isEnabled(gl.DEPTH_TEST),write=gl.getParameter(gl.DEPTH_WRITEMASK);if(options.xray)gl.disable(gl.DEPTH_TEST);gl.depthMask(false);if(options.controls){gl.uniform1i(du.markers,1);gl.bindVertexArray(centerVAO);gl.drawArrays(gl.POINTS,0,model.info.controls);}if(options.inputs&&inputCount){gl.uniform1i(du.markers,2);gl.bindVertexArray(sourceVAO);gl.drawArrays(gl.POINTS,0,inputCount);}gl.depthMask(write);if(depth)gl.enable(gl.DEPTH_TEST);}gl.bindVertexArray(null);},
  profile(xs){return model?xs.map(x=>valueAt(x,0)):xs.map(()=>0);},
  readGeometry(){const result=new Float32Array((segments+1)**2*6);gl.bindBuffer(gl.ARRAY_BUFFER,geometry);gl.getBufferSubData(gl.ARRAY_BUFFER,0,result);return result;},
  storedControls(){if(!model||model.kernelMode!==4)return new Float32Array(0);const result=new Float32Array(model.info.controls*3);for(let i=0;i<model.info.controls;i++)result.set([features[i*4],features[i*4+1],features[i*4+3]],i*3);return result;},
  readControls(){const result=new Float32Array(model.info.controls*6);gl.bindBuffer(gl.ARRAY_BUFFER,centers);gl.getBufferSubData(gl.ARRAY_BUFFER,0,result);return result;},
  get info(){return model?.info;},get summit(){return model?.info.summit;},get pointStats(){return{controls:model?.info.controls||0,inputs:inputCount};},get stats(){return{triangles:indexCount/3,vertices:(segments+1)**2,division:actual,requestedDivision:requested,fairing,maxBin:model?.maxBin||0,derivedBytes:(model?.kernelMode===1||model?.kernelMode===4)?(features?.byteLength||0)+(fieldGrid?.byteLength||0)+12:(triangles?.byteLength||0)};}
 };
}
return{create,generationVertexShader:generationVertex};
})();

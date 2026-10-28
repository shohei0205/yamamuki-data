'use strict';
// BスプラインをGPUで評価してポリゴンへ展開する。旧交点探索は検証用に保持する。
const BsplineSurface=(()=>{
const vertex=`#version 300 es
void main(){vec2 p=vec2((gl_VertexID<<1)&2,gl_VertexID&2);gl_Position=vec4(p*2.-1.,0.,1.);}`;
const fragment=`#version 300 es
precision highp float;precision highp int;
uniform highp sampler2D coefficients;uniform mat4 matrix;
uniform vec3 eye,back,right,up;uniform vec4 viewport;
uniform vec2 rayScale,heightBounds,gradientBounds;uniform float span,stepSize,nearPlane;uniform int halo,wire,debugMode;
out vec4 color;vec2 localBound;vec3 localCurvature;
vec4 weights(float t){float s=1.-t;return vec4(s*s*s,3.*t*t*t-6.*t*t+4.,-3.*t*t*t+3.*t*t+3.*t+1.,t*t*t)/6.;}
vec4 derivatives(float t){float s=1.-t;return vec4(-3.*s*s,9.*t*t-12.*t,-9.*t*t+6.*t+3.,3.*t*t)/6.;}
vec3 field(vec2 xy){
 vec2 q=(xy+span*.5)/stepSize,integer=floor(q);ivec2 cell=ivec2(integer)+ivec2(halo);vec2 t=clamp(q-integer,0.,1.);
 float origin=texelFetch(coefficients,cell,0).r;
 vec4 bx=weights(t.x),by=weights(t.y),dx=derivatives(t.x),dy=derivatives(t.y);vec3 value=vec3(origin,0.,0.);vec4 previousRow=vec4(0.),secondRow=vec4(0.);localBound=vec2(0.);localCurvature=vec3(0.);
 // 共通の高さを引いてから加算し、平坦部の高さと勾配の丸め誤差を抑える。
 for(int j=0;j<4;j++){vec4 row=vec4(0.);float previous=0.,second=0.;for(int i=0;i<4;i++){float c=texelFetch(coefficients,cell+ivec2(i-1,j-1),0).r;value+=(c-origin)*vec3(bx[i]*by[j],dx[i]*by[j]/stepSize,bx[i]*dy[j]/stepSize);if(i>0)localBound.x=max(localBound.x,abs(c-previous));if(j>0)localBound.y=max(localBound.y,abs(c-previousRow[i]));if(i>1)localCurvature.x=max(localCurvature.x,abs(c-2.*previous+second));if(j>1)localCurvature.z=max(localCurvature.z,abs(c-2.*previousRow[i]+secondRow[i]));if(i>0&&j>0)localCurvature.y=max(localCurvature.y,abs(c-previous-previousRow[i]+previousRow[i-1]));second=previous;previous=c;row[i]=c;}secondRow=previousRow;previousRow=row;}localBound/=stepSize;localCurvature/=stepSize*stepSize;
 return value;
}
bool slab(float p,float d,float low,float high,inout float a,inout float b){if(abs(d)<1e-9)return p>=low&&p<=high;float x=(low-p)/d,y=(high-p)/d;a=max(a,min(x,y));b=min(b,max(x,y));return a<=b;}
// ほぼ接線になる区間は6次Bernstein多項式の凸包で根を分離する。
// 高さ差だけで停止せず、根がない区間は曲面に近くても捨てられる。
float intervalRoot(vec3 start,vec3 direction,float length){
 vec2 q=(start.xz+span*.5)/stepSize,integer=floor(q),delta=direction.xz*length/stepSize;
 ivec2 cell=ivec2(integer)+ivec2(halo);vec2 uv=clamp(q-integer,0.,1.),end=uv+delta;
 vec4 ux[4],vy[4];ux[0]=weights(uv.x);ux[3]=weights(end.x);ux[1]=ux[0]+delta.x/3.*derivatives(uv.x);ux[2]=ux[3]-delta.x/3.*derivatives(end.x);
 vy[0]=weights(uv.y);vy[3]=weights(end.y);vy[1]=vy[0]+delta.y/3.*derivatives(uv.y);vy[2]=vy[3]-delta.y/3.*derivatives(end.y);
 float controls[16];float origin=texelFetch(coefficients,cell,0).r;
 for(int j=0;j<4;j++)for(int i=0;i<4;i++)controls[j*4+i]=texelFetch(coefficients,cell+ivec2(i-1,j-1),0).r-origin;
 float choose3[4]=float[4](1.,3.,3.,1.);float choose6[7]=float[7](1.,6.,15.,20.,15.,6.,1.);float initial[7];
 for(int k=0;k<7;k++){float h=0.;for(int a=0;a<4;a++){int b=k-a;if(b<0||b>3)continue;float v=0.;for(int j=0;j<4;j++)for(int i=0;i<4;i++)v+=controls[j*4+i]*ux[a][i]*vy[b][j];h+=v*choose3[a]*choose3[b]/choose6[k];}initial[k]=(start.y-origin)+direction.y*length*float(k)/6.-h;}
 vec4 stackA[32];vec3 stackB[32];vec2 intervals[32];int count=1;
 stackA[0]=vec4(initial[0],initial[1],initial[2],initial[3]);stackB[0]=vec3(initial[4],initial[5],initial[6]);intervals[0]=vec2(0.,1.);
 for(int visit=0;visit<256;visit++){
  if(count==0)return -1.;count--;vec2 range=intervals[count];float c[7];for(int i=0;i<4;i++)c[i]=stackA[count][i];for(int i=0;i<3;i++)c[i+4]=stackB[count][i];
  float low=c[0],high=c[0];for(int i=1;i<7;i++){low=min(low,c[i]);high=max(high,c[i]);}if(low>0.||high<0.)continue;
  if(high==0.&&low==0.)continue;
  if((range.y-range.x)*length<.02)return (range.x+range.y)*.5*length;
  float left[7],right[7];left[0]=c[0];right[6]=c[6];for(int row=1;row<7;row++){for(int i=0;i<7-row;i++)c[i]=(c[i]+c[i+1])*.5;left[row]=c[0];right[6-row]=c[6-row];}
  if(count>29)return -2.;float middle=(range.x+range.y)*.5;
  stackA[count]=vec4(right[0],right[1],right[2],right[3]);stackB[count]=vec3(right[4],right[5],right[6]);intervals[count]=vec2(middle,range.y);count++;
  stackA[count]=vec4(left[0],left[1],left[2],left[3]);stackB[count]=vec3(left[4],left[5],left[6]);intervals[count]=vec2(range.x,middle);count++;
 }return -2.;
}
void main(){
 vec2 pixel=((gl_FragCoord.xy-viewport.xy)/viewport.zw*2.-1.)*rayScale;
 vec3 direction=normalize(-back+right*pixel.x+up*pixel.y);
 float a=nearPlane/max(1e-6,-dot(direction,back)),b=2000000.;
 if(!slab(eye.x,direction.x,-span*.5,span*.5,a,b)||!slab(eye.z,direction.z,-span*.5,span*.5,a,b)||!slab(eye.y,direction.y,heightBounds.x,heightBounds.y,a,b))discard;
 // 係数差の勾配上界を使い、探索で地表を飛び越さないようにする。
 float t=a,last=a,lastF=0.;bool hit=false;int iterations=0;
 for(int i=0;i<2048;i++){
  iterations=i;vec3 p=eye+direction*t,curve=field(p.xz);vec2 boundAtP=localBound;vec3 curvatureAtP=localCurvature;float f=p.y-curve.x;
  vec2 cell=floor((p.xz+span*.5)/stepSize),edge=(cell+step(vec2(0.),direction.xz))*stepSize-span*.5;
  float exitDistance=2000000.;if(abs(direction.x)>1e-9)exitDistance=min(exitDistance,max(0.,(edge.x-p.x)/direction.x));if(abs(direction.z)>1e-9)exitDistance=min(exitDistance,max(0.,(edge.y-p.z)/direction.z));
  // 高さ差が小さいだけでは交点としない。平行な視線に偽の面が生じるため。
  float derivative=direction.y-dot(curve.yz,direction.xz);
  if(abs(derivative)>1e-7){float correction=-f/derivative,candidate=t+correction;
   float curvature=dot(curvatureAtP,vec3(direction.x*direction.x,2.*abs(direction.x*direction.z),direction.z*direction.z));
   bool monotone=correction>=0.&&correction<=exitDistance&&abs(derivative)>curvature*correction;
   if((abs(correction)<.05||monotone)&&candidate>=a&&candidate<=b){vec3 q=eye+direction*candidate;float residual=q.y-field(q.xz).x;if(abs(residual)<.0005){t=candidate;hit=true;break;}}
  }
  if(i>0&&f*lastF<0.){float lo=last,hi=t,fl=lastF;for(int k=0;k<20;k++){float m=(lo+hi)*.5;vec3 v=eye+direction*m;float fm=v.y-field(v.xz).x;if(fm*fl>0.){lo=m;fl=fm;}else hi=m;}t=(lo+hi)*.5;hit=true;break;}
  if((abs(f)<.25||i>128)&&exitDistance>.001){float interval=min(exitDistance,b-t),root=intervalRoot(p,direction,interval);if(root>=0.){t+=root;hit=true;break;}if(root== -1.){last=t;lastF=f;t+=max(.002,t*1e-7)+interval;if(t>b)break;continue;}}
  // 各区間内の係数差から求めた上界を使い、区間を出る所までに歩幅を制限する。
  float bound=abs(direction.y)+dot(abs(direction.xz),min(boundAtP,gradientBounds));
  last=t;lastF=f;t+=max(max(.001,t*1e-7),bound<1e-8?exitDistance+.001:min(abs(f)/bound*.95,exitDistance+.001));if(t>b)break;
 }
 if(!hit){if(debugMode==1&&iterations==2047){color=vec4(1.,0.,1.,1.);gl_FragDepth=.999999;return;}discard;}
 // 区間分離で得た距離も実際の曲面へ寄せる。距離の許容幅を高さ誤差にしない。
 for(int refine=0;refine<4;refine++){vec3 p=eye+direction*t,v=field(p.xz);float f=p.y-v.x,d=direction.y-dot(v.yz,direction.xz);if(abs(f)<.0005||abs(d)<1e-7)break;float candidate=t-f/d;if(candidate<a||candidate>b||abs(candidate-t)>.1)break;vec3 q=eye+direction*candidate;if(abs(q.y-field(q.xz).x)>=abs(f))break;t=candidate;}
 vec3 position=eye+direction*t,curve=field(position.xz);

 float residualTolerance=.002+3e-7*(abs(position.y)+abs(eye.y)+abs(t*direction.y)+dot(abs(curve.yz),abs(position.xz)+vec2(span*.5)));
 if(t<a||t>b||any(greaterThan(abs(position.xz),vec2(span*.5+.01)))||abs(position.y-curve.x)>residualTolerance){if(debugMode==1){color=vec4(1.,0.,0.,1.);gl_FragDepth=.999999;return;}discard;}
 vec3 n=normalize(vec3(-curve.y,1.,-curve.z));
 vec3 light=normalize(vec3(-.5,1.,-.6));float shade=clamp(.82+dot(n,light)-light.y,.15,1.2);
 vec3 base=mix(vec3(.40,.49,.32),vec3(.69,.65,.51),clamp(position.y/3800.,0.,1.));
 if(wire==1){vec2 q=(position.xz+span*.5)/stepSize,edge=min(fract(q),1.-fract(q));vec2 aa=max(fwidth(q),vec2(.0001));shade*=mix(.25,1.,min(smoothstep(0.,aa.x,edge.x),smoothstep(0.,aa.y,edge.y)));}
 color=vec4(clamp(base*shade,0.,1.),1.);vec4 clip=matrix*vec4(position,1.);gl_FragDepth=clip.z/clip.w*.5+.5;
}`;
function createRay(gl){
 function shader(type,text){const s=gl.createShader(type);gl.shaderSource(s,text);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;}
 const program=gl.createProgram();gl.attachShader(program,shader(gl.VERTEX_SHADER,vertex));gl.attachShader(program,shader(gl.FRAGMENT_SHADER,fragment));gl.linkProgram(program);if(!gl.getProgramParameter(program,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(program));
 const keys=['coefficients','matrix','eye','back','right','up','viewport','rayScale','heightBounds','gradientBounds','span','stepSize','nearPlane','halo','wire','debugMode'],u=Object.fromEntries(keys.map(k=>[k,gl.getUniformLocation(program,k)]));
 const texture=gl.createTexture(),vao=gl.createVertexArray();let model=null,controls=null;
 return {upload(value){model=value;const bytes=Uint8Array.from(atob(value.coefficients),c=>c.charCodeAt(0)),c=new Float32Array(bytes.buffer);controls=c;gl.bindTexture(gl.TEXTURE_2D,texture);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);gl.texImage2D(gl.TEXTURE_2D,0,gl.R32F,value.size,value.size,0,gl.RED,gl.FLOAT,c);},
 profile(xs){if(!model)return xs.map(()=>0);const basis=t=>[(1-t)**3/6,(3*t**3-6*t*t+4)/6,(-3*t**3+3*t*t+3*t+1)/6,t**3/6];return xs.map(x=>{const qx=(x+model.span/2)/model.info.stepMeters+model.halo,qy=model.span/2/model.info.stepMeters+model.halo,bx=basis(qx%1),by=basis(qy%1);let h=0;for(let j=0;j<4;j++)for(let i=0;i<4;i++)h+=controls[(Math.floor(qy)+j-1)*model.size+Math.floor(qx)+i-1]*bx[i]*by[j];return h;});},
 draw(matrix,camera,viewport,wire){if(!model)return;const back=[Math.cos(camera.pitch)*Math.sin(camera.yaw),Math.sin(camera.pitch),Math.cos(camera.pitch)*Math.cos(camera.yaw)],right=[Math.cos(camera.yaw),0,-Math.sin(camera.yaw)],up=[back[1]*right[2]-back[2]*right[1],back[2]*right[0]-back[0]*right[2],back[0]*right[1]-back[1]*right[0]],eye=[camera.x+back[0]*camera.distance,camera.height+back[1]*camera.distance,camera.y+back[2]*camera.distance];
 gl.useProgram(program);gl.uniformMatrix4fv(u.matrix,false,matrix);for(const[k,v]of Object.entries({eye,back,right,up}))gl.uniform3fv(u[k],v);gl.uniform4fv(u.viewport,viewport);const tan=Math.tan(Math.PI/8);gl.uniform2f(u.rayScale,tan*viewport[2]/viewport[3],tan);gl.uniform2fv(u.heightBounds,model.heightBounds);gl.uniform2fv(u.gradientBounds,model.gradientBounds);gl.uniform1f(u.span,model.span);gl.uniform1f(u.stepSize,model.info.stepMeters);gl.uniform1f(u.nearPlane,Math.max(1,camera.distance/10000));gl.uniform1i(u.halo,model.halo);gl.uniform1i(u.wire,wire?1:0);gl.uniform1i(u.debugMode,new URLSearchParams(location.search).has('splineDebug')?1:0);gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,texture);gl.uniform1i(u.coefficients,0);gl.bindVertexArray(vao);gl.disable(gl.CULL_FACE);gl.drawArrays(gl.TRIANGLES,0,3);
 },get info(){return model?.info;},get summit(){return model?.summit;}};
}
const surfaceFunctions=`
vec4 weights(float t){float s=1.-t;return vec4(s*s*s,3.*t*t*t-6.*t*t+4.,-3.*t*t*t+3.*t*t+3.*t+1.,t*t*t)/6.;}
vec4 derivatives(float t){float s=1.-t;return vec4(-3.*s*s,9.*t*t-12.*t,-9.*t*t+6.*t+3.,3.*t*t)/6.;}
vec3 field(vec2 xy){
 vec2 q=(xy+span*.5)/stepSize,integer=floor(q);ivec2 cell=ivec2(integer)+ivec2(halo);vec2 t=clamp(q-integer,0.,1.);
 float origin=texelFetch(coefficients,cell,0).r;vec4 bx=weights(t.x),by=weights(t.y),dx=derivatives(t.x),dy=derivatives(t.y);vec3 result=vec3(origin,0.,0.);
 for(int j=0;j<4;j++)for(int i=0;i<4;i++){float c=texelFetch(coefficients,cell+ivec2(i-1,j-1),0).r-origin;result+=c*vec3(bx[i]*by[j],dx[i]*by[j]/stepSize,bx[i]*dy[j]/stepSize);}
 return result;
}`;
const polygonVertex=`#version 300 es
precision highp float;precision highp int;
layout(location=0) in vec2 grid;
uniform highp sampler2D coefficients;uniform mat4 matrix;uniform float span,stepSize;uniform int halo,cells;
out vec3 position;
${surfaceFunctions}
void main(){ivec2 cell=ivec2(gl_InstanceID%cells,gl_InstanceID/cells);vec2 xy=(vec2(cell)+grid)*stepSize-span*.5;position=vec3(xy.x,field(xy).x,xy.y);gl_Position=matrix*vec4(position,1.);}`;
const polygonFragment=`#version 300 es
precision highp float;precision highp int;
uniform highp sampler2D coefficients;uniform float span,stepSize;uniform int halo,wire,normalMode,divisions;
in vec3 position;out vec4 color;
${surfaceFunctions}
void main(){
 vec3 n;if(normalMode==1){vec3 v=field(position.xz);n=normalize(vec3(-v.y,1.,-v.z));}else{n=normalize(cross(dFdx(position),dFdy(position)));if(n.y<0.)n=-n;}
 vec3 light=normalize(vec3(-.5,1.,-.6));float shade=clamp(.82+dot(n,light)-light.y,.15,1.2);
 vec3 base=mix(vec3(.40,.49,.32),vec3(.69,.65,.51),clamp(position.y/3800.,0.,1.));
 if(wire==1){vec2 q=fract((position.xz+span*.5)/stepSize*float(divisions));vec3 bary=q.x+q.y<=1.?vec3(1.-q.x-q.y,q.x,q.y):vec3(q.x+q.y-1.,1.-q.x,1.-q.y);vec3 edge=smoothstep(vec3(0.),max(fwidth(bary),vec3(.00001)),bary);shade*=mix(.3,1.,min(edge.x,min(edge.y,edge.z)));}
 color=vec4(clamp(base*shade,0.,1.),1.);
}`;
const controlPointVertex=`#version 300 es
precision highp float;precision highp int;
layout(location=0) in vec3 inputPoint;
uniform highp sampler2D coefficients;uniform mat4 matrix;uniform float span,stepSize,pointSize;uniform int halo,cells,pointMode;
void main(){vec3 p;if(pointMode==0){ivec2 ij=ivec2(gl_VertexID%(cells+1),gl_VertexID/(cells+1));vec2 xy=vec2(ij)*stepSize-span*.5;p=vec3(xy.x,texelFetch(coefficients,ij+ivec2(halo),0).r,xy.y);}else p=vec3(inputPoint.x,inputPoint.z,inputPoint.y);gl_Position=matrix*vec4(p,1.);gl_PointSize=pointSize;}`;
const controlPointFragment=`#version 300 es
precision highp float;uniform vec3 pointColor;out vec4 color;
void main(){vec2 d=gl_PointCoord-.5;if(dot(d,d)>.25)discard;color=vec4(pointColor,1.);}`;
function create(gl){
 function compile(type,source){const s=gl.createShader(type);gl.shaderSource(s,source);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;}
 const program=gl.createProgram();gl.attachShader(program,compile(gl.VERTEX_SHADER,polygonVertex));gl.attachShader(program,compile(gl.FRAGMENT_SHADER,polygonFragment));gl.linkProgram(program);if(!gl.getProgramParameter(program,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(program));
 const pointProgram=gl.createProgram();gl.attachShader(pointProgram,compile(gl.VERTEX_SHADER,controlPointVertex));gl.attachShader(pointProgram,compile(gl.FRAGMENT_SHADER,controlPointFragment));gl.linkProgram(pointProgram);if(!gl.getProgramParameter(pointProgram,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(pointProgram));
 const pu=Object.fromEntries(['coefficients','matrix','span','stepSize','halo','cells','pointMode','pointSize','pointColor'].map(k=>[k,gl.getUniformLocation(pointProgram,k)]));
 const storedControlVAO=gl.createVertexArray(),storedControlBuffer=gl.createBuffer();let storedControlCount=0;
 const pointVAO=gl.createVertexArray(),inputVAO=gl.createVertexArray(),inputBuffer=gl.createBuffer();let inputCount=0,pointDisplay={controls:false,inputs:false,xray:true};
 const u=Object.fromEntries(['coefficients','matrix','span','stepSize','halo','cells','wire','normalMode','divisions'].map(k=>[k,gl.getUniformLocation(program,k)]));
 const texture=gl.createTexture(),vao=gl.createVertexArray(),vertices=gl.createBuffer(),indices=gl.createBuffer();let model=null,controls=null,requested=4,actual=0,indexCount=0;
 function build(){if(!model)return;let d=requested;while(d>1&&2*model.info.n**2*d*d>4000000)d/=2;if(d===actual)return;actual=d;
  const xy=new Float32Array((d+1)**2*2),ix=new Uint16Array(d*d*6);let at=0;
  for(let y=0;y<=d;y++)for(let x=0;x<=d;x++){xy[at++]=x/d;xy[at++]=y/d;}at=0;
  for(let y=0;y<d;y++)for(let x=0;x<d;x++){const a=y*(d+1)+x,b=a+1,c=a+d+1,e=c+1;ix.set([a,b,c,b,e,c],at);at+=6;}
  gl.bindVertexArray(vao);gl.bindBuffer(gl.ARRAY_BUFFER,vertices);gl.bufferData(gl.ARRAY_BUFFER,xy,gl.STATIC_DRAW);gl.enableVertexAttribArray(0);gl.vertexAttribPointer(0,2,gl.FLOAT,false,8,0);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,indices);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,ix,gl.STATIC_DRAW);gl.bindVertexArray(null);indexCount=ix.length;
 }
 return {
 upload(value){model=value;inputCount=0;storedControlCount=0;if(value.controlPoints!==undefined){const points=new Float32Array(Uint8Array.from(atob(value.controlPoints),c=>c.charCodeAt(0)).buffer);storedControlCount=points.length/3;gl.bindVertexArray(storedControlVAO);gl.bindBuffer(gl.ARRAY_BUFFER,storedControlBuffer);gl.bufferData(gl.ARRAY_BUFFER,points,gl.STATIC_DRAW);gl.enableVertexAttribArray(0);gl.vertexAttribPointer(0,3,gl.FLOAT,false,12,0);gl.bindVertexArray(null);}controls=new Float32Array(Uint8Array.from(atob(value.coefficients),c=>c.charCodeAt(0)).buffer);gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,texture);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);gl.texImage2D(gl.TEXTURE_2D,0,gl.R32F,value.size,value.size,0,gl.RED,gl.FLOAT,controls);build();},
 setPointDisplay(value){pointDisplay={...pointDisplay,...value};},
 setInputPoints(value){if(value.length%3)throw Error('入力点の座標数が不正です');inputCount=value.length/3;gl.bindVertexArray(inputVAO);gl.bindBuffer(gl.ARRAY_BUFFER,inputBuffer);gl.bufferData(gl.ARRAY_BUFFER,value,gl.STATIC_DRAW);gl.enableVertexAttribArray(0);gl.vertexAttribPointer(0,3,gl.FLOAT,false,12,0);gl.bindVertexArray(null);},
 get pointStats(){return {controls:model?(model.controlPoints!==undefined?storedControlCount:(model.info.n+1)**2):0,inputs:inputCount};},
 setDivisions(value){if(![1,2,4,8,16].includes(value))throw Error('曲面分割数が不正です');requested=value;build();},
 profile(xs){if(!model)return xs.map(()=>0);const basis=t=>[(1-t)**3/6,(3*t**3-6*t*t+4)/6,(-3*t**3+3*t*t+3*t+1)/6,t**3/6];return xs.map(x=>{const qx=(x+model.span/2)/model.info.stepMeters,qy=model.span/2/model.info.stepMeters,ix=Math.floor(qx),iy=Math.floor(qy),bx=basis(qx-ix),by=basis(qy-iy);let h=0;for(let j=0;j<4;j++)for(let i=0;i<4;i++)h+=controls[(iy+model.halo+j-1)*model.size+ix+model.halo+i-1]*bx[i]*by[j];return h;});},
 draw(matrix,camera,viewport,wire,smooth=true){if(!model)return;gl.useProgram(program);gl.uniformMatrix4fv(u.matrix,false,matrix);gl.uniform1f(u.span,model.span);gl.uniform1f(u.stepSize,model.info.stepMeters);gl.uniform1i(u.halo,model.halo);gl.uniform1i(u.cells,model.info.n);gl.uniform1i(u.divisions,actual);gl.uniform1i(u.wire,wire?1:0);gl.uniform1i(u.normalMode,smooth?1:0);gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,texture);gl.uniform1i(u.coefficients,0);gl.bindVertexArray(vao);gl.disable(gl.CULL_FACE);gl.drawElementsInstanced(gl.TRIANGLES,indexCount,gl.UNSIGNED_SHORT,0,model.info.n**2);
  if(pointDisplay.controls||(pointDisplay.inputs&&inputCount)){const depth=gl.isEnabled(gl.DEPTH_TEST),write=gl.getParameter(gl.DEPTH_WRITEMASK);if(pointDisplay.xray)gl.disable(gl.DEPTH_TEST);gl.depthMask(false);gl.useProgram(pointProgram);gl.uniformMatrix4fv(pu.matrix,false,matrix);gl.uniform1f(pu.span,model.span);gl.uniform1f(pu.stepSize,model.info.stepMeters);gl.uniform1i(pu.halo,model.halo);gl.uniform1i(pu.cells,model.info.n);gl.uniform1i(pu.coefficients,0);gl.uniform1f(pu.pointSize,4*Math.min(devicePixelRatio,2));
   if(pointDisplay.controls){const custom=model.controlPoints!==undefined;gl.uniform1i(pu.pointMode,custom?1:0);gl.uniform3f(pu.pointColor,.1,.85,1.);gl.bindVertexArray(custom?storedControlVAO:pointVAO);gl.drawArrays(gl.POINTS,0,custom?storedControlCount:(model.info.n+1)**2);}
   if(pointDisplay.inputs&&inputCount){gl.uniform1i(pu.pointMode,1);gl.uniform3f(pu.pointColor,1.,.35,.05);gl.bindVertexArray(inputVAO);gl.drawArrays(gl.POINTS,0,inputCount);}
   gl.depthMask(write);if(depth)gl.enable(gl.DEPTH_TEST);else gl.disable(gl.DEPTH_TEST);gl.bindVertexArray(vao);
  }
 },
 get stats(){return model?{triangles:2*(model.info.n*actual)**2,vertices:(model.info.n*actual+1)**2,division:actual,requestedDivision:requested,templateBytes:(actual+1)**2*8+actual*actual*12}:null;},get info(){return model?.info;},get summit(){return model?.summit;}
 };
}
return {create,createRay,vertexShader:vertex,fragmentShader:fragment,polygonVertexShader:polygonVertex,polygonFragmentShader:polygonFragment};
})();

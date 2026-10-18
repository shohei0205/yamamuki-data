/* 海域ベクターを標高 0 m の独立した面として描く。 */
'use strict';
window.SeaVectors=class {
 constructor(gl){
  this.gl=gl;this.cache=new Map();this.count=0;this.bytes=0;this.transferred=0;
  const shader=(type,source)=>{const s=gl.createShader(type);gl.shaderSource(s,source);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;};
  this.program=gl.createProgram();
  gl.attachShader(this.program,shader(gl.VERTEX_SHADER,`#version 300 es
  precision highp float;in vec2 uv;out vec2 mapUV;uniform mat4 matrix;uniform vec2 worldSize;uniform bool national;uniform vec2 center;uniform float scale;uniform float aspect;uniform float bearing;uniform float tilt;uniform float depthRange;
  void main(){mapUV=uv;vec3 p=vec3((uv-0.5)*worldSize*vec2(1.0,-1.0),0.3);
   if(national){p.xy-=center;p.xy=mat2(cos(bearing),-sin(bearing),sin(bearing),cos(bearing))*p.xy;vec3 q=vec3(p.x,p.y*cos(tilt)+p.z*sin(tilt),-p.y*sin(tilt)+p.z*cos(tilt));gl_Position=vec4(q.x/(scale*aspect),q.y/scale,-q.z/depthRange,1.0);}
   else gl_Position=matrix*vec4(p,1.0);
  }`));
  gl.attachShader(this.program,shader(gl.FRAGMENT_SHADER,`#version 300 es
  precision highp float;in vec2 mapUV;uniform bool maskSea;uniform sampler2D waterMask;uniform vec4 waterBounds;out vec4 color;void main(){if(maskSea){vec2 uv=(mapUV-waterBounds.xy)/waterBounds.zw;if(any(lessThan(uv,vec2(0.0)))||any(greaterThan(uv,vec2(1.0)))||texture(waterMask,uv).b<0.5)discard;}color=vec4(0.12,0.35,0.55,1.0);}`));
  gl.linkProgram(this.program);if(!gl.getProgramParameter(this.program,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(this.program));
  this.uniforms=Object.fromEntries(['maskSea','waterMask','waterBounds','matrix','worldSize','national','center','scale','aspect','bearing','tilt','depthRange'].map(k=>[k,gl.getUniformLocation(this.program,k)]));
  this.vao=gl.createVertexArray();this.buffer=gl.createBuffer();
  const previous=gl.getParameter(gl.VERTEX_ARRAY_BINDING);gl.bindVertexArray(this.vao);gl.bindBuffer(gl.ARRAY_BUFFER,this.buffer);const location=gl.getAttribLocation(this.program,'uv');gl.enableVertexAttribArray(location);gl.vertexAttribPointer(location,2,gl.FLOAT,false,0,0);gl.bindVertexArray(previous);
  this.maskVAO=gl.createVertexArray();const quad=gl.createBuffer();gl.bindVertexArray(this.maskVAO);gl.bindBuffer(gl.ARRAY_BUFFER,quad);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array([0,0,1,0,0,1,1,0,1,1,0,1]),gl.STATIC_DRAW);gl.enableVertexAttribArray(location);gl.vertexAttribPointer(location,2,gl.FLOAT,false,0,0);gl.bindVertexArray(previous);
 }
 async load(name){
  if(!this.cache.has(name)){const r=await fetch('/sea-vectors/'+name+'.sea');if(!r.ok)throw Error('海域ベクターを取得できません');const data=await r.arrayBuffer(),view=new DataView(data);if(data.byteLength<8||view.getUint32(0,true)!==0x31414553||data.byteLength!==8+view.getUint32(4,true)*8)throw Error('海域ベクターの形式が不正です');this.cache.set(name,data);this.transferred+=data.byteLength;}
  const data=this.cache.get(name);this.count=(data.byteLength-8)/8;this.bytes=data.byteLength;this.name=name;
  this.gl.bindBuffer(this.gl.ARRAY_BUFFER,this.buffer);this.gl.bufferData(this.gl.ARRAY_BUFFER,new Float32Array(data,8),this.gl.STATIC_DRAW);
 }
 // 元の海域三角形を直接描画する。内周の穴も元データのまま保つ。
 updateDetailed(source){
  const p=source.geometry,r=source.currentRequest;if(!p||!r||!source.hasSea){this.detailedCount=0;return;}
  const key=source.windowKey+':'+r.frame.join(',');if(key===this.detailedKey)return;
  const offset=p.counts.slice(0,4).reduce((a,b)=>a+b,0)*2,points=p.vertices.subarray(offset),out=new Float32Array(points.length),b=r.bounds,f=r.frame;
  for(let i=0;i<points.length;i+=2){out[i]=(b[0]+points[i]*b[2]-f[0])/f[2];out[i+1]=(b[1]+points[i+1]*b[3]-f[1])/f[3];}
  const gl=this.gl,previous=gl.getParameter(gl.VERTEX_ARRAY_BINDING);if(!this.detailedVAO){this.detailedVAO=gl.createVertexArray();this.detailedBuffer=gl.createBuffer();}
  gl.bindVertexArray(this.detailedVAO);gl.bindBuffer(gl.ARRAY_BUFFER,this.detailedBuffer);gl.bufferData(gl.ARRAY_BUFFER,out,gl.STATIC_DRAW);const a=gl.getAttribLocation(this.program,'uv');gl.enableVertexAttribArray(a);gl.vertexAttribPointer(a,2,gl.FLOAT,false,0,0);gl.bindVertexArray(previous);this.detailedCount=out.length/2;this.detailedKey=key;
 }
 draw(options){
  if(options.source)this.updateDetailed(options.source);const detailed=!!options.source?.hasSea&&!!this.detailedVAO;if(!this.count&&!options.mask&&!detailed)return;const gl=this.gl,program=gl.getParameter(gl.CURRENT_PROGRAM),vao=gl.getParameter(gl.VERTEX_ARRAY_BINDING),u=this.uniforms;
  gl.useProgram(this.program);gl.bindVertexArray(detailed?this.detailedVAO:options.mask?this.maskVAO:this.vao);gl.uniform1i(u.maskSea,!detailed&&!!options.mask);gl.uniform1i(u.waterMask,6);gl.uniform4fv(u.waterBounds,options.bounds||[0,0,1,1]);gl.uniform1f(u.depthRange,options.depthRange||2*Math.max(...options.worldSize));gl.uniform1i(u.national,!!options.national);gl.uniform2f(u.worldSize,...options.worldSize);
  if(options.national){gl.uniform2f(u.center,...options.center);for(const k of ['scale','aspect','bearing','tilt'])gl.uniform1f(u[k],options[k]);}else gl.uniformMatrix4fv(u.matrix,false,options.matrix);
  gl.drawArrays(gl.TRIANGLES,0,detailed?this.detailedCount:options.mask?6:this.count);gl.bindVertexArray(vao);gl.useProgram(program);
 }
};

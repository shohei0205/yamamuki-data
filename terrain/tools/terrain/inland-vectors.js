/* 河川線と、穴を保った湖沼・河川水面を独立して描画する。 */
'use strict';
window.InlandVectors=class {
 constructor(gl){
  this.gl=gl;this.cache=new Map();this.counts=[0,0,0];this.features=[0,0,0];this.bytes=0;this.transferred=0;this.bounds=[0,0,1,1];this.hasSea=false;this.seaCounts=[0,0];this.packetCache=new Map();this.loading=false;this.error='';this.rasterCrop=[0,0,1,1];this.rasterCount=0;
  const shader=(type,source)=>{const s=gl.createShader(type);gl.shaderSource(s,source);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;};
  this.program=gl.createProgram();
  gl.attachShader(this.program,shader(gl.VERTEX_SHADER,`#version 300 es
  precision highp float;in vec2 uv;uniform vec4 rasterCrop;void main(){gl_Position=vec4((uv-rasterCrop.xy)/rasterCrop.zw*2.0-1.0,0.0,1.0);}`));
  gl.attachShader(this.program,shader(gl.FRAGMENT_SHADER,`#version 300 es
  precision highp float;uniform vec3 layerMask;out vec4 color;void main(){color=vec4(layerMask,1.0);}`));
  gl.linkProgram(this.program);if(!gl.getProgramParameter(this.program,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(this.program));
  this.maskLocation=gl.getUniformLocation(this.program,'layerMask');this.cropLocation=gl.getUniformLocation(this.program,'rasterCrop');
  this.size=this.desiredSize();this.texture=gl.createTexture();this.framebuffer=gl.createFramebuffer();
  gl.activeTexture(gl.TEXTURE6);gl.bindTexture(gl.TEXTURE_2D,this.texture);gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA8,this.size,this.size,0,gl.RGBA,gl.UNSIGNED_BYTE,null);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.LINEAR_MIPMAP_LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);
  this.vao=gl.createVertexArray();this.buffer=gl.createBuffer();const old=gl.getParameter(gl.VERTEX_ARRAY_BINDING);gl.bindVertexArray(this.vao);gl.bindBuffer(gl.ARRAY_BUFFER,this.buffer);const attribute=gl.getAttribLocation(this.program,'uv');gl.enableVertexAttribArray(attribute);gl.vertexAttribPointer(attribute,2,gl.FLOAT,false,0,0);gl.bindVertexArray(old);

 }
 async load(name){
  if(!this.cache.has(name)){const r=await fetch('/inland-vectors/'+name+'.inland');if(!r.ok)throw Error('河川・湖沼ベクターを取得できません');const data=await r.arrayBuffer(),view=new DataView(data);if(data.byteLength<8||view.getUint32(0,true)!==0x314c4e49)throw Error('河川・湖沼ベクターの形式が不正です');const length=view.getUint32(4,true);if(length>data.byteLength-8)throw Error('河川・湖沼の見出しが不正です');const header=JSON.parse(new TextDecoder().decode(new Uint8Array(data,8,length))),body=data.slice(8+length);if(header.counts.length!==3||header.counts.some(n=>!Number.isInteger(n)||n<0)||header.counts.reduce((a,b)=>a+b,0)*8!==body.byteLength||header.counts[0]%2||header.counts[1]%3||header.counts[2]%3)throw Error('河川・湖沼の頂点数が不正です');this.cache.set(name,{header,body,bytes:data.byteLength});this.transferred+=data.byteLength;}
  const {header,body,bytes}=this.cache.get(name);this.hasSea=false;this.seaCounts=[0,0];this.bounds=[0,0,1,1];this.counts=header.counts;this.features=header.features;this.zoom=header.zoom;this.bytes=bytes;this.name=name;this.gl.bindBuffer(this.gl.ARRAY_BUFFER,this.buffer);this.gl.bufferData(this.gl.ARRAY_BUFFER,body,this.gl.STATIC_DRAW);this.rasterize();
 }
 desiredSize(){const value=document.getElementById('waterQuality')?.value||'auto',pixels=Math.max(innerWidth,innerHeight)*Math.min(devicePixelRatio||1,1.5),wanted=value==='auto'?2**Math.ceil(Math.log2(pixels*2)):Number(value);return Math.min(4096,this.gl.getParameter(this.gl.MAX_TEXTURE_SIZE),wanted);}
 setBounds(request){this.currentRequest=request;const b=request.maskBounds||request.bounds,f=request.frame,d=request.bounds;const key=b.join(',');const changed=key!==this.cropKey;this.cropKey=key;this.bounds=[(b[0]-f[0])/f[2],(b[1]-f[1])/f[3],b[2]/f[2],b[3]/f[3]];this.rasterCrop=[(b[0]-d[0])/d[2],(b[1]-d[1])/d[3],b[2]/d[2],b[3]/d[3]];return changed;}
 async loadWindow(request,onChange=()=>{}){
  if(!request)return;this.desiredRequest=request;this.wantedKey=request.key;this.loading=true;this.abort?.abort();this.abort=new AbortController();
  try{
   if(!this.packetCache.has(request.key)){
    let data;
    if(window.localWaterSource)data=await window.localWaterSource(request);
    else{const query=new URLSearchParams(Object.fromEntries(['z','seaZ','riverZ','x0','x1','y0','y1'].map(k=>[k,request[k]]))),r=await fetch('/water-tiles?'+query,{signal:this.abort.signal});
    if(!r.ok)throw Error('詳細な水域タイルを取得できません');data=await r.arrayBuffer();}this.transferred+=data.byteLength;
    const decoded=WaterTileClient.decode(data,request);this.packetCache.set(request.key,decoded);while(this.packetCache.size>8)this.packetCache.delete(this.packetCache.keys().next().value);
   }
   if(request.key!==this.wantedKey)return;
   const p=this.packetCache.get(request.key);this.geometry=p;this.counts=p.counts.slice(0,3);this.features=p.features;this.seaCounts=p.counts.slice(3);this.hasSea=p.seaTiles>0;this.zoom=request.z;this.seaZoom=request.seaZ||request.z;this.riverZoom=request.riverZ||request.z;this.bytes=p.bytes;this.setBounds(this.desiredRequest);this.windowKey=request.key;
   this.gl.bindBuffer(this.gl.ARRAY_BUFFER,this.buffer);this.gl.bufferData(this.gl.ARRAY_BUFFER,p.vertices,this.gl.STATIC_DRAW);this.rasterize();this.error='';
  }catch(e){if(e.name!=='AbortError'&&request.key===this.wantedKey)this.error=e.message;}
  finally{if(request.key===this.wantedKey){this.loading=false;onChange();}}
 }
 schedule(frame,view,widthKm,onChange){
  const request=WaterTileClient.request(frame,view,widthKm);if(!request)return;
  this.desiredRequest=request;if(request.key===this.windowKey){const boundsChanged=this.setBounds(request);if(boundsChanged||this.desiredSize()!==this.size)this.rasterize();return;}
  if(request.key===this.wantedKey)return;this.wantedKey=request.key;this.loading=true;clearTimeout(this.timer);
  this.timer=setTimeout(()=>this.loadWindow(this.desiredRequest,onChange),120);
 }
 // ベクターを GPU 上の一時マスクへ描き、地形面の陰影に重ねる。
 // 高さの異なる大きな水面三角形を直接投影せず、地形との縞や隠れのずれを防ぐ。
 rasterize(){
  const gl=this.gl,program=gl.getParameter(gl.CURRENT_PROGRAM),vao=gl.getParameter(gl.VERTEX_ARRAY_BINDING),frame=gl.getParameter(gl.FRAMEBUFFER_BINDING),viewport=gl.getParameter(gl.VIEWPORT),clear=gl.getParameter(gl.COLOR_CLEAR_VALUE),depth=gl.isEnabled(gl.DEPTH_TEST),blend=gl.isEnabled(gl.BLEND);
  const size=this.desiredSize();if(size!==this.size){this.size=size;gl.activeTexture(gl.TEXTURE6);gl.bindTexture(gl.TEXTURE_2D,this.texture);gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA8,size,size,0,gl.RGBA,gl.UNSIGNED_BYTE,null);}
  this.rasterCount++;gl.bindFramebuffer(gl.FRAMEBUFFER,this.framebuffer);gl.framebufferTexture2D(gl.FRAMEBUFFER,gl.COLOR_ATTACHMENT0,gl.TEXTURE_2D,this.texture,0);if(gl.checkFramebufferStatus(gl.FRAMEBUFFER)!==gl.FRAMEBUFFER_COMPLETE)throw Error('水域描画用の GPU メモリーを確保できません');
  gl.viewport(0,0,this.size,this.size);gl.disable(gl.DEPTH_TEST);gl.disable(gl.BLEND);gl.clearColor(0,0,this.hasSea?1:0,0);gl.clear(gl.COLOR_BUFFER_BIT);gl.useProgram(this.program);gl.uniform4fv(this.cropLocation,this.rasterCrop);gl.bindVertexArray(this.vao);
  if(this.hasSea){
   const offset=this.counts.reduce((a,b)=>a+b,0);gl.colorMask(false,false,true,false);gl.uniform3f(this.maskLocation,0,0,0);gl.drawArrays(gl.TRIANGLES,offset,this.seaCounts[0]);gl.uniform3f(this.maskLocation,0,0,1);gl.drawArrays(gl.TRIANGLES,offset+this.seaCounts[0],this.seaCounts[1]);
  }
  gl.colorMask(true,true,false,true);
  // 中心線には川幅がないため描かず、水面ポリゴンの実形状だけを使う。

  gl.uniform3f(this.maskLocation,0,1,0);gl.drawArrays(gl.TRIANGLES,this.counts[0],this.counts[1]);
  gl.uniform3f(this.maskLocation,1,0,0);gl.drawArrays(gl.TRIANGLES,this.counts[0]+this.counts[1],this.counts[2]);
  gl.colorMask(true,true,true,true);gl.bindFramebuffer(gl.FRAMEBUFFER,frame);gl.activeTexture(gl.TEXTURE6);gl.bindTexture(gl.TEXTURE_2D,this.texture);gl.generateMipmap(gl.TEXTURE_2D);gl.viewport(...viewport);gl.clearColor(...clear);if(depth)gl.enable(gl.DEPTH_TEST);if(blend)gl.enable(gl.BLEND);gl.bindVertexArray(vao);gl.useProgram(program);
 }
 bind(){this.gl.activeTexture(this.gl.TEXTURE6);this.gl.bindTexture(this.gl.TEXTURE_2D,this.texture);}
};

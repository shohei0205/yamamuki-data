'use strict';
const RidgeSurface=(()=>{
const generationVertex=`#version 300 es
precision highp float;precision highp int;
layout(location=0) in vec2 xy;
uniform highp usampler2D lookup;uniform highp sampler2D weights,controls;
uniform int lookupWidth,weightsWidth,controlsWidth;
out vec3 generatedPosition;out vec3 generatedNormal;
float height(int i){return texelFetch(controls,ivec2(i%controlsWidth,i/controlsWidth),0).r;}
void main(){uvec2 row=texelFetch(lookup,ivec2(gl_VertexID%lookupWidth,gl_VertexID/lookupWidth),0).rg;int begin=int(row.x),count=int(row.y);float origin=0.;
 for(int j=0;j<256;j++){if(j>=count)break;int i=begin+j;vec4 w=texelFetch(weights,ivec2(i%weightsWidth,i/weightsWidth),0);if(w.y!=0.){origin=height(int(w.x));break;}}
 float h=origin,hComp=0.;vec2 gradient=vec2(0.),gComp=vec2(0.);
 for(int j=0;j<256;j++){if(j>=count)break;int i=begin+j;vec4 w=texelFetch(weights,ivec2(i%weightsWidth,i/weightsWidth),0);float z=height(int(w.x))-origin;if(w.y!=0.){float v=z*w.y-hComp,next=h+v;hComp=(next-h)-v;h=next;}vec2 v=z*w.zw-gComp,next=gradient+v;gComp=(next-gradient)-v;gradient=next;}
 generatedPosition=vec3(xy.x,h,xy.y);generatedNormal=vec3(-gradient.x,1.,-gradient.y);gl_Position=vec4(0.,0.,0.,1.);
}`;
const vertex=`#version 300 es
precision highp float;layout(location=0) in vec3 point;layout(location=1) in vec3 gradient;uniform mat4 matrix;uniform int mode;out vec3 position;out vec3 normal;
void main(){position=point;normal=gradient;vec3 p=point;if(mode>0)p.y+=mode==2?12.:2.;gl_Position=matrix*vec4(p,1.);gl_PointSize=7.;}`;
const fragment=`#version 300 es
precision highp float;precision highp int;in vec3 position;in vec3 normal;uniform int mode,smoothMode;uniform vec3 lineColor;out vec4 color;
void main(){if(mode>0){if(mode==2&&length(gl_PointCoord-.5)>.5)discard;color=vec4(lineColor,1.);return;}vec3 n=smoothMode==1?normalize(normal):normalize(cross(dFdx(position),dFdy(position)));if(n.y<0.)n=-n;vec3 light=normalize(vec3(-.5,1.,-.6));float shade=clamp(.82+dot(n,light)-light.y,.15,1.2);vec3 tint=mix(vec3(.40,.49,.32),vec3(.69,.65,.51),clamp(position.y/3800.,0.,1.));color=vec4(clamp(tint*shade,0.,1.),1.);}`;
function create(gl){
 const shader=(type,text)=>{const s=gl.createShader(type);gl.shaderSource(s,text);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;};
 function program(v,f,varyings){const p=gl.createProgram();gl.attachShader(p,shader(gl.VERTEX_SHADER,v));gl.attachShader(p,shader(gl.FRAGMENT_SHADER,f));if(varyings)gl.transformFeedbackVaryings(p,varyings,gl.INTERLEAVED_ATTRIBS);gl.linkProgram(p);if(!gl.getProgramParameter(p,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(p));return p;}
 const generate=program(generationVertex,'#version 300 es\nprecision highp float;out vec4 color;void main(){color=vec4(0.);}', ['generatedPosition','generatedNormal']),drawProgram=program(vertex,fragment);
 const gu=Object.fromEntries(['lookup','weights','controls','lookupWidth','weightsWidth','controlsWidth'].map(k=>[k,gl.getUniformLocation(generate,k)])),du=Object.fromEntries(['matrix','mode','smoothMode','lineColor'].map(k=>[k,gl.getUniformLocation(drawProgram,k)]));
 const input=gl.createBuffer(),geometry=gl.createBuffer(),inputVAO=gl.createVertexArray(),meshVAO=gl.createVertexArray();let model=null,showLines=true,indexCount=0,textures=[],elementBuffers={};
 function decode(value,Type){return new Type(Uint8Array.from(atob(value),c=>c.charCodeAt(0)).buffer);}
 function texture(unit,array,components,integer=false){const tex=gl.createTexture();textures.push(tex);const width=Math.min(1024,Math.max(1,array.length/components)),height=Math.ceil(array.length/components/width);if(height>gl.getParameter(gl.MAX_TEXTURE_SIZE))throw Error('表示用ステンシルがGPUの上限を超えます');const padded=new (integer?Uint32Array:Float32Array)(width*height*components);padded.set(array);gl.activeTexture(gl.TEXTURE0+unit);gl.bindTexture(gl.TEXTURE_2D,tex);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);gl.texImage2D(gl.TEXTURE_2D,0,integer?gl.RG32UI:components===4?gl.RGBA32F:gl.R32F,width,height,0,integer?gl.RG_INTEGER:components===4?gl.RGBA:gl.RED,integer?gl.UNSIGNED_INT:gl.FLOAT,padded);return width;}
 function elements(name,data){if(!elementBuffers[name])elementBuffers[name]=gl.createBuffer();gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,elementBuffers[name]);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,data,gl.STATIC_DRAW);return data.length;}
 return {
 upload(value){model=value;for(const t of textures)gl.deleteTexture(t);textures=[];const m=model.mesh,xy=decode(m.xy,Float32Array),indices=decode(m.indices,Uint32Array);gl.useProgram(generate);
  const widths=[texture(0,decode(m.lookup,Uint32Array),2,true),texture(1,decode(m.weights,Float32Array),4),texture(2,decode(m.controls,Float32Array),1)];for(const[k,v]of [['lookup',0],['weights',1],['controls',2]])gl.uniform1i(gu[k],v);gl.uniform1i(gu.lookupWidth,widths[0]);gl.uniform1i(gu.weightsWidth,widths[1]);gl.uniform1i(gu.controlsWidth,widths[2]);
  gl.bindVertexArray(inputVAO);gl.bindBuffer(gl.ARRAY_BUFFER,input);gl.bufferData(gl.ARRAY_BUFFER,xy,gl.STATIC_DRAW);gl.enableVertexAttribArray(0);gl.vertexAttribPointer(0,2,gl.FLOAT,false,8,0);gl.bindBuffer(gl.TRANSFORM_FEEDBACK_BUFFER,geometry);gl.bufferData(gl.TRANSFORM_FEEDBACK_BUFFER,m.vertexCount*24,gl.STATIC_DRAW);gl.bindBufferBase(gl.TRANSFORM_FEEDBACK_BUFFER,0,geometry);gl.enable(gl.RASTERIZER_DISCARD);gl.beginTransformFeedback(gl.POINTS);gl.drawArrays(gl.POINTS,0,m.vertexCount);gl.endTransformFeedback();gl.disable(gl.RASTERIZER_DISCARD);gl.bindBufferBase(gl.TRANSFORM_FEEDBACK_BUFFER,0,null);
  gl.bindVertexArray(meshVAO);gl.bindBuffer(gl.ARRAY_BUFFER,geometry);gl.enableVertexAttribArray(0);gl.vertexAttribPointer(0,3,gl.FLOAT,false,24,0);gl.enableVertexAttribArray(1);gl.vertexAttribPointer(1,3,gl.FLOAT,false,24,12);indexCount=elements('mesh',indices);const wire=new Uint32Array(indices.length*2);for(let i=0;i<indices.length;i+=3)wire.set([indices[i],indices[i+1],indices[i+1],indices[i+2],indices[i+2],indices[i]],i*2);elements('wire',wire);
  for(const k of ['ridgeLines','valleyLines','peaks']){const data=decode(m[k],Uint32Array);m[k+'Count']=elements(k,data);}gl.bindVertexArray(null);gl.activeTexture(gl.TEXTURE0);
 },setLines(v){showLines=v;},
 draw(matrix,camera,viewport,wire,smooth=true){if(!model)return;gl.useProgram(drawProgram);gl.uniformMatrix4fv(du.matrix,false,matrix);gl.uniform1i(du.mode,0);gl.uniform1i(du.smoothMode,smooth?1:0);gl.bindVertexArray(meshVAO);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,elementBuffers.mesh);gl.disable(gl.CULL_FACE);if(wire){gl.enable(gl.POLYGON_OFFSET_FILL);gl.polygonOffset(1,1);}gl.drawElements(gl.TRIANGLES,indexCount,gl.UNSIGNED_INT,0);gl.disable(gl.POLYGON_OFFSET_FILL);
  if(wire){gl.uniform1i(du.mode,1);gl.uniform3f(du.lineColor,.08,.08,.08);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,elementBuffers.wire);gl.drawElements(gl.LINES,indexCount*2,gl.UNSIGNED_INT,0);}
  if(showLines){gl.uniform1i(du.mode,1);for(const[k,color]of [['ridgeLines',[1.,.42,.1]],['valleyLines',[.1,.65,1.]]]){gl.uniform3fv(du.lineColor,color);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,elementBuffers[k]);gl.drawElements(gl.LINES,model.mesh[k+'Count'],gl.UNSIGNED_INT,0);}gl.uniform1i(du.mode,2);gl.uniform3f(du.lineColor,1.,.8,.1);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,elementBuffers.peaks);gl.drawElements(gl.POINTS,model.mesh.peaksCount,gl.UNSIGNED_INT,0);}
 },
 profile(xs){if(!model)return xs.map(()=>0);return xs.map(x=>{const t=Math.max(0,Math.min(512,(x+15680)/31360*512)),i=Math.min(511,Math.floor(t));return model.mesh.profile[i]*(1-t+i)+model.mesh.profile[i+1]*(t-i);});},
 readGeometry(){const values=new Float32Array(model.mesh.vertexCount*6);gl.bindBuffer(gl.ARRAY_BUFFER,geometry);gl.getBufferSubData(gl.ARRAY_BUFFER,0,values);return values;},get info(){return model?.info;},get stats(){return model?.mesh;},get summit(){return model?.summit;}
 };
}
return {create,generationVertexShader:generationVertex};
})();

const nativeTiles=new URL(location.href).searchParams.get('tiles')==='2';
const region=new URL(location.href).searchParams.get('region')==='tsukuba'?'tsukuba':'yari',regionPrefix=(nativeTiles?'/tile-regions/':'/regions/')+region;
let dynamicJXL=null,seaVectors,solarControls,inlandVectors,inlandReported=0;
/* 地域比較用。形状と法線をブラウザーで復元し、同じ視点で比較する。 */
'use strict';
const $=id=>document.getElementById(id),canvas=$('terrain'),gl=canvas.getContext('webgl2',{antialias:true});
if(!gl)throw Error('WebGL 2 が必要です');
let meta,program,vertexBuffer,indexBuffer,indexCount=0,currentStep=0,bytes=0,loaded=false,patchSize=256,heightTexture,dataPrefix='',levelInfos=[],normalVariants=[];
let sourceMeta,heightTextureLevel=null,polygonGeometry=null,heightLevelPreference='14';
let heightFloat=false,selectedHeightBytes=0,heightDecodeMs=0,normalInfo=null;
let heightASTC=false,heightASTCInfo=null,heightTileTexture,heightTileParams,heightValidTexture;
const astcExtension=gl.getExtension('WEBGL_compressed_texture_astc');
const polygonCache=new Map();
function polygonLevel(){const value=$('geometryMode').value;return value.startsWith('polygon')?Number(value.slice(7)):null;}
let target=[0,0,region==='tsukuba'?250:1500],distance=24000,bearing=0,tilt=55,frames=0,fps=0,lastFPS=performance.now();
const rad=Math.PI/180;
function shader(type,source){const s=gl.createShader(type);gl.shaderSource(s,source);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;}
function multiply(a,b){const out=new Float32Array(16);for(let c=0;c<4;c++)for(let r=0;r<4;r++)for(let k=0;k<4;k++)out[c*4+r]+=a[k*4+r]*b[c*4+k];return out;}
function normalize(a){const n=Math.hypot(...a);return a.map(x=>x/n);}
function cross(a,b){return [a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]];}
function dot(a,b){return a.reduce((s,x,i)=>s+x*b[i],0);}
function cameraMatrix(){
 const t=tilt*rad,b=bearing*rad,eye=[target[0]+distance*Math.sin(t)*Math.sin(b),target[1]-distance*Math.sin(t)*Math.cos(b),target[2]+distance*Math.cos(t)];
 const z=normalize(eye.map((v,i)=>v-target[i])),x=normalize(cross([0,0,1],z)),y=cross(z,x);
 const view=new Float32Array([x[0],y[0],z[0],0,x[1],y[1],z[1],0,x[2],y[2],z[2],0,-dot(x,eye),-dot(y,eye),-dot(z,eye),1]);
 const extent=meta?meta.width*meta.meters:30000,near=Math.max(10,distance*.001),far=Math.max(400000,distance+extent*4),f=1/Math.tan(25*rad),a=canvas.width/canvas.height;
 const p=new Float32Array([f/a,0,0,0,0,f,0,0,0,0,(far+near)/(near-far),-1,0,0,2*far*near/(near-far),0]);
 return multiply(p,view);
}
function buildGrid(step){
 if(step===currentStep)return;
 currentStep=step;const n=Math.ceil(patchSize/step),points=[],indices=[];
 for(let y=0;y<=n;y++)for(let x=0;x<=n;x++)points.push(Math.min(x*step,patchSize),Math.min(y*step,patchSize));
 for(let y=0;y<n;y++)for(let x=0;x<n;x++){const a=y*(n+1)+x,b=a+1,c=a+n+1,d=c+1;indices.push(a,c,b,b,c,d);}
 if(vertexBuffer)gl.deleteBuffer(vertexBuffer);if(indexBuffer)gl.deleteBuffer(indexBuffer);
 vertexBuffer=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,vertexBuffer);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array(points),gl.STATIC_DRAW);
 const position=gl.getAttribLocation(program,'gridPosition');gl.enableVertexAttribArray(position);gl.vertexAttribPointer(position,2,gl.FLOAT,false,0,0);
 indexBuffer=gl.createBuffer();gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,indexBuffer);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,new Uint32Array(indices),gl.STATIC_DRAW);indexCount=indices.length;
}
const common=`
uniform highp sampler2D heightMap;
uniform vec2 dimensions;
uniform float heightOffset;
uniform float meters;
uniform bool polygonMode;
uniform bool heightFloat;
uniform bool heightASTC;
uniform bool heightHasMask;
uniform highp sampler2DArray heightTileMap;
uniform highp sampler2D heightTileParams;
uniform highp sampler2D heightValidMap;
uniform int heightTileCore;
uniform int heightTilePadding;
uniform int heightTileColumns;
bool heightMissing(ivec2 p){
 if(!heightHasMask)return false;
 int bits=int(floor(texelFetch(heightValidMap,ivec2(p.x/8,p.y),0).r*255.0+0.5));
 return (bits&(1<<(p.x%8)))==0;
}
float rawHeight(ivec2 p){
 p=clamp(p,ivec2(0),ivec2(dimensions)-1);
 if(heightASTC){
  if(heightMissing(p))return 0.0;
  ivec2 tile=p/heightTileCore,local=p-tile*heightTileCore;
  float h=texelFetch(heightTileMap,ivec3(local+heightTilePadding,tile.y*heightTileColumns+tile.x),0).r;
  vec2 scale=texelFetch(heightTileParams,tile,0).rg;return scale.x+h*scale.y;
 }
 vec3 c=texelFetch(heightMap,p,0).rgb;
 if(heightFloat)return c.r<-32000.0?0.0:c.r;
 if(c.b<0.5)return 0.0;
 return floor(c.r*255.0+0.5)+256.0*floor(c.g*255.0+0.5)+heightOffset;
}
float elevation(vec2 p){
 vec2 q=clamp(p,vec2(0.0),dimensions-1.0),f=fract(q);ivec2 a=ivec2(floor(q));
 return mix(mix(rawHeight(a),rawHeight(a+ivec2(1,0)),f.x),mix(rawHeight(a+ivec2(0,1)),rawHeight(a+ivec2(1,1)),f.x),f.y);
}`;
function makeProgram(){
 program=gl.createProgram();
 gl.attachShader(program,shader(gl.VERTEX_SHADER,`#version 300 es
 precision highp float;precision highp int;
 in vec2 gridPosition;in vec3 polygonPosition;in vec3 polygonNormal;uniform mat4 viewProjection;uniform int columns;uniform float patchSize;
 out vec2 sourcePixel;out vec3 worldPosition;out vec3 baseNormal;
 ${common}
 void main(){
  if(polygonMode){
   worldPosition=polygonPosition;baseNormal=polygonNormal;
   sourcePixel=vec2(worldPosition.x,-worldPosition.y)/meters+(dimensions-1.0)*0.5;
  }else{
   baseNormal=vec3(0.0,0.0,1.0);
   ivec2 tile=ivec2(gl_InstanceID%columns,gl_InstanceID/columns);
   sourcePixel=min(vec2(tile)*patchSize+gridPosition,dimensions-1.0);
   worldPosition=vec3((sourcePixel.x-(dimensions.x-1.0)*0.5)*meters,((dimensions.y-1.0)*0.5-sourcePixel.y)*meters,elevation(sourcePixel));
  }
  gl_Position=viewProjection*vec4(worldPosition,1.0);
 }`));
 gl.attachShader(program,shader(gl.FRAGMENT_SHADER,`#version 300 es
 precision highp float;precision highp int;
 in vec2 sourcePixel;in vec3 worldPosition;in vec3 baseNormal;out vec4 outputColor;
 uniform vec4 waterBounds;uniform bool detailedWater;uniform bool showSea;uniform highp sampler2D inlandMap;uniform bool showRivers;uniform bool showLakes;uniform highp sampler2D normalMap;uniform int normalMode;uniform float normalStep;uniform vec3 lightDirection;uniform float strength;uniform vec2 normalWorldSize;uniform bool normalRA;uniform bool tangentNormal;uniform highp sampler2D patternMap;uniform float patternScale;uniform float patternStrength;uniform vec2 patternOrigin;uniform int tileLines;uniform vec2 heightTileCounts;uniform vec2 normalTileCounts;
 ${common}
 ${SolarPosition.glsl}
 // 直径 0.5 描画ピクセルの円が入らない細い水面を除く。
 // 円の中心をずらして探すため、残す川の縁を一律には削らない。
 float visibleRiver(vec2 uv,vec2 dx,vec2 dy,float coverage){
  if(coverage<=0.0)return 0.0;
  const vec2 ring[9]=vec2[9](vec2(0),vec2(1,0),vec2(-1,0),vec2(0,1),vec2(0,-1),vec2(.707107,.707107),vec2(-.707107,.707107),vec2(.707107,-.707107),vec2(-.707107,-.707107));
  for(int i=0;i<9;i++){
   vec2 center=uv+(dx*ring[i].x+dy*ring[i].y)*0.25;bool fits=true;
   for(int j=0;j<9;j++){
    vec2 p=center+(dx*ring[j].x+dy*ring[j].y)*0.25;
    if(any(lessThan(p,vec2(0)))||any(greaterThan(p,vec2(1)))||textureLod(inlandMap,p,0.0).r<0.5){fits=false;break;}
   }
   if(fits)return coverage;
  }
  return 0.0;
 }

 float gridLine(vec2 uv,vec2 cells){vec2 q=uv*cells,k=floor(q+0.5),d=abs(q-k),a=max(fwidth(q),vec2(0.00001));vec2 line=1.0-smoothstep(a*0.3,a*1.2,d);line*=step(vec2(0.5),k)*step(k,cells-0.5);return max(line.x,line.y);}
 void main(){
  ivec2 p=ivec2(clamp(sourcePixel,vec2(0.0),dimensions-1.0));
  vec2 groundUV=vec2(worldPosition.x,-worldPosition.y)/normalWorldSize+0.5;
  if(polygonMode){if(any(lessThan(groundUV,vec2(0.0)))||any(greaterThan(groundUV,vec2(1.0))))discard;}
  else if(any(lessThan(sourcePixel,vec2(0.0)))||any(greaterThan(sourcePixel,dimensions-1.0)))discard;
  vec2 waterUV=(sourcePixel/(dimensions-1.0)-waterBounds.xy)/waterBounds.zw;vec2 riverDX=dFdx(waterUV),riverDY=dFdy(waterUV);bool inside=all(greaterThanEqual(waterUV,vec2(0.0)))&&all(lessThanEqual(waterUV,vec2(1.0)));vec3 water=inside?texture(inlandMap,waterUV).rgb:vec3(0.0);
  if(showSea&&detailedWater&&inside&&water.b>0.5)discard;
  if(!polygonMode){vec3 heightPixel=texelFetch(heightMap,p,0).rgb;if(heightASTC?heightMissing(p):(heightFloat?heightPixel.r<-32000.0:heightPixel.b<0.5))if(!(showSea&&detailedWater&&inside&&water.b<=0.5))discard;}
  vec3 n;
  if(normalMode==1){
   // 同じ地上位置では標高レベルと形状方式によらず同じ法線を参照する。
   vec2 normalSize=vec2(textureSize(normalMap,0));
   vec2 uv=(groundUV*(normalSize-1.0)+0.5)/normalSize;
   vec4 sampleNormal=texture(normalMap,uv);
   if(tangentNormal){
    vec3 base=normalize(baseNormal),east=vec3(1.0,0.0,0.0);
    vec3 t=east-dot(east,base)*base;
    if(dot(t,t)<0.000001){vec3 north=vec3(0.0,1.0,0.0);t=north-dot(north,base)*base;}
    t=normalize(t);vec3 b=cross(base,t),detail=normalize(sampleNormal.rgb*2.0-1.0);
    n=normalize(t*detail.x+b*detail.y+base*detail.z);
   }else{
   vec2 xy=(normalRA?sampleNormal.ra:sampleNormal.rg)*2.0-1.0;
   n=normalize(vec3(xy,sqrt(max(0.00001,1.0-dot(xy,xy)))));
   }
  }else if(normalMode==2||normalMode==3){
   float dx=(elevation(sourcePixel+vec2(normalStep,0.0))-elevation(sourcePixel-vec2(normalStep,0.0)))/(2.0*normalStep*meters);
   float dy=(elevation(sourcePixel+vec2(0.0,normalStep))-elevation(sourcePixel-vec2(0.0,normalStep)))/(2.0*normalStep*meters);
   if(normalMode==3){
    // 地上位置に固定した共通の高さ模様。実地形の尾根や谷を復元するものではない。
    vec2 uv=(worldPosition.xy+patternOrigin)/patternScale;
    vec2 e=vec2(1.0/256.0,0.0);
    float gx=texture(patternMap,uv+e).r-texture(patternMap,uv-e).r;
    float gy=texture(patternMap,uv+e.yx).r-texture(patternMap,uv-e.yx).r;
    float footprint=max(length(dFdx(uv)),length(dFdy(uv)));
    float fade=1.0-smoothstep(0.015,0.12,footprint);
    float slope=smoothstep(0.02,0.4,length(vec2(dx,dy)));
    dx-=gx*patternStrength*12.0*fade*slope;
    dy+=gy*patternStrength*12.0*fade*slope;
   }
   n=normalize(vec3(-dx,dy,1.0));
  }else{n=normalize(cross(dFdx(worldPosition),dFdy(worldPosition)));if(n.z<0.0)n=-n;}
  vec3 sun=solarEnabled?solarDirection(groundUV):lightDirection;
  float day=solarEnabled?solarDay(sun):1.0;
  float diffuse=(solarEnabled&&sun.z<=0.0)?0.0:max(dot(n,sun),0.0);
  float illumination=mix(1.0,mix(0.04,0.35,day)+0.65*diffuse,strength);
  vec3 color=vec3(0.76,0.79,0.73)*illumination;
  if(tileLines==1||tileLines==3)color=mix(color,vec3(0.9,0.2,0.1),gridLine(groundUV,heightTileCounts)*0.8);
  if(normalMode==1&&(tileLines==2||tileLines==3))color=mix(color,vec3(0.0,0.45,0.85),gridLine(groundUV,normalTileCounts)*0.8);
  float lake=showLakes?water.g:0.0,river=showRivers?visibleRiver(waterUV,riverDX,riverDY,water.r):0.0;
  color=mix(color,vec3(0.18,0.62,0.86),river*(1.0-lake));color=mix(color,vec3(0.20,0.51,0.72),lake);
  outputColor=vec4(color,1.0);
 }`));
 gl.linkProgram(program);if(!gl.getProgramParameter(program,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(program));gl.useProgram(program);
}
let uniforms;
const normalTextures=new Map();const polygonNormalReports=new Map();
async function polygonNormalReport(source){if(!polygonNormalReports.has(source))polygonNormalReports.set(source,await(await fetch('/'+(source==='baked'?'baked':'mesh')+'-normals/report.json')).json());return polygonNormalReports.get(source);}
async function uploadPNG(url,unit,filter){
 const response=await fetch(url),blob=await response.blob();bytes+=blob.size;
 const image=await createImageBitmap(blob,{colorSpaceConversion:'none',premultiplyAlpha:'none'});
 const texture=gl.createTexture();gl.activeTexture(gl.TEXTURE0+unit);gl.bindTexture(gl.TEXTURE_2D,texture);
 gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL,gl.NONE);
 gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA8,gl.RGBA,gl.UNSIGNED_BYTE,image);
 gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,filter);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,filter);
 gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);image.close();return texture;
}
// 小さい共通テクスチャを端末内で生成する。端がつながる周波数だけを合成する。
function initPattern(){
 const data=new Uint8Array(256*256);let seed=719;
 const waves=Array.from({length:32},(_,i)=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;const a=(seed%15)-7;seed=(Math.imul(seed,1664525)+1013904223)>>>0;return [a,(seed%15)-7,i*2.399,1/(2+Math.hypot(a,(seed%15)-7))];});
 for(let y=0;y<256;y++)for(let x=0;x<256;x++){let v=0;for(const [a,b,phase,amp] of waves)v+=Math.sin((a*x+b*y)*Math.PI/128+phase)*amp;data[y*256+x]=Math.round(Math.max(0,Math.min(255,128+v*30)));}
 const t=gl.createTexture();gl.activeTexture(gl.TEXTURE5);gl.bindTexture(gl.TEXTURE_2D,t);gl.texImage2D(gl.TEXTURE_2D,0,gl.R8,256,256,0,gl.RED,gl.UNSIGNED_BYTE,data);gl.generateMipmap(gl.TEXTURE_2D);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.LINEAR_MIPMAP_LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.REPEAT);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.REPEAT);gl.uniform1i(uniforms.patternMap,5);gl.activeTexture(gl.TEXTURE1);gl.bindTexture(gl.TEXTURE_2D,t);
}
async function selectNormal(){
 if($('normal').value==='pattern'){
  // 法線画像を取得せず、未使用サンプラーには共通テクスチャを割り当てる。

  if(!normalInfo)normalInfo={size:Number($('normalResolution').value),bytes:0,codec:'png',hardware:true,source:'shared',variants:normalVariants};
  return;
 }
 const size=$('normalResolution').value,codec=$('normalCodec').value,level=polygonLevel(),source=level&&$('normalSource').value!=='shared'?$('normalSource').value:'shared',key=(nativeTiles?'tiles-L'+meta.level+'-':'')+source+'-'+(source!=='shared'?level:'all')+'-'+codec+'-'+size+(codec==='dynamic-jxl'?'-'+dynamicJXL?.id:'');$('normalResolution').disabled=true;$('normalCodec').disabled=true;$('normalSource').disabled=true;
 try{
  if(!normalTextures.has(key)){
   const meshLevel=source!=='shared'?(await polygonNormalReport(source)).levels.find(m=>m.level===level):null;
   if(source!=='shared'&&!meshLevel)throw Error('この LOD の法線はまだ生成されていません');
   const variant=(meshLevel?meshLevel.normalVariants:normalVariants).find(n=>String(n.size)===size);let result;
   if(codec==='dynamic-jxl'){if(!dynamicJXL||Number(size)!==dynamicJXL.normalSize)throw Error('現在の法線解像度で圧縮を実行してください');result={texture:await uploadPNG(dynamicJXL.normalURL,1,gl.LINEAR),bytes:dynamicJXL.normalBytes,ra:false,hardware:true,gpuBytes:Number(size)**2*4};}
   else if(codec==='png')result={texture:await uploadPNG(variant.url,1,gl.LINEAR),bytes:variant.bytes,ra:false,hardware:true,gpuBytes:Number(size)**2*4};
   else{
    const block=Number(codec.match(/astc(\d)/)[1]),r=await TerrainCodecs.report(),row=(meshLevel?meshLevel.compressed:r.normals?.results).find(n=>String(n.size)===size&&n.block===block);if(!row)throw Error('この法線の圧縮結果がありません');
    const compressed=codec.endsWith('zstd'),data=await TerrainCodecs.bytes(compressed?row.zstdUrl:row.url,n=>bytes+=n),payload=TerrainCodecs.ktxPayload(data);let texture;
    if(astcExtension){
     texture=gl.createTexture();gl.activeTexture(gl.TEXTURE1);gl.bindTexture(gl.TEXTURE_2D,texture);
     gl.compressedTexImage2D(gl.TEXTURE_2D,0,astcExtension[`COMPRESSED_RGBA_ASTC_${block}x${block}_KHR`],Number(size),Number(size),0,payload);
     gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);
    }else texture=await uploadPNG(row.previewUrl,1,gl.LINEAR);
    result={texture,bytes:compressed?row.ktx2ZstdBytes:row.ktx2Bytes,ra:!!astcExtension&&source!=='baked',hardware:!!astcExtension,gpuBytes:astcExtension?row.gpuPayloadBytes:Number(size)**2*4,row};
   }
   result.codec=codec;result.size=Number(size);result.source=source;result.level=source!=='shared'?level:null;result.tangent=source==='baked';result.variants=meshLevel?meshLevel.normalVariants:normalVariants;normalTextures.set(key,result);
  }
  normalInfo=normalTextures.get(key);
  for(const option of $('normalResolution').options){const v=normalInfo.variants.find(n=>String(n.size)===option.value);option.textContent=`${v.size}×${v.size}（PNG ${(v.bytes/1024).toFixed(1)} KB）`;}
  gl.activeTexture(gl.TEXTURE1);gl.bindTexture(gl.TEXTURE_2D,normalInfo.texture);
 }finally{$('normalResolution').disabled=false;$('normalCodec').disabled=false;$('normalSource').disabled=!polygonLevel();}
}
function uploadFloatHeight(values){
 const pixels=new Float32Array(values);for(let i=0;i<pixels.length;i++)if(!Number.isFinite(pixels[i]))pixels[i]=-32768;
 heightTexture=gl.createTexture();gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,heightTexture);gl.texImage2D(gl.TEXTURE_2D,0,gl.R32F,meta.width,meta.height,0,gl.RED,gl.FLOAT,pixels);
 gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);heightFloat=true;
}
function initHeightASTC(){
 gl.pixelStorei(gl.UNPACK_ALIGNMENT,1);heightTileTexture=gl.createTexture();gl.activeTexture(gl.TEXTURE2);gl.bindTexture(gl.TEXTURE_2D_ARRAY,heightTileTexture);gl.texImage3D(gl.TEXTURE_2D_ARRAY,0,gl.RGBA8,1,1,1,0,gl.RGBA,gl.UNSIGNED_BYTE,new Uint8Array([0,0,0,255]));
 gl.texParameteri(gl.TEXTURE_2D_ARRAY,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D_ARRAY,gl.TEXTURE_MAG_FILTER,gl.NEAREST);
 heightTileParams=gl.createTexture();gl.activeTexture(gl.TEXTURE3);gl.bindTexture(gl.TEXTURE_2D,heightTileParams);gl.texImage2D(gl.TEXTURE_2D,0,gl.RG32F,1,1,0,gl.RG,gl.FLOAT,new Float32Array([0,0]));
 gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);
 heightValidTexture=gl.createTexture();gl.activeTexture(gl.TEXTURE4);gl.bindTexture(gl.TEXTURE_2D,heightValidTexture);gl.texImage2D(gl.TEXTURE_2D,0,gl.R8,1,1,0,gl.RED,gl.UNSIGNED_BYTE,new Uint8Array([255]));
 gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);
 gl.uniform1i(uniforms.heightTileMap,2);gl.uniform1i(uniforms.heightTileParams,3);gl.uniform1i(uniforms.heightValidMap,4);
}
async function ensureHeight(){
 const codec=$('heightCodec').value,key=meta.level+'-'+codec+(codec==='dynamic-jxl'?'-'+dynamicJXL?.id:'');if(heightTextureLevel===key)return;
 if(heightTexture)gl.deleteTexture(heightTexture);heightASTC=false;heightASTCInfo=null;
 if(codec==='dynamic-jxl'){if(!dynamicJXL||meta.level!==dynamicJXL.level)throw Error('現在の標高レベルで圧縮を実行してください');const data=await TerrainCodecs.bytes(dynamicJXL.heightURL,n=>bytes+=n);if(data.byteLength!==meta.width*meta.height*4)throw Error("復号標高の寸法が不正です");uploadFloatHeight(new Float32Array(data.buffer,data.byteOffset,data.byteLength/4));selectedHeightBytes=dynamicJXL.heightBytes;heightDecodeMs=dynamicJXL.decodeMs;}
 else if(codec==='png'){heightTexture=await uploadPNG(dataPrefix+'/height.png',0,gl.NEAREST);heightFloat=false;selectedHeightBytes=meta.heightBytes;heightDecodeMs=0;}
 else if(codec.startsWith('height-astc')){
  const data=await TerrainCodecs.loadHeightASTC(meta.level,codec,n=>bytes+=n),h=data.header;
  if(h.width!==meta.width||h.height!==meta.height)throw Error('標高 ASTC の範囲が違います');
  heightASTC=!!astcExtension&&h.layers<=gl.getParameter(gl.MAX_ARRAY_TEXTURE_LAYERS);heightASTCInfo={row:data.row,hardware:heightASTC};
  if(heightASTC){
   heightTexture=gl.createTexture();gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,heightTexture);gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA8,1,1,0,gl.RGBA,gl.UNSIGNED_BYTE,new Uint8Array([0,0,255,255]));gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);
   gl.activeTexture(gl.TEXTURE2);gl.bindTexture(gl.TEXTURE_2D_ARRAY,heightTileTexture);gl.compressedTexImage3D(gl.TEXTURE_2D_ARRAY,0,astcExtension[`COMPRESSED_RGBA_ASTC_${h.block}x${h.block}_KHR`],h.side,h.side,h.layers,0,data.raw);
   const scales=new Float32Array(h.layers*2);for(let i=0;i<h.layers;i++){scales[i*2]=h.tiles[i].min;scales[i*2+1]=h.tiles[i].range;}
   gl.activeTexture(gl.TEXTURE3);gl.bindTexture(gl.TEXTURE_2D,heightTileParams);gl.texImage2D(gl.TEXTURE_2D,0,gl.RG32F,h.columns,h.rows,0,gl.RG,gl.FLOAT,scales);
   const hasMissing=data.mask.some(v=>!v);gl.uniform1i(uniforms.heightHasMask,hasMissing?1:0);
   if(hasMissing){const maskWidth=Math.ceil(h.width/8),packed=new Uint8Array(maskWidth*h.height);for(let y=0;y<h.height;y++)for(let x=0;x<h.width;x++)if(data.mask[y*h.width+x])packed[y*maskWidth+(x>>3)]|=1<<(x&7);
    gl.activeTexture(gl.TEXTURE4);gl.bindTexture(gl.TEXTURE_2D,heightValidTexture);gl.pixelStorei(gl.UNPACK_ALIGNMENT,1);gl.texImage2D(gl.TEXTURE_2D,0,gl.R8,maskWidth,h.height,0,gl.RED,gl.UNSIGNED_BYTE,packed);
   }
   gl.uniform1i(uniforms.heightTileCore,h.core);gl.uniform1i(uniforms.heightTilePadding,h.padding);gl.uniform1i(uniforms.heightTileColumns,h.columns);heightFloat=false;heightDecodeMs=data.decodeMs;
  }else{const decoded=await TerrainCodecs.loadHeight(meta.level,codec,n=>bytes+=n);uploadFloatHeight(decoded.values);heightDecodeMs=decoded.decodeMs;}
  selectedHeightBytes=data.bytes;
 }else{
  const decoded=await TerrainCodecs.loadHeight(meta.level,codec,n=>bytes+=n);if(decoded.width!==meta.width||decoded.height!==meta.height)throw Error('圧縮標高の範囲が違います');
  uploadFloatHeight(decoded.values);selectedHeightBytes=decoded.bytes;heightDecodeMs=decoded.decodeMs;
 }
 heightTextureLevel=key;
}
function makePolygonNormals(points,indices){
 const count=points.length/3,groups=new Uint32Array(count),ids=new Map(),sum=new Float64Array(points.length);
 for(let i=0;i<count;i++){const key=[0,1,2].map(a=>Math.round(points[i*3+a]*10000)).join(',');if(!ids.has(key))ids.set(key,ids.size);groups[i]=ids.get(key);}
 for(let f=0;f<indices.length;f+=3){const a=indices[f]*3,b=indices[f+1]*3,c=indices[f+2]*3;
  const ux=points[b]-points[a],uy=points[b+1]-points[a+1],uz=points[b+2]-points[a+2],vx=points[c]-points[a],vy=points[c+1]-points[a+1],vz=points[c+2]-points[a+2];
  let nx=uy*vz-uz*vy,ny=uz*vx-ux*vz,nz=ux*vy-uy*vx;if(nz<0){nx=-nx;ny=-ny;nz=-nz;}
  for(let j=0;j<3;j++){const g=groups[indices[f+j]]*3;sum[g]+=nx;sum[g+1]+=ny;sum[g+2]+=nz;}
 }
 const values=new Float32Array(points.length);for(let i=0;i<count;i++){const g=groups[i]*3,l=Math.hypot(sum[g],sum[g+1],sum[g+2]);if(l<1e-15){values[i*3+2]=1;continue;}for(let j=0;j<3;j++)values[i*3+j]=sum[g+j]/l;}
 const buffer=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,buffer);gl.bufferData(gl.ARRAY_BUFFER,values,gl.STATIC_DRAW);return buffer;
}
async function loadPolygon(level){
 const codec=$('meshCodec').value,key=level+'-'+codec;if(polygonCache.has(key))return polygonCache.get(key);
 if(codec!=='original'){
  const decoded=await TerrainCodecs.loadMesh(level,codec,n=>bytes+=n),vertices=gl.createBuffer(),elements=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,vertices);gl.bufferData(gl.ARRAY_BUFFER,decoded.points,gl.STATIC_DRAW);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,elements);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,decoded.indices,gl.STATIC_DRAW);
  const result={vertices,elements,normals:makePolygonNormals(decoded.points,decoded.indices),count:decoded.indices.length,bytes:decoded.bytes,heightError:decoded.heightError,quality:decoded.row,decodeMs:decoded.decodeMs};polygonCache.set(key,result);return result;
 }
 const response=await fetch('/polygon/'+level);if(!response.ok)throw Error('ポリゴンの取得に失敗しました');
 const raw=new Uint8Array(await response.arrayBuffer());bytes+=raw.byteLength;
 const view=new DataView(raw.buffer),headerLength=view.getUint32(4,true);
 if(new TextDecoder().decode(raw.subarray(0,4))!=='YSQ1')throw Error('ポリゴンの形式が不正です');
 const rows=JSON.parse(new TextDecoder().decode(raw.subarray(8,8+headerLength)));
 const points=[],indices=[];let offset=8+headerLength,packed=0,heightError=0;
 for(const row of rows){
  const tile=await decodeMesh(raw.subarray(offset,offset+row.bytes));offset+=row.bytes;packed+=row.bytes;
  heightError=Math.max(heightError,row.heightError);const scale=2**(14-row.geometryZoom),base=points.length/3;
  for(const [x,y,z] of tile.points){
   const sx=(row.x*256+x)*scale-sourceMeta.west*256,sy=(row.y*256+y)*scale-sourceMeta.north*256;
   points.push((sx-(sourceMeta.width-1)*.5)*sourceMeta.meters,((sourceMeta.height-1)*.5-sy)*sourceMeta.meters,z);
  }
  for(const index of tile.indices)indices.push(base+index);
 }
 const vertices=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,vertices);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array(points),gl.STATIC_DRAW);
 const elements=gl.createBuffer();gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,elements);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,new Uint32Array(indices),gl.STATIC_DRAW);
 const result={vertices,elements,normals:makePolygonNormals(new Float32Array(points),new Uint32Array(indices)),count:indices.length,bytes:packed,heightError,decodeMs:0};polygonCache.set(key,result);return result;
}
$('normalResolution').addEventListener('change',()=>selectNormal().catch(console.error));
$('normalSource').addEventListener('change',()=>selectNormal().catch(e=>{$('codecHint').textContent=e.message;console.error(e);}));
$('normalCodec').addEventListener('change',async()=>{
 if($('normalCodec').value==='dynamic-jxl'&&(!dynamicJXL||dynamicJXL.level!==meta.level||dynamicJXL.normalSize!==Number($('normalResolution').value))){await encodeDynamicJXL();return;}
 await selectNormal().catch(e=>{$('codecHint').textContent=e.message;console.error(e);});
});
$('normal').addEventListener('change',async()=>{
 if($('normal').value==='png')await selectNormal();
 if(polygonLevel()&&['gpu','pattern'].includes($('normal').value)){
  loaded=false;try{await ensureHeight();loaded=true;}catch(e){$('stats').textContent=e.message;console.error(e);}
 }
});
function updateSettingsLine(geometryBytes,normalBytes,auxiliaryBytes){
 const text=(id,value=$(id).value)=>[...$(id).options].find(o=>o.value===value)?.textContent||value;
 const mb=value=>(value/1e6).toFixed(3)+' MB',poly=polygonLevel();
 const parts=[poly?`ポリゴン L${meta.level}／${text('meshCodec')}`:`ハイトマップ L${meta.level}／${text('heightCodec')}／格子 ${$('step').value==='auto'?'自動 LOD・':''}${currentStep} 倍間隔`];
 if(dynamicJXL&&($('heightCodec').value==='dynamic-jxl'||normalInfo.codec==='dynamic-jxl'))parts.push(`JPEG XL:境界 ${dynamicJXL.seams}／標高 ${dynamicJXL.heightMode} d=${dynamicJXL.heightDistance}／法線 ${dynamicJXL.normalMode} d=${dynamicJXL.normalDistance}`);
 if($('normal').value==='png')parts.push(`法線 ${text('normalSource',normalInfo.source)}／${normalInfo.size}×${normalInfo.size}／${text('normalCodec',normalInfo.codec)}${normalInfo.hardware?'':'（PNG で代替表示）'}`);
 else if($('normal').value==='pattern')parts.push(`共通模様 ${$('patternScale').value} m／強さ ${$('patternStrength').value}`);
 else parts.push(`陰影 ${text('normal')}${$('normal').value==='gpu'?'／参照間隔 '+$('normalStep').value+' 点':''}`);
 parts.push(`容量 形状 ${mb(geometryBytes)}＋法線 ${mb(normalBytes)}${auxiliaryBytes?'＋補助標高 '+mb(auxiliaryBytes):''}＝${mb(geometryBytes+normalBytes+auxiliaryBytes)}`);
 if(heightASTCInfo&&(!poly||$('normal').value==='gpu'))parts.push(`標高 最大誤差 ${heightASTCInfo.row.maxErrorMeters.toFixed(2)} m／RMSE ${heightASTCInfo.row.rmseMeters.toFixed(2)} m${heightASTCInfo.hardware?'／GPU 直接参照':'／PNG で代替表示'}`);
 parts.push(solarControls?.enabled?`太陽 ${solarControls.input.value} JST（各地で計算）`:`光 ${$('light').value}°・高さ ${$('lightAltitude').value}°`);
 parts.push(`陰影 ${$('strength').value}`,`視点 中心 ${target.map(v=>v.toFixed(1)).join(',')} m／距離 ${distance.toFixed(1)} m／方角 ${bearing.toFixed(1)}°／傾き ${tilt.toFixed(1)}°`);
 const line=parts.join(' | ');if($('settingsLine').value!==line){$('settingsLine').value=line;$('settingsLine').title=line;if(document.activeElement!==$('settingsLine'))$('settingsLine').scrollLeft=0;$('copyStatus').textContent='';}
 $('copySettings').disabled=false;
}
$('settingsLine').addEventListener('click',()=> $('settingsLine').select());
$('copySettings').addEventListener('click',async()=>{
 const line=$('settingsLine');line.focus();line.select();
 try{await navigator.clipboard.writeText(line.value);$('copyStatus').textContent='コピーしました';}
 catch{$('copyStatus').textContent='選択した設定を手動でコピーしてください';}
});
function scheduleWater(){const width=(meta.width-1)*meta.meters,height=(meta.height-1)*meta.meters,aspect=canvas.width/canvas.height,rx=distance*.7*aspect/Math.max(Math.cos(tilt*rad),.35),ry=distance*.7/Math.max(Math.cos(tilt*rad),.35),x=.5+target[0]/width,y=.5-target[1]/height;inlandVectors.schedule(solarBounds(),[x-rx/width,y-ry/height,x+rx/width,y+ry/height],Math.min(width,2*rx)/1000,()=>{});}
function solarBounds(){if(!sourceMeta)return [0,0,1,1];const n=2**sourceMeta.zoom;return [sourceMeta.west/n,sourceMeta.north/n,sourceMeta.width/256/n,sourceMeta.height/256/n];}
function solarLocation(){if(!sourceMeta)return {latitude:region==='tsukuba'?36.2254:36.3419,longitude:region==='tsukuba'?140.1069:137.6476};const b=solarBounds();return SolarPosition.mercator(b[0]+(.5+target[0]/((sourceMeta.width-1)*sourceMeta.meters))*b[2],b[1]+(.5-target[1]/((sourceMeta.height-1)*sourceMeta.meters))*b[3]);}
function reset(){target=[0,0,region==='tsukuba'?250:1500];distance=nativeTiles&&meta?meta.width*meta.meters*1.5:24000;bearing=0;tilt=nativeTiles?35:55;}
function draw(){
 requestAnimationFrame(draw);
 if(!loaded)return;
 const ratio=Math.min(devicePixelRatio,1.5),w=Math.round(innerWidth*ratio),h=Math.round(innerHeight*ratio);
 if(canvas.width!==w||canvas.height!==h){canvas.width=w;canvas.height=h;}
 const autoStep=2**Math.max(0,Math.min(5,Math.floor(Math.log2(distance*Math.tan(25*rad)*6/(innerHeight*meta.meters)))));
 const poly=polygonLevel();
 if(!poly)buildGrid(Math.min(patchSize,$('step').value==='auto'?autoStep:Number($('step').value)));
 gl.viewport(0,0,w,h);gl.clearColor(.969,.969,.949,1);gl.clear(gl.COLOR_BUFFER_BIT|gl.DEPTH_BUFFER_BIT);gl.enable(gl.DEPTH_TEST);
 scheduleWater();bytes+=inlandVectors.transferred-inlandReported;inlandReported=inlandVectors.transferred;inlandVectors.bind();gl.uniform4fv(uniforms.waterBounds,inlandVectors.bounds);gl.uniform1i(uniforms.detailedWater,inlandVectors.hasSea);gl.uniform1i(uniforms.showSea,$('showSea').checked);gl.uniform1i(uniforms.inlandMap,6);gl.uniform1i(uniforms.showRivers,$('showRivers').checked);gl.uniform1i(uniforms.showLakes,$('showLakes').checked);
 gl.uniformMatrix4fv(uniforms.viewProjection,false,cameraMatrix());
 gl.uniform1i(uniforms.polygonMode,poly?1:0);gl.uniform1i(uniforms.tangentNormal,normalInfo.tangent?1:0);
 gl.uniform1i(uniforms.heightFloat,heightFloat?1:0);gl.uniform1i(uniforms.heightASTC,heightASTC?1:0);gl.uniform1i(uniforms.normalRA,normalInfo?.ra?1:0);
 gl.uniform1i(uniforms.tileLines,Number($('tileLines').value));gl.uniform2f(uniforms.heightTileCounts,Math.ceil(meta.width/256),Math.ceil(meta.height/256));gl.uniform2f(uniforms.normalTileCounts,Math.ceil(normalInfo.size/256),Math.ceil(normalInfo.size/256));
 gl.uniform1f(uniforms.patternScale,Number($('patternScale').value));gl.uniform1f(uniforms.patternStrength,Number($('patternStrength').value));gl.uniform2f(uniforms.patternOrigin,((meta.west*256+(meta.width-1)/2)*meta.meters)%Number($('patternScale').value),(-(meta.north*256+(meta.height-1)/2)*meta.meters)%Number($('patternScale').value));
 gl.uniform1i(uniforms.normalMode,{face:0,png:1,gpu:2,pattern:3}[$('normal').value]);gl.uniform1f(uniforms.normalStep,Number($('normalStep').value));
 solarControls.uniforms(gl,uniforms,solarBounds());
 const az=Number($('light').value)*rad,alt=Number($('lightAltitude').value)*rad;gl.uniform3f(uniforms.lightDirection,Math.sin(az)*Math.cos(alt),Math.cos(az)*Math.cos(alt),Math.sin(alt));gl.uniform1f(uniforms.strength,Number($('strength').value));
 const instances=Math.ceil((meta.width-1)/patchSize)*Math.ceil((meta.height-1)/patchSize);
 const gridAttribute=gl.getAttribLocation(program,'gridPosition'),polygonAttribute=gl.getAttribLocation(program,'polygonPosition'),normalAttribute=gl.getAttribLocation(program,'polygonNormal');
 if(poly){
  gl.disableVertexAttribArray(gridAttribute);gl.vertexAttrib2f(gridAttribute,0,0);
  gl.bindBuffer(gl.ARRAY_BUFFER,polygonGeometry.vertices);gl.enableVertexAttribArray(polygonAttribute);gl.vertexAttribPointer(polygonAttribute,3,gl.FLOAT,false,0,0);
  gl.bindBuffer(gl.ARRAY_BUFFER,polygonGeometry.normals);gl.enableVertexAttribArray(normalAttribute);gl.vertexAttribPointer(normalAttribute,3,gl.FLOAT,false,0,0);
  gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,polygonGeometry.elements);gl.drawElements(gl.TRIANGLES,polygonGeometry.count,gl.UNSIGNED_INT,0);
 }else{
  gl.disableVertexAttribArray(normalAttribute);gl.vertexAttrib3f(normalAttribute,0,0,1);
  gl.disableVertexAttribArray(polygonAttribute);gl.vertexAttrib3f(polygonAttribute,0,0,0);
  gl.bindBuffer(gl.ARRAY_BUFFER,vertexBuffer);gl.enableVertexAttribArray(gridAttribute);gl.vertexAttribPointer(gridAttribute,2,gl.FLOAT,false,0,0);
  gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,indexBuffer);gl.drawElementsInstanced(gl.TRIANGLES,indexCount,gl.UNSIGNED_INT,0,instances);
 }
 if($('showSea').checked)seaVectors.draw({source:inlandVectors,mask:inlandVectors.hasSea,bounds:inlandVectors.bounds,matrix:cameraMatrix(),worldSize:[(meta.width-1)*meta.meters,(meta.height-1)*meta.meters]});

 frames++;const now=performance.now();if(now-lastFPS>1000){fps=frames*1000/(now-lastFPS);frames=0;lastFPS=now;}
 const triangles=poly?polygonGeometry.count/3:indexCount/3*instances,normalVariant=normalVariants.find(n=>String(n.size)===$('normalResolution').value);
 const geometryBytes=poly?polygonGeometry.bytes:selectedHeightBytes,normalBytes=$('normal').value==='png'?normalInfo.bytes:0,auxiliaryBytes=poly&&$('normal').value==='gpu'?selectedHeightBytes:0,selectedBytes=geometryBytes+normalBytes+auxiliaryBytes+($('showRivers').checked||$('showLakes').checked?inlandVectors.bytes:0)+($('showSea').checked?seaVectors.bytes:0);
 const label=poly?`ポリゴン L${poly}`:`ハイトマップ L${meta.level}・標高 ${meta.width}×${meta.height}${nativeTiles?`・標高 2×2 タイル・法線 ${Math.ceil(normalInfo.size/256)}×${Math.ceil(normalInfo.size/256)} タイル（元 L${meta.normalSourceLevel}）`:""}`;
 $('stats').textContent=`${label} ／ 比較用ファイル：形状 ${(geometryBytes/1e6).toFixed(3)}＋法線 ${(normalBytes/1e6).toFixed(3)}${auxiliaryBytes?'＋補助標高 '+(auxiliaryBytes/1e6).toFixed(3):''}＋海岸 z${inlandVectors.seaZoom}・河川 z${inlandVectors.riverZoom}・描画 ${inlandVectors.size}²${inlandVectors.loading?'（読込中）':inlandVectors.error||''} ${(($('showRivers').checked||$('showLakes').checked?inlandVectors.bytes:0)/1e6).toFixed(3)}＋海域 ${(($('showSea').checked?seaVectors.bytes:0)/1e6).toFixed(3)}＝${(selectedBytes/1e6).toFixed(3)} MB・累計取得 ${(bytes/1e6).toFixed(2)} MB ／ ${triangles.toLocaleString()} 三角形 ／ ${fps.toFixed(0)} FPS`;
 $('codecHint').textContent=normalInfo.hardware?(normalInfo.codec==='dynamic-jxl'?'JPEG XL をサーバーで復号した PNG／浮動小数標高で表示。表示用転送量と JPEG XL の保存容量は別です。':normalInfo.codec==='png'?'':`ASTC を GPU で参照。法線 GPU 本体 ${(normalInfo.gpuBytes/1e6).toFixed(3)} MB。角度誤差 P95 ${normalInfo.row.angularError.p95Degrees.toFixed(2)}°。`):'この GPU は ASTC 未対応のため、事前復号した PNG で画質を比較しています。ファイル容量は KTX2、取得累計は実際の転送量です。';
 if(heightASTCInfo&&(!poly||$('normal').value==='gpu'))$('codecHint').textContent+=` 標高 ASTC：最大誤差 ${heightASTCInfo.row.maxErrorMeters.toFixed(2)} m、RMSE ${heightASTCInfo.row.rmseMeters.toFixed(2)} m。${heightASTCInfo.hardware?'圧縮したまま GPU で参照。':'GPU 未対応のため事前復号 PNG から再現。'}`;
 if(poly&&polygonGeometry.quality?.collapsedOrFlippedFacesAfterQuantization)$('codecHint').textContent+=` 0.5 m 量子化で細い面 ${polygonGeometry.quality.collapsedOrFlippedFacesAfterQuantization} 個がつぶれるか反転しています。頂点可逆の方式で比較できます。`;
 if(normalInfo.source==='baked')$('codecHint').textContent+=` 詳細法線をポリゴン L${normalInfo.level} の接線空間へ焼き込んで復元しています。`;
 if(normalInfo.source==='mesh')$('codecHint').textContent+=` ポリゴン L${normalInfo.level} の形状から生成した法線です。詳細標高の細かな凹凸は含みません。`;
 if($('normal').value==='pattern')$('codecHint').textContent='共通の高さ模様 256×256（GPU 約 85 KB、端末内生成・配信データ 0 B）。実際の尾根や谷の細部は含みません。遠景では模様を弱めます。';
 updateSettingsLine(geometryBytes,normalBytes,auxiliaryBytes);$('settingsLine').value+='／海域 '+($('showSea').checked?'ON':'OFF');
 window.heightmapStatus={waterMaskSize:inlandVectors.size,waterCrop:[...inlandVectors.rasterCrop],waterLoading:inlandVectors.loading,waterError:inlandVectors.error,waterZoom:inlandVectors.zoom,seaZoom:inlandVectors.seaZoom,riverZoom:inlandVectors.riverZoom,detailedSea:inlandVectors.hasSea,riversEnabled:$('showRivers').checked,lakesEnabled:$('showLakes').checked,inlandVertices:[...inlandVectors.counts],inlandFeatures:[...inlandVectors.features],inlandBytes:inlandVectors.bytes,solarEnabled:solarControls.enabled,solarDateTime:solarControls.input.value,solarCenter:SolarPosition.position(solarControls.coefficients,solarLocation().latitude,solarLocation().longitude),seaEnabled:$('showSea').checked,seaRendering:inlandVectors.hasSea?'vector':'fallback',seaVertices:inlandVectors.hasSea?(seaVectors.detailedCount||0):seaVectors.count,seaBytes:seaVectors.bytes,nativeTiles,heightTiles:Math.ceil(meta.width/256)*Math.ceil(meta.height/256),normalTiles:Math.ceil(normalInfo.size/256)**2,geometry:poly?'polygon':'height',level:meta.level||14,normalSize:normalVariant.size,meshCodec:$('meshCodec').value,heightCodec:$('heightCodec').value,normalCodec:normalInfo.codec,normalHardware:normalInfo.hardware,heightASTCHardware:heightASTCInfo?.hardware??null,heightASTCError:heightASTCInfo?.row.maxErrorMeters??null,normalSource:normalInfo.source,normalLevel:normalInfo.level,decodeMs:poly?polygonGeometry.decodeMs:heightDecodeMs,triangles,step:currentStep,bytes,geometryBytes,normalBytes,auxiliaryBytes,selectedBytes,fps,normal:$('normal').value,glError:gl.getError()};
}
let drag=null;
canvas.addEventListener('contextmenu',e=>e.preventDefault());
canvas.addEventListener('pointerdown',e=>{drag={x:e.clientX,y:e.clientY,rotate:e.button===2||e.shiftKey};canvas.setPointerCapture(e.pointerId);});
canvas.addEventListener('pointerup',()=>drag=null);canvas.addEventListener('pointercancel',()=>drag=null);
canvas.addEventListener('pointermove',e=>{if(!drag)return;const dx=e.clientX-drag.x,dy=e.clientY-drag.y;drag.x=e.clientX;drag.y=e.clientY;
 if(drag.rotate){bearing-=dx*.3;tilt=Math.max(2,Math.min(88,tilt+dy*.25));}
 else{const s=distance*.0008,b=bearing*rad;target[0]-=(dx*Math.cos(b)+dy*Math.sin(b))*s;target[1]-=(dx*Math.sin(b)-dy*Math.cos(b))*s;}
});
canvas.addEventListener('wheel',e=>{e.preventDefault();distance=Math.max(300,Math.min(nativeTiles&&meta?Math.max(150000,meta.width*meta.meters*4):150000,distance*Math.exp(e.deltaY*.001)));},{passive:false});
$('reset').addEventListener('click',reset);
async function start(){
 const report=await(await fetch(regionPrefix+'/levels.json')).json();levelInfos=report.levels;normalVariants=report.normalVariants.map(n=>({...n,url:regionPrefix+n.url}));$('nativeTiles').checked=nativeTiles;sourceMeta=levelInfos.find(m=>m.level===14);
 $('normalResolution').replaceChildren(...normalVariants.map(m=>{const option=document.createElement('option');option.value=m.size;option.textContent=`${m.size}×${m.size}（${(m.bytes/1024).toFixed(1)} KB）`;return option;}));
 $('normalResolution').value=String(Math.min(1024,normalVariants[0].size));
 const params=new URL(location.href).searchParams;for(const id of ['normalResolution','normal','normalStep','patternScale','step','jxlHeightMode','jxlNormalMode','jxlSeams']){const value=params.get(id);if([...$(id).options].some(o=>o.value===value))$(id).value=value;}
 for(const id of ['light','lightAltitude','strength','patternStrength','jxlHeightDistance','jxlNormalDistance'])if(params.has(id)&&Number.isFinite(Number(params.get(id))))$(id).value=params.get(id);
 $('regionTitle').textContent=(region==='tsukuba'?'筑波山':'槍ヶ岳')+'・圧縮比較';document.title=$('regionTitle').textContent;for(const id of ['regionYari','regionTsukuba'])$(id).disabled=$(id).dataset.region===region;
 $('dataLevel').replaceChildren(...levelInfos.map(m=>{const option=document.createElement('option');option.value=m.level;option.textContent=`L${m.level}（${m.width}×${m.height}）`;return option;}));
 const query=new URL(location.href).searchParams.get('level');$('dataLevel').value=levelInfos.some(m=>String(m.level)===query)?query:'14';
 const heightQuery=new URL(location.href).searchParams.get('heightLevel');heightLevelPreference=levelInfos.some(m=>String(m.level)===heightQuery)?heightQuery:$('dataLevel').value;
 const geometryQuery=new URL(location.href).searchParams.get('geometry');if([...$('geometryMode').options].some(o=>o.value===geometryQuery))$('geometryMode').value=geometryQuery;
 for(const key of ['meshCodec','heightCodec','normalCodec','normalSource']){const query=new URL(location.href).searchParams.get(key);if([...$(key).options].some(o=>o.value===query)&&!(region==='tsukuba'&&query!=='png'&&key.endsWith('Codec')))$(key).value=query;}
 $('heightAvailability').textContent=nativeTiles?'実タイル用：PNG／JPEG XL（選択時に圧縮）。他形式は未生成。':region==='tsukuba'?'筑波山用：PNG／JPEG XL（選択時に圧縮）。他形式は未生成。':'';
 if(region==='tsukuba'||nativeTiles)for(const id of ['heightCodec','normalCodec'])for(const option of $(id).options)if(!['png','dynamic-jxl'].includes(option.value))option.disabled=true;
 makeProgram();uniforms=Object.fromEntries(['waterBounds','detailedWater','showSea','inlandMap','showRivers','showLakes','viewProjection','dimensions','heightOffset','meters','columns','patchSize','heightMap','normalMap','normalMode','normalStep','lightDirection','strength','polygonMode','normalWorldSize','heightFloat','heightASTC','heightHasMask','heightTileMap','heightTileParams','heightValidMap','heightTileCore','heightTilePadding','heightTileColumns','normalRA','tangentNormal','solarEnabled','solarAngles','solarBounds','patternMap','patternScale','patternStrength','patternOrigin','tileLines','heightTileCounts','normalTileCounts'].map(k=>[k,gl.getUniformLocation(program,k)]));
 initHeightASTC();
 // ポリゴン＋法線 PNG では標高 PNG は不要。未使用の sampler に完全な仮テクスチャだけ割り当てる。
 heightTexture=gl.createTexture();gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,heightTexture);gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA,1,1,0,gl.RGBA,gl.UNSIGNED_BYTE,new Uint8Array([0,0,255,255]));gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);
 const requestedJXL=$('heightCodec').value==='dynamic-jxl'||$('normalCodec').value==='dynamic-jxl';if(requestedJXL){$('heightCodec').value='png';$('normalCodec').value='png';}
 solarControls=new SolarControls({manualIds:['light','lightAltitude'],location:solarLocation,onChange:()=>{}});
 initPattern();seaVectors=new SeaVectors(gl);inlandVectors=new InlandVectors(gl,common);await loadLevel();if(nativeTiles)reset();if(requestedJXL)await encodeDynamicJXL();
}
async function loadLevel(){
 const started=performance.now();loaded=false;$('recommended').disabled=true;$('dataLevel').disabled=true;$('geometryMode').disabled=true;$('stats').textContent='形状データを読み込み中…';
 const poly=polygonLevel();$('dataLevel').value=poly?String(poly):heightLevelPreference;
 meta=levelInfos.find(m=>String(m.level)===$('dataLevel').value);
 if(nativeTiles){sourceMeta=meta;normalVariants=meta.normalVariants.map(n=>({...n,url:regionPrefix+n.url}));const selected=$('normalResolution').value;$('normalResolution').replaceChildren(...normalVariants.map(n=>{const o=document.createElement('option');o.value=n.size;o.textContent=n.size+'×'+n.size;return o;}));$('normalResolution').value=normalVariants.some(n=>String(n.size)===selected)?selected:String(normalVariants[0].size);}
 dataPrefix=regionPrefix+'/levels/'+meta.level;
 const seaTransferred=seaVectors.transferred;await seaVectors.load((nativeTiles?'native':'region')+'-'+region+'-L'+meta.level);bytes+=seaVectors.transferred-seaTransferred;
 await inlandVectors.loadWindow(WaterTileClient.request(solarBounds(),[0,0,1,1],meta.width*meta.meters/1000));
 if(meta.width>gl.getParameter(gl.MAX_TEXTURE_SIZE)||meta.height>gl.getParameter(gl.MAX_TEXTURE_SIZE))throw Error('この端末の最大テクスチャサイズを超えています');
 if(poly)polygonGeometry=await loadPolygon(poly);
 if(!poly||$('normal').value==='gpu')await ensureHeight();
 $('stats').textContent='形状を取得済み。法線マップを読み込み中…';await selectNormal();
 patchSize=Math.min(256,meta.width-1,meta.height-1);currentStep=0;
 gl.uniform1i(uniforms.heightMap,0);gl.uniform1i(uniforms.normalMap,1);gl.uniform2f(uniforms.dimensions,meta.width,meta.height);gl.uniform1f(uniforms.heightOffset,meta.heightOffset);gl.uniform1f(uniforms.meters,meta.meters);gl.uniform1i(uniforms.columns,Math.ceil((meta.width-1)/patchSize));gl.uniform1f(uniforms.patchSize,patchSize);
 gl.uniform2f(uniforms.normalWorldSize,(sourceMeta.width-1)*sourceMeta.meters,(sourceMeta.height-1)*sourceMeta.meters);
 window.heightmapLoadMilliseconds=performance.now()-started;window.heightmapMetadata=meta;loaded=true;$('dataLevel').disabled=!!poly;$('geometryMode').disabled=false;$('step').disabled=!!poly;$('meshCodec').disabled=!poly;$('normalSource').disabled=!poly;$('recommended').disabled=false;$('jxlApply').disabled=false;
 const url=new URL(location.href);url.searchParams.set('level',meta.level);url.searchParams.set('heightLevel',heightLevelPreference);url.searchParams.set('geometry',$('geometryMode').value);history.replaceState(null,'',url);
}
$('dataLevel').addEventListener('change',async()=>{
 heightLevelPreference=$('dataLevel').value;
 const regenerate=$('heightCodec').value==='dynamic-jxl'||$('normalCodec').value==='dynamic-jxl';
 if(regenerate){$('heightCodec').value='png';$('normalCodec').value='png';}
 try{await loadLevel();if(nativeTiles)reset();if(regenerate)await encodeDynamicJXL();}catch(e){$('stats').textContent=e.message;$('dataLevel').disabled=false;console.error(e);}
});
$('geometryMode').addEventListener('change',()=>loadLevel().catch(e=>{$('stats').textContent=e.message;$('geometryMode').disabled=false;console.error(e);}));
$('meshCodec').addEventListener('change',()=>loadLevel().catch(console.error));
$('heightCodec').addEventListener('change',async()=>{
 if($('heightCodec').value==='dynamic-jxl'&&(!dynamicJXL||dynamicJXL.level!==meta.level)){await encodeDynamicJXL();return;}
 try{await loadLevel();}catch(e){$('stats').textContent=e.message;$('geometryMode').disabled=false;console.error(e);}
});
$('recommended').addEventListener('click',async()=>{
 $('recommended').disabled=true;
 // 視点と光源は保ち、利用可能なハイトマップの設定へ戻す。
 heightLevelPreference='11';
 for(const [id,value] of Object.entries({geometryMode:'height',normalSource:'shared',meshCodec:'meshopt-f32-zstd',heightCodec:'png',step:'auto',normal:'png',normalResolution:'1024',normalCodec:region==='tsukuba'||nativeTiles?'png':'astc4-zstd',normalStep:'2'}))$(id).value=value;
 try{await loadLevel();}catch(e){$('stats').textContent=e.message;console.error(e);}finally{$('recommended').disabled=false;}
});
requestAnimationFrame(draw);
start().catch(e=>{$('stats').textContent=e.message;console.error(e);});

async function displayDynamicJXL(original){
 if(!dynamicJXL)return;
 $('geometryMode').value='height';heightLevelPreference=String(dynamicJXL.level);$('normalResolution').value=String(dynamicJXL.normalSize);$('normalSource').value='shared';
 for(const id of ['heightCodec','normalCodec']){const option=[...$(id).options].find(o=>o.value==='dynamic-jxl');option.disabled=false;$(id).value=original?'png':'dynamic-jxl';}
 await loadLevel();
}
async function encodeDynamicJXL(){
 if($('jxlApply').disabled)return;
 const button=$('jxlApply');button.disabled=true;$('jxlOriginal').disabled=true;
 // 未適用の解像度選択でも、圧縮元はリクエスト開始時の値に固定する。
 const options={region,tileGrid:nativeTiles?2:0,seams:$('jxlSeams').value,heightMode:$('jxlHeightMode').value,normalMode:$('jxlNormalMode').value,level:Number($('dataLevel').value),normalSize:Number($('normalResolution').value),heightDistance:Number($('jxlHeightDistance').value),normalDistance:Number($('jxlNormalDistance').value)};
 $('jxlResult').textContent='サーバーで JPEG XL をエンコード・デコード中…（L14 は時間がかかります）';
 try{
  const response=await fetch('/dynamic-jxl/encode',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(options)}),result=await response.json();if(!response.ok)throw Error(result.error);
  dynamicJXL=result;$('jxlOriginal').checked=false;await displayDynamicJXL(false);
  $('jxlResult').textContent=`標高 ${result.heightMode} ${(result.heightBytes/1e6).toFixed(3)} MB（画像＋管理値・欠損マスク）・法線 ${result.normalMode} ${(result.normalBytes/1e6).toFixed(3)} MB ／ 標高 ${result.heightTileColumns}×${result.heightTileRows} タイル・法線 ${result.normalTileColumns}×${result.normalTileRows} タイル ／ 境界対策 ${result.seams}・境界誤差段差 ${result.heightBoundaryErrorJump.toFixed(2)} m ／ 標高最大誤差 ${result.maxHeightError.toFixed(2)} m・RMSE ${result.rmseHeight.toFixed(2)} m ／ 法線 P95 ${result.normalP95.toFixed(2)}°・最大 ${result.normalMax.toFixed(2)}° ／ 符号化 ${(result.encodeMs/1000).toFixed(2)} 秒・復号 ${(result.decodeMs/1000).toFixed(2)} 秒（CLI 起動・画像入出力込み）`;
 }catch(e){$('jxlResult').textContent=e.message;console.error(e);
  $('heightCodec').value='png';$('normalCodec').value='png';await loadLevel();
 }finally{button.disabled=false;$('jxlOriginal').disabled=!dynamicJXL;}
}
$('jxlApply').addEventListener('click',encodeDynamicJXL);
$('jxlOriginal').addEventListener('change',()=>displayDynamicJXL($('jxlOriginal').checked).catch(e=>{$('jxlResult').textContent=e.message;}));

for(const id of ['regionYari','regionTsukuba'])$(id).addEventListener('click',()=>{
 const url=new URL(location.href);url.searchParams.set('region',$(id).dataset.region);url.searchParams.set('level',heightLevelPreference);url.searchParams.set('heightLevel',heightLevelPreference);url.searchParams.set('geometry','height');
 for(const key of ['normalResolution','normal','normalStep','patternScale','patternStrength','step','light','lightAltitude','strength','jxlHeightDistance','jxlNormalDistance','jxlHeightMode','jxlNormalMode','jxlSeams'])url.searchParams.set(key,$(key).value);
 url.searchParams.set('sunMode',solarControls.mode.value);url.searchParams.set('sunDateTime',solarControls.input.value);url.searchParams.set('heightCodec','png');url.searchParams.set('normalCodec','png');url.searchParams.set('normalSource','shared');location.href=url;
});

$('nativeTiles').addEventListener('change',()=>{const url=new URL(location.href);url.searchParams.set('tiles',$('nativeTiles').checked?'2':'0');url.searchParams.set('level',$('dataLevel').value);url.searchParams.set('heightLevel',$('dataLevel').value);url.searchParams.set('geometry','height');url.searchParams.set('heightCodec','png');url.searchParams.set('normalCodec','png');location.href=url;});

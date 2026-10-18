/* 全国で同じ粗い地形を使い、法線と縮小済み陰影だけを切り替える。 */
'use strict';
const $=id=>document.getElementById(id),canvas=$('map'),gl=canvas.getContext('webgl2',{antialias:true});
let manifest,program,uniforms,mode='normal',resolution='equal',center=[0,0],scale=1,bearing=0,triangles=0,transferred=0,ready=false,frame=0;
let bakedSun={azimuth:315,altitude:45},shadeBytes=0,sunBusy=false,sunGeneration=0,sunError='';
let illuminationLevels=[],bakedLevels=[],bakedRoot='/national-shading-data/',normalLevel=4,shadeLevel=4,illuminationRequest=0;
let geometryLevels=[],currentGeometry=null,heightTexture=null,meshVertexBuffer=null,meshIndexBuffer=null;
const geometryCache=new Map();
let seaVectors,solarControls,inlandVectors,inlandReported=0;
const textures=new Map(),base='/national-shading-data/';
function shader(type,source){const s=gl.createShader(type);gl.shaderSource(s,source);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;}
function createProgram(){
 program=gl.createProgram();gl.attachShader(program,shader(gl.VERTEX_SHADER,`#version 300 es
 precision highp float;in vec2 uv;uniform highp sampler2D heightMap;uniform vec2 worldSize;uniform vec2 center;uniform float scale;uniform float aspect;uniform float bearing;uniform float tilt;out vec2 mapUV;
 void main(){mapUV=uv;float h=texture(heightMap,uv).r;vec2 p=(uv-0.5)*worldSize*vec2(1.0,-1.0)-center;
 p=mat2(cos(bearing),-sin(bearing),sin(bearing),cos(bearing))*p;
 vec3 q=vec3(p.x,p.y*cos(tilt)+h*sin(tilt),-p.y*sin(tilt)+h*cos(tilt));
 gl_Position=vec4(q.x/(scale*aspect),q.y/scale,-q.z/(2.0*max(worldSize.x,worldSize.y)),1.0);}`));
 gl.attachShader(program,shader(gl.FRAGMENT_SHADER,`#version 300 es
 precision highp float;in vec2 mapUV;uniform sampler2D inlandMap;uniform vec4 waterBounds;uniform bool detailedWater;uniform bool showRivers;uniform bool showLakes;uniform sampler2D normalMap;uniform sampler2D shadeMap;uniform sampler2D coverageMap;uniform bool baked;uniform bool showSea;uniform float strength;uniform float contrast;uniform vec3 sunDirection;uniform float flatDiffuse;out vec4 color;
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

 void main(){float coverage=texture(coverageMap,mapUV).r;vec2 waterUV=(mapUV-waterBounds.xy)/waterBounds.zw;vec2 riverDX=dFdx(waterUV),riverDY=dFdy(waterUV);bool inside=all(greaterThanEqual(waterUV,vec2(0.0)))&&all(lessThanEqual(waterUV,vec2(1.0)));vec3 water=inside?texture(inlandMap,waterUV).rgb:vec3(0.0);float sea=detailedWater&&inside?water.b:1.0-coverage;if(showSea&&sea>0.5)discard;
 vec3 sun=solarEnabled?solarDirection(mapUV):sunDirection;float flatLevel=solarEnabled?max(sun.z,0.0):flatDiffuse;
 float diffuse;if(baked){diffuse=texture(shadeMap,mapUV).r;}else{vec3 n=normalize(texture(normalMap,mapUV).rgb*2.0-1.0);diffuse=solarEnabled&&sun.z<=0.0?0.0:max(dot(n,sun),0.0);}
 // 海域 OFF 時の欠損部分は水平面の照明値で埋める。
 diffuse=mix(flatLevel,diffuse,clamp(coverage*4.0,0.0,1.0));
 diffuse=clamp(flatLevel+(diffuse-flatLevel)*contrast,0.0,1.0);
 float illumination=mix(1.0,mix(0.04,0.35,solarEnabled?solarDay(sun):1.0)+0.65*diffuse,strength);vec3 land=vec3(0.76,0.79,0.73)*illumination;float lake=showLakes?water.g:0.0,river=showRivers?visibleRiver(waterUV,riverDX,riverDY,water.r):0.0;land=mix(land,vec3(.18,.62,.86),river*(1.0-lake));land=mix(land,vec3(.20,.51,.72),lake);color=vec4(land,1.0);}`));
 gl.linkProgram(program);if(!gl.getProgramParameter(program,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(program));gl.useProgram(program);
 uniforms=Object.fromEntries(['waterBounds','detailedWater','inlandMap','showRivers','showLakes','heightMap','normalMap','shadeMap','coverageMap','worldSize','center','scale','aspect','bearing','tilt','baked','showSea','strength','contrast','solarEnabled','solarAngles','solarBounds','sunDirection','flatDiffuse'].map(k=>[k,gl.getUniformLocation(program,k)]));
 for(const [i,name]of ['heightMap','normalMap','shadeMap','coverageMap'].entries())gl.uniform1i(uniforms[name],i);
 gl.enable(gl.DEPTH_TEST);gl.enable(gl.BLEND);gl.blendFunc(gl.SRC_ALPHA,gl.ONE_MINUS_SRC_ALPHA);
}
function makeGeometry(width,height){
 const w=Math.ceil((width-1)/4),h=Math.ceil((height-1)/4),positions=new Float32Array((w+1)*(h+1)*2),indices=new Uint32Array(w*h*6);
 let p=0;for(let y=0;y<=h;y++)for(let x=0;x<=w;x++){positions[p++]=x/w;positions[p++]=y/h;}
 p=0;for(let y=0;y<h;y++)for(let x=0;x<w;x++){const a=y*(w+1)+x,b=a+1,c=a+w+1,d=c+1;indices.set([a,b,c,b,d,c],p);p+=6;}
 if(meshVertexBuffer)gl.deleteBuffer(meshVertexBuffer);if(meshIndexBuffer)gl.deleteBuffer(meshIndexBuffer);
 meshVertexBuffer=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,meshVertexBuffer);gl.bufferData(gl.ARRAY_BUFFER,positions,gl.STATIC_DRAW);const location=gl.getAttribLocation(program,'uv');gl.enableVertexAttribArray(location);gl.vertexAttribPointer(location,2,gl.FLOAT,false,0,0);
 meshIndexBuffer=gl.createBuffer();gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,meshIndexBuffer);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,indices,gl.STATIC_DRAW);triangles=indices.length/3;
}
async function loadGeometry(level){
 const info=geometryLevels.find(g=>g.level===level);if(!info)throw Error('この地形レベルは未生成です');
 if(info.width>gl.getParameter(gl.MAX_TEXTURE_SIZE)||info.height>gl.getParameter(gl.MAX_TEXTURE_SIZE))throw Error('端末の最大テクスチャサイズを超えています');
 if(!geometryCache.has(level)){const data=await bytes(info.url),values=new Float32Array(data);if(values.length!==info.width*info.height)throw Error('標高の寸法が不正です');geometryCache.set(level,values);}
 const texture=gl.createTexture();gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,texture);gl.texImage2D(gl.TEXTURE_2D,0,gl.R32F,info.width,info.height,0,gl.RED,gl.FLOAT,geometryCache.get(level));gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);
 if(heightTexture)gl.deleteTexture(heightTexture);heightTexture=texture;makeGeometry(info.width,info.height);currentGeometry=info;
}
$('geometryLevel').addEventListener('change',async()=>{
 const select=$('geometryLevel');select.disabled=true;
 try{await loadGeometry(Number(select.value));await loadIllumination();draw();}catch(e){select.value=String(currentGeometry.level);$('status').textContent=e.message;console.error(e);}finally{select.disabled=false;}
});

async function bytes(name){const r=await fetch(name.startsWith('/')?name:base+name);if(!r.ok)throw Error('取得失敗: '+name);const data=await r.arrayBuffer();transferred+=Number(r.headers.get('Content-Length'))||data.byteLength;return data;}
async function image(name,unit,gray=false){
 const data=await bytes(name),bitmap=await createImageBitmap(new Blob([data],{type:'image/png'}),{colorSpaceConversion:'none',premultiplyAlpha:'none'}),texture=gl.createTexture();gl.activeTexture(gl.TEXTURE0+unit);gl.bindTexture(gl.TEXTURE_2D,texture);gl.pixelStorei(gl.UNPACK_ALIGNMENT,1);gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL,gl.NONE);
 if(gray){const surface=new OffscreenCanvas(bitmap.width,bitmap.height),context=surface.getContext('2d');context.drawImage(bitmap,0,0);const rgba=context.getImageData(0,0,bitmap.width,bitmap.height).data,red=new Uint8Array(bitmap.width*bitmap.height);for(let i=0;i<red.length;i++)red[i]=rgba[i*4];gl.texImage2D(gl.TEXTURE_2D,0,gl.R8,bitmap.width,bitmap.height,0,gl.RED,gl.UNSIGNED_BYTE,red);}
 else gl.texImage2D(gl.TEXTURE_2D,0,gl.RGB8,gl.RGB,gl.UNSIGNED_BYTE,bitmap);
 gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.LINEAR_MIPMAP_LINEAR);gl.generateMipmap(gl.TEXTURE_2D);bitmap.close();textures.set(name,texture);
}
function scheduleWater(){
 const t=Number($('tilt').value)*Math.PI/180,aspect=canvas.width/canvas.height,c=Math.abs(Math.cos(bearing)),s=Math.abs(Math.sin(bearing)),ry=scale/Math.max(Math.cos(t),.2)+5000;
 const rx=(scale*aspect*c+ry*s)*1.15,dy=(scale*aspect*s+ry*c)*1.15,x=.5+center[0]/manifest.worldWidth,y=.5-center[1]/manifest.worldHeight;
 inlandVectors.schedule(solarBounds(),[x-rx/manifest.worldWidth,y-dy/manifest.worldHeight,x+rx/manifest.worldWidth,y+dy/manifest.worldHeight],2*rx/1000,draw);
}
function focusLocation(latitude,longitude,span){const b=solarBounds(),x=(longitude+180)/360,y=(1-Math.asinh(Math.tan(latitude*Math.PI/180))/Math.PI)/2;scale=span*1000/2;bearing=0;$('tilt').value='0';const top=document.querySelector('header').getBoundingClientRect().bottom+15;center=[((x-b[0])/b[2]-.5)*manifest.worldWidth,(.5-(y-b[1])/b[3])*manifest.worldHeight+scale*(top-65)/innerHeight];}
function reset(){center=[0,0];bearing=0;$('tilt').value='0';const top=document.querySelector('header').getBoundingClientRect().bottom+15,bottom=65,available=Math.max(200,innerHeight-top-bottom);const bounds=manifest.landBounds||[0,0,1,1];scale=Math.max(manifest.worldHeight*(bounds[3]-bounds[1])*innerHeight/available,manifest.worldWidth*(bounds[2]-bounds[0])/(innerWidth/innerHeight))*.54;center=[((bounds[0]+bounds[2])/2-.5)*manifest.worldWidth,(.5-(bounds[1]+bounds[3])/2)*manifest.worldHeight+scale*(top-bottom)/innerHeight];}
function draw(){
 if(!ready)return;const ratio=Math.min(devicePixelRatio,1.5);canvas.width=Math.round(innerWidth*ratio);canvas.height=Math.round(innerHeight*ratio);gl.viewport(0,0,canvas.width,canvas.height);const flatSun=mode==='shade'?bakedSun:selectedSun(),flat=Math.sin(flatSun.altitude*Math.PI/180),flatLight=1-Number($('strength').value)+Number($('strength').value)*(.35+.65*flat);if($('showSea').checked)gl.clearColor(.12,.35,.55,1);else gl.clearColor(.76*flatLight,.79*flatLight,.73*flatLight,1);gl.clear(gl.COLOR_BUFFER_BIT|gl.DEPTH_BUFFER_BIT);
 gl.activeTexture(gl.TEXTURE1);gl.bindTexture(gl.TEXTURE_2D,textures.get(base+`normal-L${normalLevel}.png`));gl.activeTexture(gl.TEXTURE2);gl.bindTexture(gl.TEXTURE_2D,textures.get(bakedRoot+`shade-L${shadeLevel}.png`));
 scheduleWater();transferred+=inlandVectors.transferred-inlandReported;inlandReported=inlandVectors.transferred;inlandVectors.bind();gl.uniform4fv(uniforms.waterBounds,inlandVectors.bounds);gl.uniform1i(uniforms.detailedWater,inlandVectors.hasSea);gl.uniform1i(uniforms.inlandMap,6);gl.uniform1i(uniforms.showRivers,$('showRivers').checked);gl.uniform1i(uniforms.showLakes,$('showLakes').checked);
 solarControls.uniforms(gl,uniforms,solarBounds());
 if(mode==='shade'){gl.uniform1i(uniforms.solarEnabled,!!bakedSun.datetime);if(bakedSun.datetime){const coeff=SolarPosition.ephemeris(SolarPosition.parseJST(bakedSun.datetime));gl.uniform2f(uniforms.solarAngles,coeff.declination,coeff.hourAngle);}}
 gl.uniform2f(uniforms.worldSize,manifest.worldWidth,manifest.worldHeight);gl.uniform2f(uniforms.center,...center);gl.uniform1f(uniforms.scale,scale);gl.uniform1f(uniforms.aspect,canvas.width/canvas.height);gl.uniform1f(uniforms.bearing,bearing);gl.uniform1f(uniforms.tilt,Number($('tilt').value)*Math.PI/180);gl.uniform1f(uniforms.strength,Number($('strength').value));gl.uniform1i(uniforms.showSea,$('showSea').checked);gl.uniform1i(uniforms.baked,mode==='shade');gl.uniform1f(uniforms.contrast,Number($('contrast').value));const sun=selectedSun(),az=sun.azimuth*Math.PI/180,alt=sun.altitude*Math.PI/180;gl.uniform3f(uniforms.sunDirection,Math.sin(az)*Math.cos(alt),Math.cos(az)*Math.cos(alt),Math.sin(alt));gl.uniform1f(uniforms.flatDiffuse,Math.sin((mode==='shade'?bakedSun.altitude:sun.altitude)*Math.PI/180));gl.drawElements(gl.TRIANGLES,triangles*3,gl.UNSIGNED_INT,0);
 if($('showSea').checked)seaVectors.draw({source:inlandVectors,mask:inlandVectors.hasSea,bounds:inlandVectors.bounds,national:true,worldSize:[manifest.worldWidth,manifest.worldHeight],center,scale,aspect:canvas.width/canvas.height,bearing,tilt:Number($('tilt').value)*Math.PI/180});

 const info=(mode==='shade'?bakedLevels:illuminationLevels).find(l=>l.level===(mode==='shade'?shadeLevel:normalLevel)),pixels=info.width*info.height,components=mode==='shade'?1:3,selectedBytes=mode==='shade'?info.shadeBytes:info.normalBytes;
 $('normalMode').setAttribute('aria-pressed',String(mode==='normal'));$('shadeMode').setAttribute('aria-pressed',String(mode==='shade'));
 $('metrics').textContent=`${mode==='shade'?'縮小した陰影画像':'法線マップ'} PNG ${(selectedBytes/1e6).toFixed(3)} MB ／ GPU 約 ${(pixels*components*4/3/1e6).toFixed(1)} MB（ミップ込み・この画像のみ） ／ 表示 L${currentGeometry.level}・法線 L${normalLevel}／陰影 L${shadeLevel}・地形 L${currentGeometry.level}（${currentGeometry.width}×${currentGeometry.height}）・${triangles.toLocaleString()} 三角形 ／ 海岸 z${inlandVectors.seaZoom}・河川 z${inlandVectors.riverZoom}・描画 ${inlandVectors.size}² ${inlandVectors.loading?'（読込中）':inlandVectors.error||''} ／ 河川・湖沼 ${(($('showRivers').checked||$('showLakes').checked?inlandVectors.bytes:0)/1e6).toFixed(3)} MB ／ 海域ベクター ${($('showSea').checked?seaVectors.bytes/1e6:0).toFixed(3)} MB ／ 比較用データ取得済み ${(transferred/1e6).toFixed(2)} MB`;
 window.nationalShadingStatus={waterMaskSize:inlandVectors.size,waterCrop:[...inlandVectors.rasterCrop],waterLoading:inlandVectors.loading,waterError:inlandVectors.error,waterZoom:inlandVectors.zoom,seaZoom:inlandVectors.seaZoom,riverZoom:inlandVectors.riverZoom,waterBounds:[...inlandVectors.bounds],detailedSea:inlandVectors.hasSea,riversEnabled:$('showRivers').checked,lakesEnabled:$('showLakes').checked,inlandVertices:[...inlandVectors.counts],inlandFeatures:[...inlandVectors.features],inlandBytes:inlandVectors.bytes,solarEnabled:solarControls.enabled,solarDateTime:solarControls.input.value,seaEnabled:$('showSea').checked,seaRendering:inlandVectors.hasSea?'vector':'fallback',seaVertices:inlandVectors.hasSea?(seaVectors.detailedCount||0):seaVectors.count,seaBytes:seaVectors.bytes,mode,resolution,sourceLevel:manifest.sourceLevel,geometryLevel:currentGeometry.level,geometryWidth:currentGeometry.width,geometryBytes:currentGeometry.bytes,geometryTransferBytes:currentGeometry.transferBytes,center:[...center],scale,bearing,tilt:Number($('tilt').value),strength:Number($('strength').value),contrast:Number($('contrast').value),triangles,bytes:selectedBytes,normalLevel,shadeLevel,textureWidth:info.width,sun:selectedSun(),bakedSun:{...bakedSun},sunBusy,frame:++frame,glError:gl.getError()};updateSunStatus();
}
for(const [id,value]of [['normalMode','normal'],['shadeMode','shade']])$(id).addEventListener('click',()=>{mode=value;draw();});
$('normalResolution').addEventListener('change',async()=>{resolution=$('normalResolution').value;await loadIllumination();draw();});for(const id of ['strength','tilt','contrast'])$(id).addEventListener('input',draw);for(const id of ['showSea','showRivers','showLakes','waterQuality','seaLevel','riverLevel'])$(id).addEventListener('change',draw);$('reset').addEventListener('click',()=>{if(ready){reset();draw();}});addEventListener('resize',draw);
let drag;canvas.addEventListener('contextmenu',e=>e.preventDefault());canvas.addEventListener('pointerdown',e=>{drag={x:e.clientX,y:e.clientY,rotate:e.button===2};canvas.setPointerCapture(e.pointerId);});canvas.addEventListener('pointerup',()=>drag=null);canvas.addEventListener('pointercancel',()=>drag=null);canvas.addEventListener('pointermove',e=>{if(!drag||!ready)return;const dx=e.clientX-drag.x,dy=e.clientY-drag.y;drag.x=e.clientX;drag.y=e.clientY;if(drag.rotate)bearing+=dx*.005;else{const units=scale*2/innerHeight;center[0]-=(dx*Math.cos(bearing)+dy*Math.sin(bearing))*units;center[1]+=(-dx*Math.sin(bearing)+dy*Math.cos(bearing))*units;}draw();});canvas.addEventListener('wheel',e=>{e.preventDefault();scale=Math.max(10000,Math.min(12000000,scale*Math.exp(e.deltaY*.001)));draw();},{passive:false});
async function start(){
 if(!gl)throw Error('WebGL 2 が必要です');const response=await fetch(base+'manifest.json');if(!response.ok){const p=await(await fetch(base+'progress.json')).json();$('status').textContent=`全国比較データ生成中：${p.completed.toLocaleString()} / ${p.total.toLocaleString()} タイル。3 秒後に確認します。`;setTimeout(start,3000);return;}
 manifest=await response.json();bakedSun={azimuth:manifest.lightAzimuth%360,altitude:manifest.lightAltitude};shadeBytes=manifest.files['shade.png'];createProgram();$('status').textContent='全国の地形・法線・陰影画像を読み込み中…';
 geometryLevels=(await(await fetch('/national-geometry/levels.json')).json()).levels;await loadGeometry(Number($('geometryLevel').value));
 // テクスチャの割当先が競合しないよう、GPU への登録は順番に行う。
 illuminationLevels=(await(await fetch(base+'illumination-levels.json')).json()).levels;bakedLevels=illuminationLevels;await loadIllumination();await image('coverage.png',3,true);
 $('status').textContent=`元データ：L${manifest.sourceLevel}・全国 ${manifest.sourceTiles.toLocaleString()} タイル。表示レベルに応じて地形・法線・縮小陰影を切り替えます。法線は太陽位置を即時反映。縮小陰影は再生成後に反映。JPEG XL の圧縮差を含めず PNG で比較。`;

 solarControls=new SolarControls({manualIds:['sunAzimuth','sunAltitude'],location:solarLocation,onChange:()=>{if(ready)draw();}});
 seaVectors=new SeaVectors(gl);await seaVectors.load('national');transferred+=seaVectors.transferred;inlandVectors=new InlandVectors(gl);await inlandVectors.loadWindow(WaterTileClient.request(solarBounds(),[0,0,1,1],manifest.worldWidth/1000));
 ready=true;$('geometryLevel').disabled=false;$('normalMode').disabled=false;$('shadeMode').disabled=false;$('sunBake').disabled=false;reset();draw();
 // 容量表示で操作欄の高さが確定してから、全国が隠れない画角に合わせる。
 reset();const q=new URL(location.href).searchParams;if(q.has('lat')&&q.has('lon')){const lat=Number(q.get('lat')),lon=Number(q.get('lon')),span=Number(q.get('span')||180);if(lat>=20&&lat<=50&&lon>=120&&lon<=155&&span>=10&&span<=5000)focusLocation(lat,lon,span);}draw();
}
start().catch(e=>{$('status').textContent=e.message;$('status').setAttribute('role','alert');console.error(e);});

function solarBounds(){return manifest?[manifest.west/64,manifest.north/64,manifest.width/256/64,manifest.height/256/64]:[0,0,1,1];}
function solarLocation(){if(!manifest)return {latitude:36,longitude:138};const b=solarBounds();return SolarPosition.mercator(b[0]+(.5+center[0]/manifest.worldWidth)*b[2],b[1]+(.5-center[1]/manifest.worldHeight)*b[3]);}
function selectedSun(){if(solarControls?.enabled){const p=solarLocation();return {...SolarPosition.position(solarControls.coefficients,p.latitude,p.longitude),datetime:solarControls.input.value};}return {azimuth:Number($('sunAzimuth').value)%360,altitude:Number($('sunAltitude').value)};}
function sameSun(a,b){return a.datetime||b.datetime?a.datetime===b.datetime:a.azimuth===b.azimuth&&a.altitude===b.altitude;}
function updateSunStatus(){
 const current=selectedSun();$('sunAzimuthValue').value=current.azimuth.toFixed(1)+'°';$('sunAltitudeValue').value=current.altitude.toFixed(1)+'°';
 if(!sunBusy)$('sunStatus').textContent=(sunError?'エラー：'+sunError+' ／ ':'')+`法線の太陽：方角 ${current.azimuth}°・高さ ${current.altitude}° ／ 縮小陰影の太陽：方角 ${bakedSun.azimuth}°・高さ ${bakedSun.altitude}°。${sameSun(current,bakedSun)?'同じ光源で比較できます。':'縮小陰影は旧光源の画像です。「再生成」でそろえられます。'}`;
 if(solarControls?.enabled&&!sunBusy)$('sunStatus').textContent=`日時指定 ${solarControls.input.value} JST。法線は地点ごとに即時計算。縮小陰影：${bakedSun.datetime?bakedSun.datetime+' JST':'手動光源の画像'}。${sameSun(current,bakedSun)?'同じ日時で比較できます。':'再生成すると指定日時になります。'}`;
 $('sunBake').disabled=!ready||sunBusy||!!solarControls?.error;
}
for(const id of ['sunAzimuth','sunAltitude'])$(id).addEventListener('input',()=>{updateSunStatus();draw();});
async function applySunImage(result){
 const reference=solarLocation();const dated=result.datetime?SolarPosition.position(SolarPosition.ephemeris(SolarPosition.parseJST(result.datetime)),reference.latitude,reference.longitude):result;
 bakedSun={azimuth:dated.azimuth,altitude:dated.altitude,datetime:result.datetime};bakedRoot=result.imageRoot;bakedLevels=result.levels;await loadIllumination();draw();
}
async function loadIllumination(){
 const token=++illuminationRequest,level=currentGeometry.level,nlevel=Math.min(7,level+(resolution==='detail'?1:0)),nfile=base+`normal-L${nlevel}.png`,sfile=bakedRoot+`shade-L${level}.png`;
 if(!textures.has(nfile))await image(nfile,1);
 if(!textures.has(sfile))await image(sfile,2,true);
 if(token!==illuminationRequest)return;
 normalLevel=nlevel;shadeLevel=level;gl.activeTexture(gl.TEXTURE1);gl.bindTexture(gl.TEXTURE_2D,textures.get(nfile));gl.activeTexture(gl.TEXTURE2);gl.bindTexture(gl.TEXTURE_2D,textures.get(sfile));
}
$('sunBake').addEventListener('click',async()=>{
 const requested=solarControls.request()||selectedSun(),generation=++sunGeneration;sunBusy=true;sunError='';updateSunStatus();$('sunStatus').textContent='指定した太陽位置の縮小陰影を確認中…';
 try{
  const response=await fetch('/national-sun/start',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(requested)});let result=await response.json();if(!response.ok)throw Error(result.error);
  while(result.state==='running'){
   $('sunStatus').textContent=`${requested.datetime?requested.datetime+' JST':'方角 '+requested.azimuth+'°・高さ '+requested.altitude+'°'} の縮小陰影を生成中：${result.completed.toLocaleString()} / ${result.total.toLocaleString()} タイル。全国 L14 から地点ごとに計算するため数分かかります。現在の表示・操作は使えます。`;
   await new Promise(resolve=>setTimeout(resolve,2000));if(generation!==sunGeneration)return;
   const query=new URLSearchParams(requested);const poll=await fetch('/national-sun/status?'+query);result=await poll.json();if(!poll.ok)throw Error(result.error);
  }
  if(result.state!=='complete')throw Error(result.error||'縮小陰影を生成できませんでした');await applySunImage(result);
 }catch(e){sunError=e.message;$('sunStatus').textContent=e.message;console.error(e);}
 finally{sunBusy=false;draw();}
});

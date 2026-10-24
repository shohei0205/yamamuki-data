"use strict";
function readVar(data,state){let value=0,mul=1;for(let i=0;i<5;i++){if(state.at>=data.length)throw Error('可変長整数の切断');const b=data[state.at++];value+=(b%128)*mul;if(b<128)return value;mul*=128}throw Error('可変長整数超過')}
let wasmDecoder=null;
const wasmReady=typeof document!=='undefined'?import('/xyz_wasm.mjs').then(async m=>{const response=await fetch('/xyz_decoder.wasm');if(!response.ok)throw Error('WASM取得失敗');wasmDecoder=await m.createXYZDecoder(await response.arrayBuffer());return wasmDecoder}):null;
const unzig=v=>v%2?-(Math.floor(v/2)+1):v/2;
function decodeTFC(data,count,nv){if(data[0]!==194)throw Error('TFC2 magic');const state={at:1},edges=[],recent=[],out=new Uint32Array(count*3);let last=0;
for(let i=0;i<count;i++){const code=data[state.at++];let a,b,c;
if(code===240){let prev=last;const f=[];for(let j=0;j<3;j++){prev+=unzig(readVar(data,state));f.push(prev)}[a,b,c]=f}
else if(code<240){const edge=Math.floor(code/16),tag=code%16;if(!edges[edge])throw Error('共有辺範囲');[a,b]=edges[edge];if(tag<12){if(tag>=recent.length)throw Error('LRU範囲');c=recent[tag]}else if(tag===12)c=a+1;else if(tag===13)c=b+1;else if(tag===14)c=last+1;else{const v=readVar(data,state);c=(v%2?b:last)+unzig(Math.floor(v/2))}}
else throw Error('TFC2 code');if(Math.min(a,b,c)<0||Math.max(a,b,c)>=nv||a===b||b===c||a===c)throw Error('面番号');out.set([a,b,c],i*3);edges.unshift([b,a],[c,b],[a,c]);edges.length=Math.min(edges.length,15);for(const v of [a,b,c]){const at=recent.indexOf(v);if(at>=0)recent.splice(at,1);recent.unshift(v);recent.length=Math.min(recent.length,12)}last=c}
if(state.at!==data.length)throw Error('TFC2末尾');return out}
async function decodeYMV(data,z,x,y,resolve){const view=new DataView(data.buffer,data.byteOffset,data.byteLength);if(String.fromCharCode(...data.subarray(0,4))!=='YMV4'||view.getUint16(4,true)!==4)throw Error('YMV4形式');const mode=view.getUint16(6,true),v=view.getUint32(8,true),d=view.getUint32(12,true),r=view.getUint32(16,true),a=view.getUint32(20,true),p=view.getUint32(24,true);
if(v!==r+a||d>256||data.length!==32+d*12+p+a*6)throw Error('YMV4長');const parents=await Promise.all(Array.from({length:d},(_,i)=>{const at=32+i*12;return resolve(view.getUint32(at,true),view.getInt32(at+4,true),view.getInt32(at+8,true)).then(v=>v.points)}));
let at=32+d*12;const refs=[];
if(mode===0){if(p!==r*4)throw Error('生参照長');for(let i=0;i<r;i++)refs.push(view.getUint32(at+i*4,true))}
else if(mode===1){const bytes=data.subarray(at,at+p),s={at:2};const groups=bytes[0]+256*bytes[1];for(let g=0;g<groups;g++){const slot=bytes[s.at++],n=readVar(bytes,s);let index=readVar(bytes,s);for(let i=0;i<n;i++){if(i)index+=readVar(bytes,s)+1;refs.push(slot*16777216+index)}}if(s.at!==p||refs.length!==r)throw Error('参照個数')}
else throw Error('参照方式');const out=new Float64Array(v*3);for(let i=0;i<r;i++){const slot=Math.floor(refs[i]/16777216),index=refs[i]%16777216;const parent=parents[slot];if(!parent||index*3+2>=parent.length)throw Error('親参照範囲');out.set(parent.subarray(index*3,index*3+3),i*3)}
at+=p;const k=2**(14-z);for(let i=0;i<a;i++){const q=at+i*6;out.set([(x*16384+view.getInt16(q,true))*k,(y*16384+view.getInt16(q+2,true))*k,view.getUint16(q+4,true)],(r+i)*3)}return out}
if(typeof module!=='undefined')module.exports={decodeTFC,decodeYMV};
if(typeof document!=='undefined'){try{main()}catch(e){document.getElementById('status').textContent='エラー：'+e.message}}
function main(){const $=id=>document.getElementById(id),canvas=$('canvas'),gl=canvas.getContext('webgl2',{antialias:true});if(!gl){$('status').textContent='WebGL 2 が必要です';return}
const shader=(type,source)=>{const s=gl.createShader(type);gl.shaderSource(s,source);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s};
const program=gl.createProgram();gl.attachShader(program,shader(gl.VERTEX_SHADER,`#version 300 es
in vec3 position;in vec3 normal;uniform mat4 mvp;out vec3 N;out vec3 P;void main(){P=position;N=normal;gl_Position=mvp*vec4(position,1.);}`));
gl.attachShader(program,shader(gl.FRAGMENT_SHADER,`#version 300 es
precision highp float;in vec3 N;in vec3 P;uniform bool smoothShade;uniform bool wire;uniform float strength;out vec4 color;void main(){if(wire){color=vec4(.08,.12,.08,1);return;}vec3 n=smoothShade?normalize(N):normalize(cross(dFdx(P),dFdy(P)));if(n.y<0.)n=-n;vec3 light=normalize(vec3(-.5,1.,-.6));float flatLight=dot(vec3(0,1,0),light);float shade=clamp(.82+(dot(n,light)-flatLight)*strength,.15,1.2);vec3 base=mix(vec3(.40,.49,.32),vec3(.69,.65,.51),clamp(P.y/3800.,0.,1.));color=vec4(base*shade,1);}`));gl.linkProgram(program);if(!gl.getProgramParameter(program,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(program));gl.useProgram(program);
const uniforms=Object.fromEntries(['mvp','smoothShade','wire','strength'].map(n=>[n,gl.getUniformLocation(program,n)]));
const vao=gl.createVertexArray();gl.bindVertexArray(vao);const pb=gl.createBuffer(),nb=gl.createBuffer(),ib=gl.createBuffer(),wb=gl.createBuffer();for(const [name,buffer]of [['position',pb],['normal',nb]]){gl.bindBuffer(gl.ARRAY_BUFFER,buffer);const at=gl.getAttribLocation(program,name);gl.enableVertexAttribArray(at);gl.vertexAttribPointer(at,3,gl.FLOAT,false,0,0)}
let indices=null,lines=null,cache=new Map(),prefetched=new Map(),identities=new Map(),listCache=new Map(),metadataCache=new Map(),sceneCache=new Map(),fetchController=null,meta=null,serial=0,dist=30000,yaw=0,pitch=.85,target=[0,500,0],needsDraw=true;
for(let z=14;z>=6;z--){const opt=document.createElement('option');opt.value=z;opt.textContent=`z${z}・誤差 ${12.5*2**(14-z)} m`;$('level').append(opt)}$('level').value=12;
const query=new URLSearchParams(location.search);for(const [id,key]of [['region','region'],['level','z'],['span','span']])if(query.has(key))$(id).value=query.get(key);
function reset(){dist=Number($('span').value)*.9;yaw=0;pitch=.85;target=[0,500,0];needsDraw=true}$('reset').onclick=reset;
async function tile(region,z,x,y){
const key=`${region}/${z}/${x}/${y}`;let promise=cache.get(key);
if(!promise){promise=(async()=>{const stored=prefetched.get(key);if(!stored?.vertices?.length)throw Error('未取得の頂点 '+key);return ($('decoder').value==='wasm'?wasmDecoder.decodeYMV:decodeYMV)(stored.vertices,z,x,y,(a,b,c)=>tile(region,a,b,c))})();cache.set(key,promise);promise.catch(()=>cache.delete(key));}
const points=await promise,stored=prefetched.get(key);return {points,triangles:stored?.triangles,bytes:(stored?.vertices?.byteLength||0)+(stored?.triangles?.byteLength||0)};
}
function uploadScene(scene){lines=null;indices=scene.indices;for(const [buffer,data]of [[pb,scene.vertices],[nb,scene.normals]]){gl.bindBuffer(gl.ARRAY_BUFFER,buffer);gl.bufferData(gl.ARRAY_BUFFER,data,gl.STATIC_DRAW)}gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,ib);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,indices,gl.STATIC_DRAW);needsDraw=true;}
function setURL(region,z,span){const url=new URL(location);url.search=new URLSearchParams({region,z,span});history.replaceState(null,'',url);}
async function load(){const token=++serial,region=$('region').value,z=Number($('level').value),span=Number($('span').value);if(fetchController)fetchController.abort();fetchController=new AbortController();const signal=fetchController.signal;const loadStarted=performance.now();$('status').textContent='地域情報を確認中…';
try{if($('decoder').value==='wasm'){await wasmReady;if(token!==serial)return;}let loadedMeta=metadataCache.get(region);if(!loadedMeta||performance.now()-loadedMeta.checkedAt>5000){loadedMeta=await(await fetch('/api/meta?region='+region,{signal})).json();loadedMeta.checkedAt=performance.now();metadataCache.set(region,loadedMeta)}if(token!==serial)return;meta=loadedMeta;
if(identities.get(region)!==meta.identity){for(const key of prefetched.keys())if(key.startsWith(region+'/')){prefetched.delete(key);cache.delete(key)}identities.set(region,meta.identity)}
const listKey=`${region}/${z}/${span}/${meta.identity}`,sceneKey=listKey+'/'+$('decoder').value;
const existingScene=sceneCache.get(sceneKey);if(existingScene){sceneCache.delete(sceneKey);sceneCache.set(sceneKey,existingScene);const started=performance.now();uploadScene(existingScene);$('status').textContent=existingScene.label+` ／ 組立済み再利用・転送 0 MiB ／ GPU転送 ${((performance.now()-started)/1000).toFixed(3)}s`;reset();setURL(region,z,span);return;}
let rows=listCache.get(listKey),newBytes=0;
const ready=rows&&rows.every(r=>{const key=`${region}/${z}/${r[0]}/${r[1]}`,value=prefetched.get(key);return value?.vertices?.length&&value?.triangles?.length&&cache.has(key)});
if(ready){$('status').textContent='取得済みタイルを再利用中…'}else{
$('status').textContent='未取得データを確認中…';await new Promise(requestAnimationFrame);if(token!==serial)return;
const known=[];for(const [key,value]of prefetched){if(!key.startsWith(region+'/'))continue;const [,zz,x,y]=key.split('/');const level=Number(zz);if(level<z-2||level>z)continue;known.push([level,Number(x),Number(y),!!value.vertices?.length,!!value.triangles?.length]);}
const response=await fetch(`/api/bundle?region=${region}&z=${z}&span=${span}`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({known}),signal});if(!response.ok)throw Error('一括取得に失敗');
const reader=response.body.getReader(),chunks=[];let received=0;const expected=Number(response.headers.get('Content-Length'));
for(;;){const {value,done}=await reader.read();if(done)break;chunks.push(value);received+=value.length;if(token===serial)$('status').textContent=`地形データを取得中 ${(received/1048576).toFixed(2)}${expected?' / '+(expected/1048576).toFixed(2):''} MiB`;}
const bytes=new Uint8Array(received);let cursor=0;for(const chunk of chunks){bytes.set(chunk,cursor);cursor+=chunk.length}const bundle=bytes.buffer,header=new DataView(bundle),manifestLength=header.getUint32(0,true),manifest=JSON.parse(new TextDecoder().decode(new Uint8Array(bundle,4,manifestLength)));rows=manifest.rows;let offset=4+manifestLength;const incoming=new Map();
for(const [zz,x,y,vb,tb] of manifest.entries){const key=`${region}/${zz}/${x}/${y}`,old=prefetched.get(key)||{};incoming.set(key,{vertices:vb?new Uint8Array(bundle,offset,vb).slice():old.vertices,triangles:tb?new Uint8Array(bundle,offset+vb,tb).slice():old.triangles});offset+=vb+tb}if(offset!==bundle.byteLength)throw Error('一括データ長');
if(token!==serial)return;for(const [key,value]of incoming){prefetched.delete(key);prefetched.set(key,value)}newBytes=manifest.entries.reduce((s,e)=>s+e[3]+e[4],0);listCache.set(listKey,rows);
}
const fetchedAt=performance.now();const results=new Array(rows.length);let next=0,done=0;await Promise.all(Array.from({length:Math.min(6,rows.length)},async()=>{while(next<rows.length){const i=next++,r=rows[i];results[i]=await tile(region,z,r[0],r[1]);if(token!==serial)return;$('status').textContent=`読込・ブラウザー復元 ${++done} / ${rows.length} タイル`}}));if(token!==serial)return;
const decodedAt=performance.now();$('status').textContent='面の重複除去・陰影を準備中…';await new Promise(requestAnimationFrame);
const map=new Map(),faceSet=new Set(),pos=[],faces=[];
const faceBase=results.reduce((sum,t)=>sum+t.points.length/3,0)+1,useNumericFaces=faceBase**3<=Number.MAX_SAFE_INTEGER;let bytes=0;const centerX=(meta.px+(meta.n-1)/2)*64,centerY=(meta.py+(meta.n-1)/2)*64,unit=40075016.68557849/268435456;
for(const t of results){bytes+=t.bytes;const p=t.points,remap=new Uint32Array(p.length/3);for(let i=0;i<remap.length;i++){const gx=p[i*3],gy=p[i*3+1],h=p[i*3+2],key=(gx-meta.px*64)*((meta.n-1)*64+1)+(gy-meta.py*64);let id=map.get(key);if(id==null){id=pos.length/3;map.set(key,id);pos.push((gx-centerX)*unit,h<24000?(h-24000)*.5:(h-24000)*.25,(gy-centerY)*unit)}remap[i]=id}
const tv=new DataView(t.triangles.buffer,t.triangles.byteOffset,t.triangles.byteLength);if(tv.getUint32(0,true)!==0x33544d59)throw Error('YMT3 magic');const count=tv.getUint32(12,true),length=tv.getUint32(16,true);const f=($('decoder').value==='wasm'?wasmDecoder.decodeTFC:decodeTFC)(t.triangles.subarray(24,24+length),count,remap.length);
for(let i=0;i<f.length;i+=3){const a=remap[f[i]],b=remap[f[i+1]],c=remap[f[i+2]],lo=Math.min(a,b,c),hi=Math.max(a,b,c),middle=a+b+c-lo-hi,key=useNumericFaces?(lo*faceBase+middle)*faceBase+hi:lo+','+middle+','+hi;if(!faceSet.has(key)){faceSet.add(key);faces.push(a,b,c)}}}
const vertices=new Float32Array(pos),normals=new Float32Array(pos.length);
for(let i=0;i<faces.length;i+=3){
const a=faces[i]*3,b=faces[i+1]*3,c=faces[i+2]*3;
const ux=pos[b]-pos[a],uy=pos[b+1]-pos[a+1],uz=pos[b+2]-pos[a+2],vx=pos[c]-pos[a],vy=pos[c+1]-pos[a+1],vz=pos[c+2]-pos[a+2];
const nx=uy*vz-uz*vy,ny=uz*vx-ux*vz,nz=ux*vy-uy*vx;
normals[a]+=nx;normals[a+1]+=ny;normals[a+2]+=nz;normals[b]+=nx;normals[b+1]+=ny;normals[b+2]+=nz;normals[c]+=nx;normals[c+1]+=ny;normals[c+2]+=nz;
}
for(let i=0;i<normals.length;i+=3){const length=Math.hypot(normals[i],normals[i+1],normals[i+2])||1;normals[i]/=length;normals[i+1]/=length;normals[i+2]/=length;}
if(token!==serial)return;const scene={vertices,normals,indices:new Uint32Array(faces)};uploadScene(scene);
$('status').textContent=`${({fuji:'富士山',yari:'槍ヶ岳',tsukuba:'筑波山'})[meta.name]||meta.name} ／ z${z} ／ ${rows.length} タイル ／ ${map.size.toLocaleString()} 頂点 ／ ${(faces.length/3).toLocaleString()} 三角形 ／ 表示タイルBLOB ${(bytes/1048576).toFixed(2)} MiB（親参照を除く） ／ ${$('decoder').value==='wasm'?'WASM復元':'JavaScript復元'} ／ 今回取得 ${(newBytes/1048576).toFixed(3)} MiB ／ 取得 ${((fetchedAt-loadStarted)/1000).toFixed(2)}s・復元 ${((decodedAt-fetchedAt)/1000).toFixed(2)}s・組立 ${((performance.now()-decodedAt)/1000).toFixed(2)}s`;
scene.label=`${({fuji:'富士山',yari:'槍ヶ岳',tsukuba:'筑波山'})[meta.name]||meta.name} ／ z${z} ／ ${rows.length} タイル ／ ${map.size.toLocaleString()} 頂点 ／ ${(faces.length/3).toLocaleString()} 三角形 ／ ${$('decoder').value==='wasm'?'WASM復元':'JavaScript復元'}`;
scene.size=vertices.byteLength+normals.byteLength+scene.indices.byteLength;sceneCache.set(sceneKey,scene);
let sceneBytes=0;for(const value of sceneCache.values())sceneBytes+=value.size;
for(const key of sceneCache.keys()){if(sceneBytes<=128*1048576)break;sceneBytes-=sceneCache.get(key).size;sceneCache.delete(key)}
let retained=0;for(const value of prefetched.values())retained+=(value.vertices?.byteLength||0)+(value.triangles?.byteLength||0);
// 圧縮BLOBの上限は128 MiB。復元済み座標は別途メモリーを使う。
for(const key of prefetched.keys()){if(retained<=128*1048576)break;const value=prefetched.get(key);retained-=(value.vertices?.byteLength||0)+(value.triangles?.byteLength||0);prefetched.delete(key);cache.delete(key)}
reset();setURL(region,z,span);
}catch(e){if(token===serial)$('status').textContent='エラー：'+e.message}}
$('decoder').onchange=()=>{cache.clear();load()};
for(const id of ['region','level','span'])$(id).onchange=load;for(const id of ['smooth','wire','strength'])$(id).oninput=()=>{needsDraw=true};
let drag=null;canvas.onpointerdown=e=>{drag=[e.clientX,e.clientY];canvas.setPointerCapture(e.pointerId)};canvas.onpointerup=()=>drag=null;canvas.onpointermove=e=>{if(!drag)return;const dx=e.clientX-drag[0],dy=e.clientY-drag[1];drag=[e.clientX,e.clientY];if(e.shiftKey){const scale=dist*.001;target[0]+=(-dx*Math.cos(yaw)-dy*Math.sin(yaw))*scale;target[2]+=(dx*Math.sin(yaw)-dy*Math.cos(yaw))*scale}else{yaw-=dx*.006;pitch=Math.max(.03,Math.min(1.55,pitch+dy*.006))}needsDraw=true};canvas.onwheel=e=>{e.preventDefault();dist=Math.max(100,Math.min(1000000,dist*Math.exp(e.deltaY*.001)));needsDraw=true};
function matrix(){const eye=[target[0]+dist*Math.cos(pitch)*Math.sin(yaw),target[1]+dist*Math.sin(pitch),target[2]+dist*Math.cos(pitch)*Math.cos(yaw)];const z=eye.map((v,i)=>(v-target[i])/dist),x=[z[2],0,-z[0]],xl=Math.hypot(...x);for(let i=0;i<3;i++)x[i]/=xl;const y=[z[1]*x[2],z[2]*x[0]-z[0]*x[2],-z[1]*x[0]];const dot=a=>-a.reduce((s,v,i)=>s+v*eye[i],0);const v=[x[0],y[0],z[0],0,x[1],y[1],z[1],0,x[2],y[2],z[2],0,dot(x),dot(y),dot(z),1];const near=Math.max(1,dist/10000),far=Math.max(1000000,dist*10),f=1/Math.tan(Math.PI/8),aspect=canvas.width/canvas.height,p=[f/aspect,0,0,0,0,f,0,0,0,0,(far+near)/(near-far),-1,0,0,2*far*near/(near-far),0],out=new Float32Array(16);for(let c=0;c<4;c++)for(let r=0;r<4;r++)for(let k=0;k<4;k++)out[c*4+r]+=p[k*4+r]*v[c*4+k];return out}
function draw(){requestAnimationFrame(draw);const ratio=Math.min(devicePixelRatio||1,2),w=Math.round(canvas.clientWidth*ratio),h=Math.round(canvas.clientHeight*ratio);if(w!==canvas.width||h!==canvas.height){canvas.width=w;canvas.height=h;needsDraw=true}if(!needsDraw)return;needsDraw=false;gl.viewport(0,0,w,h);gl.clearColor(.90,.945,.98,1);gl.clear(gl.COLOR_BUFFER_BIT|gl.DEPTH_BUFFER_BIT);if(!indices)return;gl.enable(gl.DEPTH_TEST);gl.uniformMatrix4fv(uniforms.mvp,false,matrix());gl.uniform1i(uniforms.smoothShade,$('smooth').checked);gl.uniform1f(uniforms.strength,Number($('strength').value));gl.uniform1i(uniforms.wire,false);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,ib);gl.enable(gl.POLYGON_OFFSET_FILL);gl.polygonOffset(1,1);gl.drawElements(gl.TRIANGLES,indices.length,gl.UNSIGNED_INT,0);gl.disable(gl.POLYGON_OFFSET_FILL);if($('wire').checked){if(!lines){lines=new Uint32Array(indices.length*2);for(let i=0,j=0;i<indices.length;i+=3){const a=indices[i],b=indices[i+1],c=indices[i+2];lines.set([a,b,b,c,c,a],j);j+=6}gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,wb);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,lines,gl.STATIC_DRAW)}else gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,wb);gl.uniform1i(uniforms.wire,true);gl.drawElements(gl.LINES,lines.length,gl.UNSIGNED_INT,0)}}
reset();load();draw();}

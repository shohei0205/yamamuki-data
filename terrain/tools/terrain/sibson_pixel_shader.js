// Sibson面積座標を画素ごとに評価する。高さ画像の補間ではない。
const SIBSON_PIXEL_GLSL=`
uniform highp sampler2D uSibsonData;
uniform highp usampler2D uSibsonCells;
uniform highp usampler2D uSibsonRefs;
const int SV=24;
const int SC=48;
vec4 sibData(uint i){int w=textureSize(uSibsonData,0).x;return texelFetch(uSibsonData,ivec2(int(i)%w,int(i)/w),0);}
uint sibRef(uint i){int w=textureSize(uSibsonRefs,0).x;return texelFetch(uSibsonRefs,ivec2(int(i)%w,int(i)/w),0).x;}
uvec2 sibCell(vec2 q){ivec2 size=textureSize(uSibsonCells,0);ivec2 c=ivec2(clamp(floor((q+4000.)/8000.*vec2(size)),vec2(0),vec2(size-1)));return texelFetch(uSibsonCells,c,0).xy;}
int sibClip(inout vec2 p[SV],inout int owners[SV],int n,vec2 a,float c,int owner){
 vec2 outP[SV];int outOwner[SV];int m=0;if(n==0)return 0;vec2 s=p[n-1];float ds=dot(a,s)-c;
 for(int i=0;i<SV;i++){if(i>=n)break;vec2 e=p[i];float de=dot(a,e)-c;
  if((ds<=0.)!=(de<=0.)){if(m>=SV)return -1;outP[m]=mix(s,e,ds/(ds-de));outOwner[m]=ds<=0.?owners[i]:owner;m++;}
  if(de<=0.){if(m>=SV)return -1;outP[m]=e;outOwner[m]=owners[i];m++;}s=e;ds=de;
 }
 for(int i=0;i<SV;i++){if(i>=m)break;p[i]=outP[i];owners[i]=outOwner[i];}return m;
}
int sibClipPlain(inout vec2 p[SV],int n,vec2 a,float c){
 vec2 outP[SV];int m=0;if(n==0)return 0;vec2 s=p[n-1];float ds=dot(a,s)-c;
 for(int i=0;i<SV;i++){if(i>=n)break;vec2 e=p[i];float de=dot(a,e)-c;
  if((ds<=0.)!=(de<=0.)){if(m>=SV)return -1;outP[m++]=mix(s,e,ds/(ds-de));}
  if(de<=0.){if(m>=SV)return -1;outP[m++]=e;}s=e;ds=de;
 }for(int i=0;i<SV;i++){if(i>=m)break;p[i]=outP[i];}return m;
}
float sibArea(vec2 p[SV],int n){if(n<3)return 0.;float sum=0.;for(int i=1;i<SV-1;i++){if(i+1>=n)break;vec2 a=p[i]-p[0],b=p[i+1]-p[0];sum+=a.x*b.y-a.y*b.x;}return abs(sum)*.5;}
vec2 sibsonHeight(vec2 q){
 uvec2 cell=sibCell(q);if(cell.y==0xffffffffu||cell.y==0u||cell.y>uint(SC))return vec2(0,0);
 float bound=1.;for(int i=0;i<SC;i++){if(uint(i)>=cell.y)break;vec4 p=sibData(sibRef(cell.x+uint(i)));float d=length(p.xy-q);if(d<.0001)return vec2(p.z,1);bound=max(bound,d*2.);}
 vec2 polygon[SV];int owners[SV];int n=0;bool open=true;
 for(int attempt=0;attempt<3;attempt++){
  polygon[0]=vec2(-bound,-bound);polygon[1]=vec2(bound,-bound);polygon[2]=vec2(bound,bound);polygon[3]=vec2(-bound,bound);for(int i=0;i<4;i++)owners[i]=-1;n=4;
  for(int i=0;i<SC;i++){if(uint(i)>=cell.y)break;vec2 d=sibData(sibRef(cell.x+uint(i))).xy-q;float r=length(d);n=sibClip(polygon,owners,n,d/r,r*.5,i);if(n<0)return vec2(0,0);}
  open=false;for(int i=0;i<SV;i++){if(i>=n)break;if(owners[i]<0)open=true;}if(!open)break;bound*=8.;
 }if(open||n<3)return vec2(0,0);
 float sum=0.,value=0.,base=sibData(sibRef(cell.x)).z;
 for(int k=0;k<SV;k++){if(k>=n)break;int id=owners[k];bool duplicate=false;for(int j=0;j<SV;j++){if(j>=k)break;if(owners[j]==id)duplicate=true;}if(duplicate||id<0)continue;
  uint record=sibRef(cell.x+uint(id));vec4 site=sibData(record);int degree=int(site.w);if(degree>64)return vec2(0,0);
  vec2 piece[SV];for(int j=0;j<SV;j++){if(j>=n)break;piece[j]=polygon[j];}int m=n;
  for(int j=0;j<64;j++){if(j>=degree||m==0)break;vec4 plane=sibData(record+1u+uint(j));m=sibClipPlain(piece,m,plane.xy,plane.z-dot(q-site.xy,plane.xy));if(m<0)return vec2(0,0);}
  float w=sibArea(piece,m);sum+=w;value+=w*(site.z-base);
 }
 float area=sibArea(polygon,n);if(sum<=1e-10||abs(sum-area)>max(.001,area*.005))return vec2(0,0);return vec2(base+value/sum,1);
}
`;

function sibsonPixelTexture(gl,internal,format,type,data,components,count,width=0,height=0){
 if(!width){width=Math.min(1024,2**Math.ceil(Math.log2(Math.max(1,Math.sqrt(count)))));height=Math.max(1,Math.ceil(count/width));}
 if(width>gl.getParameter(gl.MAX_TEXTURE_SIZE)||height>gl.getParameter(gl.MAX_TEXTURE_SIZE))throw Error('GPUテクスチャの上限を超えました');
 const padded=new data.constructor(width*height*components);padded.set(data);const texture=gl.createTexture();gl.bindTexture(gl.TEXTURE_2D,texture);gl.pixelStorei(gl.UNPACK_ALIGNMENT,1);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);gl.texImage2D(gl.TEXTURE_2D,0,internal,width,height,0,format,type,padded);return texture;
}
function sibsonPixelUpload(gl,buffer){
 const size=new DataView(buffer).getUint32(0,true),info=JSON.parse(new TextDecoder().decode(new Uint8Array(buffer,4,size)));let offset=4+size;
 const points=new Float32Array(buffer.slice(offset,offset+info.pointBytes));offset+=info.pointBytes;
 const cells=new Uint32Array(buffer.slice(offset,offset+info.cellBytes));offset+=info.cellBytes;
 const refs=new Uint32Array(buffer.slice(offset,offset+info.referenceBytes));
 const textures=[sibsonPixelTexture(gl,gl.RGBA32F,gl.RGBA,gl.FLOAT,points,4,info.pointRecords),sibsonPixelTexture(gl,gl.RG32UI,gl.RG_INTEGER,gl.UNSIGNED_INT,cells,2,info.gridN*info.gridN,info.gridN,info.gridN),sibsonPixelTexture(gl,gl.R32UI,gl.RED_INTEGER,gl.UNSIGNED_INT,refs,1,info.referenceCount)];
 return{info,bytes:buffer.byteLength,textures,bind(program){for(let i=0;i<3;i++){gl.activeTexture(gl.TEXTURE0+i);gl.bindTexture(gl.TEXTURE_2D,textures[i]);gl.uniform1i(gl.getUniformLocation(program,['uSibsonData','uSibsonCells','uSibsonRefs'][i]),i);}},destroy(){textures.forEach(t=>gl.deleteTexture(t));}};
}

function sibsonPixelDummy(gl){
 const textures=[sibsonPixelTexture(gl,gl.RGBA32F,gl.RGBA,gl.FLOAT,new Float32Array(4),4,1),sibsonPixelTexture(gl,gl.RG32UI,gl.RG_INTEGER,gl.UNSIGNED_INT,new Uint32Array(2),2,1),sibsonPixelTexture(gl,gl.R32UI,gl.RED_INTEGER,gl.UNSIGNED_INT,new Uint32Array(1),1,1)];
 return{bind(program){for(let i=0;i<3;i++){gl.activeTexture(gl.TEXTURE0+i);gl.bindTexture(gl.TEXTURE_2D,textures[i]);gl.uniform1i(gl.getUniformLocation(program,['uSibsonData','uSibsonCells','uSibsonRefs'][i]),i);}}};
}

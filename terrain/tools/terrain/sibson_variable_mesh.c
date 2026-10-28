/* 表示用場の補間・混合LOD・辺整合とメッシュ組み立て。ブラウザーWorker内で実行する。 */
#include <stdint.h>
#define STRIDE 16
static double *tiles;
static int tile_count,join_enabled;
static int32_t *adj,*ranges;
static double maxd(double a,double b){return a>b?a:b;}
static double mind(double a,double b){return a<b?a:b;}
static double absd(double a){return a<0?-a:a;}
static double *tile(int i){return tiles+i*STRIDE;}
static int near(int a,int b){double *x=tile(a),*y=tile(b);return y[0]<=x[2]+250&&y[2]>=x[0]-250&&y[1]<=x[3]+250&&y[3]>=x[1]-250;}
__attribute__((visibility("default"))) int prepare_neighbors(int count,double *metadata,int32_t *range,int32_t *neighbors,int capacity){
 tiles=metadata;tile_count=count;ranges=range;adj=neighbors;int used=0;
 for(int a=0;a<count;a++){ranges[2*a]=used;for(int b=0;b<count;b++)if(near(a,b)){if(used>=capacity)return -1;adj[used++]=b;}ranges[2*a+1]=used-ranges[2*a];}return used;
}
static double coordinate(double value,double core,double side,int n,double low,double high){int offset=low<core?1:0;if(value<core)return maxd(0,(value-low)/(core-low));if(value>core+side)return offset+n+mind(1,(value-core-side)/(high-core-side));return offset+(value-core)/side*n;}
static void evaluate(int owner,double x,double y,double *out){
 double *m=tile(owner);int n=(int)m[10],nx=(int)m[11],ny=(int)m[12];float *data=(float*)(uintptr_t)(uint32_t)m[13];double u=coordinate(x,m[0],m[4],n,m[6],m[8]),v=coordinate(y,m[1],m[4],n,m[7],m[9]);int i=(int)mind(nx-1,__builtin_floor(u)),j=(int)mind(ny-1,__builtin_floor(v));double a=u-i,c=v-j;
 out[0]=out[1]=out[2]=0;
 for(int k=0;k<4;k++){int dx=k&1,dy=k>>1;double w=(dx?a:1-a)*(dy?c:1-c);int offset=((j+dy)*(nx+1)+i+dx)*3;for(int axis=0;axis<3;axis++)out[axis]+=w*data[offset+axis];}
}
static void ramp(double value,double *v,double *d){double t=maxd(0,mind(1,value));*v=t*t*(3-2*t);*d=t>0&&t<1?6*t*(1-t):0;}
static int within(int index,double x,double y){double *m=tile(index);return x>=m[0]-250&&x<=m[2]+250&&y>=m[1]-250&&y<=m[3]+250;}
static void blended(int owner,double x,double y,double *out){
 int begin=ranges[2*owner],end=begin+ranges[2*owner+1],mixed=0;double *own=tile(owner);
 if(join_enabled)for(int k=begin;k<end;k++){int index=adj[k];if(tile(index)[5]!=own[5]&&within(index,x,y)){mixed=1;break;}}
 if(!mixed){evaluate(owner,x,y,out);return;}
 double sum=0,h=0,gx=0,gy=0,sx=0,sy=0;
 for(int k=begin;k<end;k++){
  int index=adj[k];if(!within(index,x,y))continue;double *m=tile(index),r[4],d[4],value[3];ramp((x-m[0]+250)/500,&r[0],&d[0]);ramp((m[2]-x+250)/500,&r[1],&d[1]);ramp((y-m[1]+250)/500,&r[2],&d[2]);ramp((m[3]-y+250)/500,&r[3],&d[3]);
  double w=r[0]*r[1]*r[2]*r[3],wx=(d[0]*r[1]-r[0]*d[1])*r[2]*r[3]/500,wy=(d[2]*r[3]-r[2]*d[3])*r[0]*r[1]/500;if(!w)continue;evaluate(index,x,y,value);sum+=w;h+=w*value[0];gx+=w*value[1]+wx*value[0];gy+=w*value[2]+wy*value[0];sx+=wx;sy+=wy;
 }
 if(!sum){evaluate(owner,x,y,out);return;}out[0]=h/sum;out[1]=(gx-h/sum*sx)/sum;out[2]=(gy-h/sum*sy)/sum;
}
static void stitched(int owner,double x,double y,double *out){
 double step=0,start=0;int vertical=0,coarse=owner,begin=ranges[2*owner],end=begin+ranges[2*owner+1];
 for(int k=begin;k<end;k++){int index=adj[k];double *m=tile(index),s=m[4]/m[10];if(s<=step)continue;
  if((absd(x-m[0])<.01||absd(x-m[2])<.01)&&y>=m[1]-.01&&y<=m[3]+.01){step=s;vertical=1;start=m[1];coarse=index;}
  if((absd(y-m[1])<.01||absd(y-m[3])<.01)&&x>=m[0]-.01&&x<=m[2]+.01){step=s;vertical=0;start=m[0];coarse=index;}
 }
 double *own=tile(owner);if(step>own[4]/own[10]+1e-6){double q=vertical?y:x,lo=start+__builtin_floor((q-start)/step)*step,t=(q-lo)/step,a[3],b[3];blended(coarse,vertical?x:lo,vertical?lo:y,a);blended(coarse,vertical?x:lo+step,vertical?lo+step:y,b);for(int k=0;k<3;k++)out[k]=a[k]*(1-t)+b[k]*t;return;}blended(owner,x,y,out);
}
__attribute__((visibility("default"))) int build_meshes(int count,double *metadata,int32_t *range,int32_t *neighbors,int join,float *vertices,uint32_t *indices,uint32_t *parts){
 tiles=metadata;tile_count=count;ranges=range;adj=neighbors;join_enabled=join;uint32_t vo=0,io=0;
 for(int owner=0;owner<count;owner++){double *m=tile(owner);int n=(int)m[10];if(n<1||n>512)return owner+1;uint32_t vertex_count=(n+1)*(n+1),index_count=n*n*6;parts[4*owner]=vo;parts[4*owner+1]=vertex_count;parts[4*owner+2]=io;parts[4*owner+3]=index_count;
  for(int j=0;j<=n;j++)for(int i=0;i<=n;i++){double x=m[0]+m[4]*i/n,y=m[1]+m[4]*j/n,h[3];stitched(owner,x,y,h);uint32_t offset=(vo+j*(n+1)+i)*5;vertices[offset]=(float)x;vertices[offset+1]=(float)y;for(int axis=0;axis<3;axis++)vertices[offset+2+axis]=(float)h[axis];}
  uint32_t k=io;for(int j=0;j<n;j++)for(int i=0;i<n;i++){uint32_t a=j*(n+1)+i;indices[k++]=a;indices[k++]=a+1;indices[k++]=a+n+2;indices[k++]=a;indices[k++]=a+n+2;indices[k++]=a+n+1;}vo+=vertex_count;io+=index_count;
 }return 0;
}

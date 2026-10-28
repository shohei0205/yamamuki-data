/* Sibson の面積座標。倍精度で Voronoi 多角形を半平面切断する。 */
#include <math.h>
#include <stdint.h>
#define CAP 256
#ifdef _WIN32
#define API __declspec(dllexport)
#else
#define API
#endif
typedef struct {double x,y;} P;
static int clip(P *p,int n,double a,double b,double c){P out[CAP];int m=0;if(!n)return 0;P s=p[n-1];double ds=a*s.x+b*s.y-c;for(int i=0;i<n;i++){P e=p[i];double de=a*e.x+b*e.y-c;if((ds<=0)!=(de<=0)){double t=ds/(ds-de);if(m>=CAP)return -1;out[m++]=(P){s.x+t*(e.x-s.x),s.y+t*(e.y-s.y)};}if(de<=0){if(m>=CAP)return -1;out[m++]=e;}s=e;ds=de;}for(int i=0;i<m;i++)p[i]=out[i];return m;}
static double area(P *p,int n){double a=0;for(int i=0,j=n-1;i<n;j=i++)a+=p[j].x*p[i].y-p[j].y*p[i].x;return fabs(a)*.5;}
API int sibson(const double *xy,const double *z,const int32_t *tri,const int32_t *adj,const double *circles,const int32_t *off,const int32_t *links,const double *q,const int32_t *starts,int nq,double *out){
 for(int k=0;k<nq;k++){double x=q[2*k],y=q[2*k+1];int st=starts[k];if(st<0)return k+1;
 int stack[CAP],seen[CAP],verts[CAP],ns=1,nt=0,nv=0;stack[0]=st;
 while(ns){int t=stack[--ns],found=0;for(int j=0;j<nt;j++)if(seen[j]==t)found=1;if(found)continue;if(nt>=CAP)return k+1;seen[nt++]=t;
 double dx=circles[3*t]-x,dy=circles[3*t+1]-y;if(t!=st&&dx*dx+dy*dy>circles[3*t+2]*(1+1e-12))continue;
 for(int j=0;j<3;j++){int v=tri[3*t+j],exists=0;for(int h=0;h<nv;h++)if(verts[h]==v)exists=1;if(!exists){if(nv>=CAP)return k+1;verts[nv++]=v;}int a=adj[3*t+j];if(a>=0){if(ns>=CAP)return k+1;stack[ns++]=a;}}
 }
 int exact=-1;for(int j=0;j<nv;j++){double dx=xy[2*verts[j]]-x,dy=xy[2*verts[j]+1]-y;if(dx*dx+dy*dy<1e-18)exact=verts[j];}if(exact>=0){out[k]=z[exact];continue;}
 /* 計算原点を照会点に置き、面積の桁落ちを避ける。 */
 P cell[CAP];int n=0;double bound=128000;
 /* 凸包近くの細長いセルも人工境界で切らず、閉じるまで拡張する。 */
 for(int attempt=0;attempt<40;attempt++){cell[0]=(P){-bound,-bound};cell[1]=(P){bound,-bound};cell[2]=(P){bound,bound};cell[3]=(P){-bound,bound};n=4;
 for(int j=0;j<nv;j++){double dx=xy[2*verts[j]]-x,dy=xy[2*verts[j]+1]-y;n=clip(cell,n,dx,dy,(dx*dx+dy*dy)*.5);if(n<0)return k+1;}
 int open=0;for(int j=0;j<n;j++)if(fabs(cell[j].x)>bound*.999999||fabs(cell[j].y)>bound*.999999)open=1;
 if(!open)break;if(attempt==39)return k+1;bound*=2;
 }
 double sum=0,value=0;
 for(int j=0;j<nv;j++){int v=verts[j];P poly[CAP];for(int h=0;h<n;h++)poly[h]=cell[h];int m=n;double vx=xy[2*v]-x,vy=xy[2*v+1]-y;
 for(int h=off[v];h<off[v+1]&&m;h++){int u=links[h];double ux=xy[2*u]-x,uy=xy[2*u+1]-y;m=clip(poly,m,ux-vx,uy-vy,(ux*ux+uy*uy-vx*vx-vy*vy)*.5);if(m<0)return k+1;}
 double w=area(poly,m);sum+=w;value+=w*z[v];}
 if(sum<1e-15)return k+1;out[k]=value/sum;
 }return 0;
}

/* 事前計算した制御点のVoronoiセルを、照会点との二等分線で1回だけ切る。 */
#define sibson sibson_baseline
#include "sibson_eval.c"
#undef sibson
API int sibson_aux(const double *xy,const double *z,const int32_t *tri,const int32_t *adj,const double *circles,const int32_t *off,const int32_t *links,const int32_t *cell_off,const double *cells,const double *q,const int32_t *starts,int nq,double *out,int32_t *stats){
 stats[0]=stats[1]=stats[2]=0;
 for(int k=0;k<nq;k++){
  double x=q[2*k],y=q[2*k+1];int st=starts[k];if(st<0)return k+1;
  int stack[CAP],seen[CAP],verts[CAP],ns=1,nt=0,nv=0;stack[0]=st;
  while(ns){int t=stack[--ns],found=0;for(int j=0;j<nt;j++)if(seen[j]==t)found=1;if(found)continue;if(nt>=CAP)return k+1;seen[nt++]=t;
   double dx=circles[3*t]-x,dy=circles[3*t+1]-y;if(t!=st&&dx*dx+dy*dy>circles[3*t+2]*(1+1e-12))continue;
   for(int j=0;j<3;j++){int v=tri[3*t+j],exists=0;for(int h=0;h<nv;h++)if(verts[h]==v)exists=1;if(!exists){if(nv>=CAP)return k+1;verts[nv++]=v;}int a=adj[3*t+j];if(a>=0){if(ns>=CAP)return k+1;stack[ns++]=a;}}
  }
  int exact=-1,fallback=0;
  for(int j=0;j<nv;j++){int v=verts[j];double dx=xy[2*v]-x,dy=xy[2*v+1]-y;if(dx*dx+dy*dy<1e-18)exact=v;if(cell_off[v+1]==cell_off[v])fallback=1;}
  if(exact>=0){out[k]=z[exact];stats[2]++;continue;}
  /* 凸包の無限セルは人工的に打ち切らず、既存の確実な計算へ戻す。 */
  if(fallback){if(sibson_baseline(xy,z,tri,adj,circles,off,links,q+2*k,starts+k,1,out+k))return k+1;stats[1]++;continue;}
  double sum=0,value=0;
  for(int j=0;j<nv;j++){
   int v=verts[j],begin=cell_off[v],count=cell_off[v+1]-begin;if(count>CAP)return k+1;P poly[CAP];
   for(int h=0;h<count;h++)poly[h]=(P){cells[2*(begin+h)],cells[2*(begin+h)+1]};
   double vx=xy[2*v]-x,vy=xy[2*v+1]-y;
   int m=clip(poly,count,vx,vy,-(vx*vx+vy*vy)*.5);if(m<0)return k+1;
   double weight=area(poly,m);sum+=weight;value+=weight*z[v];
  }
  if(sum<1e-15||!isfinite(value/sum))return k+1;out[k]=value/sum;stats[0]++;
 }
 return 0;
}

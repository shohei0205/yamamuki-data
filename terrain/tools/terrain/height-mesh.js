/* 標高の中点をよく再現する対角線を、マスごとに選ぶ。 */
'use strict';
const HeightMesh={
 sample(values,u,v,size=256){const x=u*size-.5,y=v*size-.5,ix=Math.floor(x),iy=Math.floor(y),fx=x-ix,fy=y-iy,at=(a,b)=>values[Math.max(0,Math.min(size-1,b))*size+Math.max(0,Math.min(size-1,a))];return (at(ix,iy)*(1-fx)+at(ix+1,iy)*fx)*(1-fy)+(at(ix,iy+1)*(1-fx)+at(ix+1,iy+1)*fx)*fy;},
 diagonal(a,b,c,d,center,x,y){const ad=Math.abs((a+d)/2-center),bc=Math.abs((b+c)/2-center);if(Math.abs(ad-bc)>1e-6)return ad<bc;const slopeAD=Math.abs(a-d),slopeBC=Math.abs(b-c);if(Math.abs(slopeAD-slopeBC)>1e-6)return slopeAD<slopeBC;return (x+y)%2===0;},
 indices(values,range,n,edges,previous=null,size=256){const out=previous||new Uint32Array(n*n*6),sample=(x,y)=>{if(edges){let e=-1,t=0;if(y===0){e=0;t=x;}else if(x===n){e=1;t=y;}else if(y===n){e=2;t=x;}else if(x===0){e=3;t=y;}if(e>=0)return edges[(e*(n+1)+t)*2];}return this.sample(values,range[0]+x/n*range[2],range[1]+y/n*range[3],size);};
 for(let y=0;y<n;y++)for(let x=0;x<n;x++){if(previous&&x>0&&y>0&&x<n-1&&y<n-1)continue;const a=y*(n+1)+x,b=a+1,c=a+n+1,d=c+1,mid=this.sample(values,range[0]+(x+.5)/n*range[2],range[1]+(y+.5)/n*range[3],size),ad=this.diagonal(sample(x,y),sample(x+1,y),sample(x,y+1),sample(x+1,y+1),mid,x,y);out.set(ad?[a,c,d,a,d,b]:[a,c,b,b,c,d],(y*n+x)*6);}
 return out;}
};
if(typeof module!=='undefined')module.exports=HeightMesh;

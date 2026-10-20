/* Hsieh–Clough–Tocher型の3分割3次曲面。頂点の値・傾きと辺中央の共通勾配からC1接続する。 */
'use strict';
// 固定参照三角形で値・微分の42制約を解いた係数変換。地形データは含まない。
const HCT_MATRIX=[[0.52613334565,-0.047677857512,-0.031299918213,0.286789889448,-0.047798314908,0.000120457396,0.187076764902,-0.000120457396,-0.031179460817,-0.190711430048,0,-0.000481829584,0.000481829584,0,-0.125199672852],[-0.078977775725,0.982759940809,0.004077096571,0.074472441472,-0.012412073579,-0.004827985613,0.004505334253,0.004827985613,-0.000750889042,-0.068960236766,0,0.019311942451,-0.019311942451,0,0.016308386282],[0,0.004925869394,0.995074130606,-0.015263595581,0.002543932597,0.002381936797,0.015263595581,-0.002381936797,-0.002543932597,0.019703477575,0,-0.009527747188,0.009527747188,0,-0.019703477575],[-0.485290194962,-1.644904046101,0.06402234694,0.566480059139,-0.59441334319,-0.050490702911,-0.081189864177,0.050490702911,0.013531644029,1.420383815597,0,0.201962811644,-0.201962811644,0,0.256089387762],[0.011423540612,-0.010257937683,-2.987838138882,0.040622343297,-0.006770390549,-1.003487547133,-0.052045883909,0.003487547133,0.008674313985,-0.041031750731,4.0,0.013950188533,-0.013950188533,0,0.048647444472],[-0.835937345812,-1.328770229903,-0.310552661065,-0.305924159967,0.050987359995,0.620242410102,1.141861505779,-0.620242410102,-0.690310250963,0.684919080386,-2.0,-0.480969640408,0.480969640408,2.0,0.757789355739],[0.331142490383,0.764640521268,-0.042783439538,-0.376011136531,0.729335189422,0.035305331847,0.044868646148,-0.035305331847,-0.007478107691,-0.941437914926,0,-0.141221327387,0.141221327387,0,-0.171133758152],[-0.022847081224,0.00081239779,1.995379755339,-0.020190304268,0.003365050711,1.997447347079,0.043037385491,0.002552652921,-0.007172897582,0.003249591161,-4.0,0.010210611684,-0.010210611684,0,-0.018480978643],[-0.862099786952,-0.466237833147,2.322554535321,0.541585499355,-0.090264249892,1.624026416746,0.320514287597,2.375973583254,-0.053419047933,0.135048667413,-4.0,-0.496105666983,-1.503894333017,-2.0,1.290218141286],[1.06574868699,2.187691879918,-1.343400432087,0.229193833814,-0.038198972302,-1.107442481113,-1.294942520804,0.440775814446,0.882490420134,-0.58256581366,3.333333333333,0.42976992445,0.236896742216,-2.666666666667,-0.70693506168],[-0.210715950494,-0.100892995874,-0.100892995874,0.605357975247,-0.100892995874,0.666666666667,0.605357975247,0.666666666667,-0.100892995874,0.263094683169,-0.666666666667,-0.666666666667,-0.666666666667,-0.666666666667,0.263094683169],[2.777620928895,2.003640755023,-0.540703933541,-1.294235205358,0.21570586756,-3.212065112536,-1.483385723537,-2.787934887464,0.247230953923,-1.985436979907,4.0,2.848260450146,3.151739549854,2.0,-2.162815734163],[2.777620928895,-0.540703933541,2.003640755023,-1.483385723537,0.247230953923,-2.787934887464,-1.294235205358,-3.212065112536,0.21570586756,-2.162815734163,2.0,3.151739549854,2.848260450146,4.0,-1.985436979907],[-4.35767055807,-3.598020668068,1.371742241723,2.434892941693,-0.905815490282,4.307794822214,1.922777616376,3.692205177786,-0.320462936063,3.607917327727,-6.0,-3.231179288856,-4.768820711144,-2.0,3.486968966893],[-8.703917575528,-2.225326464627,-2.225326464627,4.351958787764,-0.725326464627,9.5,4.351958787764,9.5,-0.725326464627,7.098694141491,-8.0,-10.0,-10.0,-8.0,7.098694141491],[-4.35767055807,1.371742241723,-3.598020668068,1.922777616377,-0.320462936063,3.692205177786,2.434892941693,4.307794822214,-0.905815490282,3.486968966893,-2.0,-4.768820711144,-3.231179288856,-6.0,3.607917327727],[1.971624368674,1.730190387508,-0.734919659396,-1.169865575486,0.861644262581,-1.798120541739,-0.801758793188,-1.535212791594,0.133626465531,-1.745905116633,2.666666666667,1.192482166956,2.140851166377,0.666666666667,-1.606345304252],[5.937720187245,2.763340146257,0.22627988495,-2.929358572351,0.488226428725,-5.724886282468,-3.008361614894,-6.275113717532,0.501393602482,-4.946639414972,6.0,6.899545129872,7.100454870127,4.0,-5.094880460198],[5.937720187245,0.22627988495,2.763340146257,-3.008361614894,0.501393602482,-6.275113717532,-2.92935857235,-5.724886282468,0.488226428725,-5.094880460198,4.0,7.100454870128,6.899545129872,6.0,-4.946639414972],[1.971624368674,-0.734919659396,1.730190387509,-0.801758793188,0.133626465531,-1.535212791594,-1.169865575486,-1.798120541739,0.861644262581,-1.606345304252,0.666666666667,2.140851166377,1.192482166956,2.666666666667,-1.745905116633],[0.52613334565,-0.031299918213,-0.047677857512,0.187076764902,-0.031179460817,-0.000120457396,0.286789889448,0.000120457396,-0.047798314908,-0.125199672852,0,0.000481829584,-0.000481829584,0,-0.190711430048],[0,0.995074130606,0.004925869394,0.015263595581,-0.002543932597,-0.002381936797,-0.015263595581,0.002381936797,0.002543932597,-0.019703477575,0,0.009527747188,-0.009527747188,0,0.019703477575],[-0.078977775725,0.004077096571,0.982759940809,0.004505334253,-0.000750889042,0.004827985613,0.074472441472,-0.004827985613,-0.012412073579,0.016308386282,0,-0.019311942451,0.019311942451,0,-0.068960236766],[-0.835937345812,-0.310552661065,-1.328770229903,1.141861505779,-0.690310250963,-0.620242410102,-0.305924159967,0.620242410102,0.050987359995,0.757789355739,2.0,0.480969640408,-0.480969640408,-2.0,0.684919080386],[0.011423540612,-2.987838138882,-0.010257937683,-0.052045883909,0.008674313985,0.003487547133,0.040622343297,-1.003487547133,-0.006770390549,0.048647444472,0,-0.013950188533,0.013950188533,4.0,-0.041031750731],[-0.485290194962,0.06402234694,-1.644904046101,-0.081189864177,0.013531644029,0.050490702911,0.566480059139,-0.050490702911,-0.59441334319,0.256089387762,0,-0.201962811644,0.201962811644,0,1.420383815597],[1.06574868699,-1.343400432087,2.187691879918,-1.294942520804,0.882490420134,0.440775814446,0.229193833814,-1.107442481113,-0.038198972302,-0.70693506168,-2.666666666667,0.236896742216,0.42976992445,3.333333333333,-0.58256581366],[-0.862099786952,2.322554535321,-0.466237833147,0.320514287597,-0.053419047933,2.375973583254,0.541585499355,1.624026416746,-0.090264249892,1.290218141286,-2.0,-1.503894333017,-0.496105666983,-4.0,0.135048667413],[-0.022847081224,1.995379755339,0.00081239779,0.043037385491,-0.007172897582,0.002552652921,-0.020190304268,1.997447347079,0.003365050711,-0.018480978643,0,-0.010210611684,0.010210611684,-4.0,0.003249591161],[0.331142490383,-0.042783439538,0.764640521268,0.044868646148,-0.007478107691,-0.035305331847,-0.376011136531,0.035305331847,0.729335189422,-0.171133758152,0,0.141221327387,-0.141221327387,0,-0.941437914926]];
function createTerrainHct(points,slopes,edgeGradients=null){
 const a=points[0],b=points[1],c=points[2],ex=b[0]-a[0],ey=b[1]-a[1],fx=c[0]-a[0],fy=c[1]-a[1],det=ex*fy-ey*fx;
 if(Math.abs(det)<1e-10)throw Error('曲面の三角形が退化しています');
 const values=[];for(let i=0;i<3;i++)values.push(points[i][2]-a[2],slopes[i][0]*ex+slopes[i][1]*ey,slopes[i][0]*fx+slopes[i][1]*fy);
 let edgeIndex=0;for(const [i,j] of [[0,1],[1,2],[2,0]]){const dx=points[j][0]-points[i][0],dy=points[j][1]-points[i][1],length2=dx*dx+dy*dy;const mid=edgeGradients?.[edgeIndex++];let gx=mid?mid[0]:(slopes[i][0]+slopes[j][0])/2,gy=mid?mid[1]:(slopes[i][1]+slopes[j][1])/2;const tangent=1.5*(points[j][2]-points[i][2])-.25*((slopes[i][0]+slopes[j][0])*dx+(slopes[i][1]+slopes[j][1])*dy),adjust=(tangent-gx*dx-gy*dy)/length2;gx+=adjust*dx;gy+=adjust*dy;values.push(gx*ex+gy*ey,gx*fx+gy*fy);}
 const coeff=new Float64Array(30);for(let i=0;i<30;i++)for(let j=0;j<15;j++)coeff[i]+=HCT_MATRIX[i][j]*values[j];
 const evaluate=function(u,v){const w=1-u-v,patch=v<=u&&v<=w?0:w<=u&&w<=v?1:2,k=patch*10;const z=coeff[k]+coeff[k+1]*u+coeff[k+2]*v+coeff[k+3]*u*u+coeff[k+4]*u*v+coeff[k+5]*v*v+coeff[k+6]*u*u*u+coeff[k+7]*u*u*v+coeff[k+8]*u*v*v+coeff[k+9]*v*v*v,du=coeff[k+1]+2*coeff[k+3]*u+coeff[k+4]*v+3*coeff[k+6]*u*u+2*coeff[k+7]*u*v+coeff[k+8]*v*v,dv=coeff[k+2]+coeff[k+4]*u+2*coeff[k+5]*v+coeff[k+7]*u*u+2*coeff[k+8]*u*v+3*coeff[k+9]*v*v;return {height:z+a[2],gradient:[(du*fy-dv*ey)/det,(dv*ex-du*fx)/det]};};
 evaluate.coefficients=coeff;
 evaluate.inverse=new Float64Array([fy/det,-ey/det,-fx/det,ex/det]);
 evaluate.planeGradient=new Float64Array([((b[2]-a[2])*fy-(c[2]-a[2])*ey)/det,((c[2]-a[2])*ex-(b[2]-a[2])*fx)/det]);
 return evaluate;
}
// 傾きの場の変化率を一周の隣接点から求める。位置・高さ・保存済みの傾きは変えない。
function fitTerrainSlopeDerivatives(points,indices,slopes,bounds=null){
 const count=points.length/3,neighbors=Array.from({length:count},()=>new Set()),out=new Float64Array(count*4);
 for(let i=0;i<indices.length;i+=3)for(const [a,b] of [[indices[i],indices[i+1]],[indices[i+1],indices[i+2]],[indices[i+2],indices[i]]]){neighbors[a].add(b);neighbors[b].add(a);}
 for(let i=0;i<count;i++){
  const x=points[i*3],y=points[i*3+1];
  // タイルをまたぐ勾配の変化率も一致させる。辺上は両側共通の0を使う。
  if(bounds&&(Math.abs(x-bounds[0])<.01||Math.abs(x-bounds[1])<.01||Math.abs(y-bounds[2])<.01||Math.abs(y-bounds[3])<.01))continue;
  let a=0,b=0,c=0;const rhs=[0,0,0,0];
  for(const j of neighbors[i]){const dx=points[j*3]-x,dy=points[j*3+1]-y,d2=dx*dx+dy*dy;if(d2<1e-12)continue;a+=dx*dx/d2;b+=dx*dy/d2;c+=dy*dy/d2;for(let k=0;k<2;k++){const delta=slopes[j*2+k]-slopes[i*2+k];rhs[k*2]+=dx*delta/d2;rhs[k*2+1]+=dy*delta/d2;}}
  const det=a*c-b*b;if(det<=1e-12*(a+c)**2)continue;
  for(let k=0;k<2;k++){
   let gx=(c*rhs[k*2]-b*rhs[k*2+1])/det,gy=(a*rhs[k*2+1]-b*rhs[k*2])/det,scale=1;
   // 隣接点と逆向きの傾向や、差分の3倍を超す外挿を抑える。
   for(const j of neighbors[i]){const delta=slopes[j*2+k]-slopes[i*2+k],prediction=gx*(points[j*3]-x)+gy*(points[j*3+1]-y);if(Math.abs(prediction)<1e-10)continue;if(prediction*delta<=0){scale=0;break;}scale=Math.min(scale,3*Math.abs(delta/prediction));}
   out[i*4+k*2]=gx*scale;out[i*4+k*2+1]=gy*scale;
  }
 }
 return out;
}
// 辺方向の外挿制約を満たす傾きのうち、元の傾きに最も近いものを選ぶ。
function limitTerrainSlopes(points,indices,slopes,bounds,maxDeviation=25){
 const neighbors=Array.from({length:points.length/3},()=>new Set()),out=new Float32Array(slopes);
 for(let t=0;t<indices.length;t+=3)for(const [a,b] of [[indices[t],indices[t+1]],[indices[t+1],indices[t+2]],[indices[t+2],indices[t]]]){neighbors[a].add(b);neighbors[b].add(a);}
 for(let i=0;i<neighbors.length;i++){
  const x=points[i*3],y=points[i*3+1],gx=slopes[i*2],gy=slopes[i*2+1];
  // 共有辺では両側共通の保存済みヒントを保つ。平面の傾きを0へ潰さない。
  if(bounds&&(Math.abs(x-bounds[0])<.01||Math.abs(x-bounds[1])<.01||Math.abs(y-bounds[2])<.01||Math.abs(y-bounds[3])<.01))continue;
  let low=points[i*3+2],high=low;for(const j of neighbors[i]){low=Math.min(low,points[j*3+2]);high=Math.max(high,points[j*3+2]);}
  const allowance=Math.max(.25,Math.min(maxDeviation,(high-low)*.25)),constraints=[];
  // 曲がった地形では辺の途中で傾きが逆転してもよい。単調性を強制しない。
  for(const j of neighbors[i]){const dx=points[j*3]-x,dy=points[j*3+1]-y,length=Math.hypot(dx,dy);if(length<1e-9)continue;const dz=points[j*3+2]-points[i*3+2],bound=Math.abs(dz)+4*allowance,nx=dx/length,ny=dy/length;constraints.push([nx,ny,(dz+bound)/length],[-nx,-ny,(-dz+bound)/length]);}
  const feasible=(a,b)=>constraints.every(([nx,ny,bound])=>nx*a+ny*b<=bound+1e-10);
  if(feasible(gx,gy))continue;
  let best=[0,0],distance=gx*gx+gy*gy;
  const consider=(a,b)=>{const d=(a-gx)**2+(b-gy)**2;if(d<distance&&feasible(a,b)){best=[a,b];distance=d;}};
  // 二次元の凸領域への最短射影は、辺上への垂線または制約線の交点にある。
  for(let a=0;a<constraints.length;a++){
   const [nx,ny,bound]=constraints[a],delta=bound-nx*gx-ny*gy;consider(gx+delta*nx,gy+delta*ny);
   for(let b=a+1;b<constraints.length;b++){const [mx,my,other]=constraints[b],det=nx*my-ny*mx;if(Math.abs(det)<1e-12)continue;consider((bound*my-ny*other)/det,(nx*other-bound*mx)/det);}
  }
  out[i*2]=best[0];out[i*2+1]=best[1];
 }
 return out;
}
// 共有辺の横断方向の微分を、全三角形の曲げエネルギーが小さくなるよう解く。
// 頂点の高さ・微分と外周の条件は固定する。未知数は辺長を掛けて高さの単位にそろえる。
function fitMinimumBendingEdges(points,indices,slopes,bounds){
 const edges=new Map(),faces=[];
 const key=(a,b)=>a<b?`${a}/${b}`:`${b}/${a}`;
 const point=i=>Array.from(points.subarray(i*3,i*3+3)),gradient=i=>Array.from(slopes.subarray(i*2,i*2+2));
 for(let t=0;t<indices.length;t+=3){const ids=Array.from(indices.subarray(t,t+3)),ee=[];
  for(const [a,b] of [[ids[0],ids[1]],[ids[1],ids[2]],[ids[2],ids[0]]]){const k=key(a,b);let e=edges.get(k);if(!e){const lo=Math.min(a,b),hi=Math.max(a,b),dx=points[hi*3]-points[lo*3],dy=points[hi*3+1]-points[lo*3+1],l=Math.hypot(dx,dy);e={lo,hi,l,nx:-dy/l,ny:dx/l,count:0,id:-1,base:[(slopes[a*2]+slopes[b*2])/2,(slopes[a*2+1]+slopes[b*2+1])/2]};edges.set(k,e);}e.count++;ee.push(e);}faces.push({ids,edges:ee});
 }
 let count=0;for(const e of edges.values())if(e.count===2)e.id=count++;
 const rhs=new Float64Array(count),diag=new Float64Array(count),blocks=[];
 // 各HCT小三角形内の3点積分。2階微分の二乗は2次式なので正確に積分できる。
 const ref=[[0,0],[1,0],[0,1]],center=[1/3,1/3],samples=[];
 for(let k=0;k<3;k++){const a=ref[k],b=ref[(k+1)%3];for(const w of [[2/3,1/6,1/6],[1/6,2/3,1/6],[1/6,1/6,2/3]])samples.push([a[0]*w[0]+b[0]*w[1]+center[0]*w[2],a[1]*w[0]+b[1]*w[1]+center[1]*w[2]]);}
 function hessians(patch){const out=[];const [ux,vx,uy,vy]=patch.inverse,c=patch.coefficients;
  for(const [u,v] of samples){const w=1-u-v,k=(v<=u&&v<=w?0:w<=u&&w<=v?1:2)*10,uu=2*c[k+3]+6*c[k+6]*u+2*c[k+7]*v,uv=c[k+4]+2*c[k+7]*u+2*c[k+8]*v,vv=2*c[k+5]+2*c[k+8]*u+6*c[k+9]*v;out.push(uu*ux*ux+2*uv*ux*vx+vv*vx*vx,Math.SQRT2*(uu*ux*uy+uv*(ux*vy+vx*uy)+vv*vx*vy),uu*uy*uy+2*uv*uy*vy+vv*vy*vy);}return out;
 }
 const dot=(a,b)=>a.reduce((s,v,i)=>s+v*b[i],0);
 for(const f of faces){const p=f.ids.map(point),g=f.ids.map(gradient),mid=f.edges.map(e=>e.base.slice()),base=hessians(createTerrainHct(p,g,mid)),basis=[];
  const area=Math.abs((p[1][0]-p[0][0])*(p[2][1]-p[0][1])-(p[1][1]-p[0][1])*(p[2][0]-p[0][0]))/18;
  for(let j=0;j<3;j++){const e=f.edges[j];if(e.id<0){basis.push(null);continue;}const m=mid.map(v=>v.slice());m[j][0]+=e.nx/e.l;m[j][1]+=e.ny/e.l;basis.push(hessians(createTerrainHct(p,g,m)).map((v,i)=>v-base[i]));}
  const q=new Float64Array(9),ids=f.edges.map(e=>e.id);for(let j=0;j<3;j++)if(ids[j]>=0){rhs[ids[j]]-=area*dot(basis[j],base);for(let k=0;k<3;k++)if(ids[k]>=0)q[j*3+k]=area*dot(basis[j],basis[k]);diag[ids[j]]+=q[j*3+j];}blocks.push({ids,q});
 }
 const apply=x=>{const y=new Float64Array(count);for(const {ids,q} of blocks)for(let j=0;j<3;j++)if(ids[j]>=0)for(let k=0;k<3;k++)if(ids[k]>=0)y[ids[j]]+=q[j*3+k]*x[ids[k]];return y;};
 const x=new Float64Array(count),r=rhs.slice(),z=Float64Array.from(r,(v,i)=>v/Math.max(diag[i],1e-30)),d=z.slice();let rz=dot(r,z),initial=rz,iterations=0;
 for(;iterations<120&&rz>Math.max(initial*1e-12,1e-28);iterations++){const ad=apply(d),den=dot(d,ad);if(!(den>0))break;const alpha=rz/den;for(let i=0;i<count;i++){x[i]+=alpha*d[i];r[i]-=alpha*ad[i];z[i]=r[i]/Math.max(diag[i],1e-30);}const next=dot(r,z),beta=next/rz;for(let i=0;i<count;i++)d[i]=z[i]+beta*d[i];rz=next;}
 // 不収束でも共役勾配の途中解は同じ二次形式を減らす。非有限値だけは採用しない。
 const result=new Map();for(const [k,e] of edges){const delta=e.id<0?0:x[e.id]/e.l;result.set(k,Number.isFinite(delta)?[e.base[0]+delta*e.nx,e.base[1]+delta*e.ny]:e.base);}
 const ax=apply(x),energyDelta=dot(x,ax)+-2*dot(x,rhs);
 return {gradients:result,iterations,energyDelta,residual:initial>0?Math.sqrt(rz/initial):0};
}
// 近傍の平面への適合で、保存済みの傾きとメッシュ形状の食い違いを緩和する。
// XY・外周・局所的な山頂/谷底の高さは固定し、高さの移動は元頂点基準で制限する。
function relaxTerrainVertices(source,indices,sourceSlopes,bounds,maxHeightShift=5){
 const points=new Float32Array(source),slopes=new Float32Array(sourceSlopes),n=points.length/3,neighbors=Array.from({length:n},()=>new Set()),fixed=new Uint8Array(n),ridge=new Uint8Array(n);
 for(let t=0;t<indices.length;t+=3)for(const [a,b] of [[indices[t],indices[t+1]],[indices[t+1],indices[t+2]],[indices[t+2],indices[t]]]){neighbors[a].add(b);neighbors[b].add(a);}
 for(let i=0;i<n;i++){const x=source[i*3],y=source[i*3+1],z=source[i*3+2],nn=[...neighbors[i]];
  if(bounds&&(Math.abs(x-bounds[0])<.01||Math.abs(x-bounds[1])<.01||Math.abs(y-bounds[2])<.01||Math.abs(y-bounds[3])<.01)){fixed[i]=2;continue;}
  if(nn.every(j=>source[j*3+2]<=z)||nn.every(j=>source[j*3+2]>=z)){fixed[i]=1;continue;}
  const range=Math.max(...nn.map(j=>Math.abs(source[j*3+2]-z)),1e-6);
  for(let a=0;a<nn.length&&!ridge[i];a++)for(let b=a+1;b<nn.length;b++){const j=nn[a],k=nn[b],dx=source[j*3]-x,dy=source[j*3+1]-y,ex=source[k*3]-x,ey=source[k*3+1]-y,opposite=(dx*ex+dy*ey)/Math.hypot(dx,dy)/Math.hypot(ex,ey),za=z-source[j*3+2],zb=z-source[k*3+2];if(opposite<-.5&&za*zb>0&&Math.min(Math.abs(za),Math.abs(zb))>range*.15){ridge[i]=1;break;}}
 }
 function solve(m,r){const a=m.map((row,i)=>[...row,r[i]]);for(let c=0;c<3;c++){let pivot=c;for(let k=c+1;k<3;k++)if(Math.abs(a[k][c])>Math.abs(a[pivot][c]))pivot=k;if(Math.abs(a[pivot][c])<1e-12)return null;[a[pivot],a[c]]=[a[c],a[pivot]];const v=a[c][c];for(let j=c;j<4;j++)a[c][j]/=v;for(let k=0;k<3;k++)if(k!==c){const f=a[k][c];for(let j=c;j<4;j++)a[k][j]-=f*a[c][j];}}return a.map(row=>row[3]);}
 for(let pass=0;pass<3;pass++){const nextPoints=points.slice(),nextSlopes=slopes.slice();for(let i=0;i<n;i++){
   if(fixed[i]===2||neighbors[i].size<3)continue;const x=source[i*3],y=source[i*3+1],z=points[i*3+2],scale=Math.max(1,[...neighbors[i]].reduce((s,j)=>s+Math.hypot(source[j*3]-x,source[j*3+1]-y),0)/neighbors[i].size),m=[[2,0,0],[0,0,0],[0,0,0]],rhs=[0,0,0];
   for(const j of neighbors[i]){const dx=(source[j*3]-x)/scale,dy=(source[j*3+1]-y)/scale,w=1/Math.max(.05,dx*dx+dy*dy),v=[1,dx,dy],dz=points[j*3+2]-z;for(let a=0;a<3;a++){rhs[a]+=w*v[a]*dz;for(let b=0;b<3;b++)m[a][b]+=w*v[a]*v[b];}}
   const fit=solve(m,rhs);if(!fit)continue;
   const budget=fixed[i]?0:maxHeightShift*(ridge[i] ? .15 : 1),original=source[i*3+2];nextPoints[i*3+2]=Math.max(original-budget,Math.min(original+budget,z+.5*fit[0]));
   const gx=sourceSlopes[i*2],gy=sourceSlopes[i*2+1],mix=(fixed[i]||ridge[i]) ? .45 : .8;let dx=(fit[1]/scale-gx)*mix,dy=(fit[2]/scale-gy)*mix,cap=.05+.5*Math.hypot(gx,gy),length=Math.hypot(dx,dy);if(length>cap){dx*=cap/length;dy*=cap/length;}nextSlopes[i*2]=gx+dx;nextSlopes[i*2+1]=gy+dy;
  }points.set(nextPoints);slopes.set(nextSlopes);}
 let maxShift=0,moved=0,maxSlopeChange=0;for(let i=0;i<n;i++){const d=Math.abs(points[i*3+2]-source[i*3+2]);if(d>1e-5)moved++;maxShift=Math.max(maxShift,d);maxSlopeChange=Math.max(maxSlopeChange,Math.hypot(slopes[i*2]-sourceSlopes[i*2],slopes[i*2+1]-sourceSlopes[i*2+1]));}
 return {points,slopes,stats:{moved,maxShift,maxSlopeChange,maxHeightShift}};
}
if(typeof module!=='undefined')module.exports={createTerrainHct,fitTerrainSlopeDerivatives,limitTerrainSlopes,fitMinimumBendingEdges,relaxTerrainVertices};

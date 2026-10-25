struct Params { v:u32,e:u32,f:u32,prev_d:u32,d:u32,prev_n:u32,n:u32,face_count:u32 }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> control:array<u32>;
fn triangle(f:u32)->vec3<u32>{return vec3(control[3u*f],control[3u*f+1u],control[3u*f+2u]);}
fn interior(d:u32)->u32{if d<3u{return 0u;}return (d-1u)*(d-2u)/2u;}
fn grid(f:u32,i:u32,j:u32,d:u32)->u32{
 let t=triangle(f);if i==0u&&j==0u{return t.x;}if i==d{return t.y;}if j==d{return t.z;}
 var edge=0u;var a=0u;var b=0u;var step=0u;
 if j==0u {edge=control[3u*p.f+3u*f];a=t.x;b=t.y;step=i;}
 else if i==0u {edge=control[3u*p.f+3u*f+2u];a=t.x;b=t.z;step=j;}
 else if i+j==d {edge=control[3u*p.f+3u*f+1u];a=t.y;b=t.z;step=j;}
 else {let rank=(j-1u)*(d-1u)-j*(j-1u)/2u+i-1u;return p.v+p.e*(d-1u)+f*interior(d)+rank;}
 if a>b {step=d-step;}return p.v+edge*(d-1u)+step-1u;
}
// 頂点番号から、元の制御面と整数の重心座標を復元する。
fn owner(id:u32,d:u32)->vec3<u32>{
 if id<p.v {let f=control[6u*p.f+id];let t=triangle(f);return vec3(f,select(0u,d,t.y==id),select(0u,d,t.z==id));}
 let edge_count=p.e*(d-1u);
 if id<p.v+edge_count {
  let edge=(id-p.v)/(d-1u);let step=(id-p.v)%(d-1u)+1u;let f=control[6u*p.f+p.v+edge];
  let at=6u*p.f+p.v+p.e+2u*edge;let a=control[at];let b=control[at+1u];let t=triangle(f);
  return vec3(f,select(0u,d-step,t.y==a)+select(0u,step,t.y==b),select(0u,d-step,t.z==a)+select(0u,step,t.z==b));
 }
 let count=interior(d);let local=id-p.v-edge_count;let f=local/count;var rank=local%count;var j=1u;
 loop {let width=d-j-1u;if rank<width{break;}rank-=width;j+=1u;}
 return vec3(f,rank+1u,j);
}

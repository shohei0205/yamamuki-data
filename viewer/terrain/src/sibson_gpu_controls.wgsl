// 元Voronoiセルを一度だけ切り、Sibsonと全域C1を同時に評価する。
struct Site { xy:vec2<f32>, z:f32, offset:u32, gradient:vec2<f32>, count:u32, flags:u32 }
struct Node { children:vec4<u32>, data:vec4<u32> }
struct Sample { original:vec4<f32>, adjusted:vec4<f32> }
struct Params { bounds:vec4<f32>, lookup:vec4<f32>, counts:vec4<u32>, settings:vec4<f32> }
@group(0) @binding(0) var<storage,read> sites:array<Site>;
@group(0) @binding(1) var<storage,read> cells:array<vec2<f32>>;
@group(0) @binding(2) var<storage,read> nodes:array<Node>;
@group(0) @binding(3) var<storage,read> refs:array<u32>;
@group(0) @binding(4) var<storage,read_write> output:array<Sample>;
@group(0) @binding(5) var<uniform> params:Params;
struct Area { first:vec2<f32>, previous:vec2<f32>, value:f32, count:u32 }
fn append(state:Area,p:vec2<f32>)->Area {
 var a=state;
 if(a.count==0u){a.first=p;}else if(a.count>=2u){let u=a.previous-a.first;let v=p-a.first;a.value+=u.x*v.y-u.y*v.x;}
 a.previous=p;a.count+=1u;return a;
}
fn clippedArea(site:Site,q:vec2<f32>)->f32 {
 let d=site.xy-q;let c=-dot(d,d)*.5;var a=Area(vec2<f32>(0.),vec2<f32>(0.),0.,0u);
 var s=cells[site.offset+site.count-1u];var ds=dot(d,s)-c;
 for(var j=0u;j<site.count;j+=1u){let e=cells[site.offset+j];let de=dot(d,e)-c;
  if((ds<=0.)!=(de<=0.)){a=append(a,mix(s,e,ds/(ds-de)));}
  if(de<=0.){a=append(a,e);}s=e;ds=de;
 }return abs(a.value)*.5;
}
fn lookup(q:vec2<f32>)->vec2<u32>{
 var lower=params.lookup.xy;var upper=params.lookup.zw;var id=0u;
 for(var depth=0u;depth<24u;depth+=1u){let node=nodes[id];if(node.children.x==0xffffffffu){return node.data.xy;}
  let middle=(lower+upper)*.5;var quadrant=0u;
  if(q.x>=middle.x){quadrant+=1u;lower.x=middle.x;}else{upper.x=middle.x;}
  if(q.y>=middle.y){quadrant+=2u;lower.y=middle.y;}else{upper.y=middle.y;}
  id=node.children[quadrant];
 }return vec2<u32>(0u);
}
fn heightPair(q:vec2<f32>)->vec3<f32>{
 let leaf=lookup(q);if(leaf.y==0u||leaf.y>256u){return vec3<f32>(0.);}
 var sum=0.;var value=0.;var den=0.;var tangent=0.;var alphaNum=0.;var betaNum=0.;var origin=0.;var started=false;
 for(var i=0u;i<leaf.y;i+=1u){let site=sites[refs[leaf.x+i]];let d=site.xy-q;let r2=dot(d,d);
  if(r2<1e-16){return vec3<f32>(site.z,site.z,1.);}
  if(site.count==0u||site.count>128u){return vec3<f32>(0.);}
  let area=clippedArea(site,q);if(area<=0.){continue;}
  if(!started){origin=site.z;started=true;}
  let r=sqrt(r2);let inverse=area/r;sum+=area;value+=area*(site.z-origin);den+=inverse;
  tangent+=inverse*(site.z-origin-dot(site.gradient,d)*params.settings.x);alphaNum+=area*r;betaNum+=area*r2;
 }
 if(sum<=1e-15||den<=0.){return vec3<f32>(0.);}
 let alpha=alphaNum/den;let beta=betaNum/sum;let h0=origin+value/sum;let h1=origin+(alpha*value/sum+beta*tangent/den)/(alpha+beta);
 return vec3<f32>(h0,h1,1.);
}
@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) id:vec3<u32>){
 let n=params.counts.x;if(id.x>n||id.y>n){return;}
 let q=mix(params.bounds.xy,params.bounds.zw,vec2<f32>(id.xy)/f32(n));let d=params.settings.y/params.settings.x;
 let center=heightPair(q);let xp=heightPair(q+vec2<f32>(d,0.));let xm=heightPair(q-vec2<f32>(d,0.));let yp=heightPair(q+vec2<f32>(0.,d));let ym=heightPair(q-vec2<f32>(0.,d));
 let valid=min(center.z,min(min(xp.z,xm.z),min(yp.z,ym.z)));let dx=(xp.xy-xm.xy)/(2.*params.settings.y);let dy=(yp.xy-ym.xy)/(2.*params.settings.y);let index=params.counts.w+id.y*(n+1u)+id.x;
 output[index].original=vec4<f32>(center.x,dx.x,dy.x,valid);output[index].adjusted=vec4<f32>(center.y,dx.y,dy.y,valid);
}

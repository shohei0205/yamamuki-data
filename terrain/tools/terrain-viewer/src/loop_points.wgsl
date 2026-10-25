struct Point { base:vec4<f32>,curved:vec4<f32> }
@group(0) @binding(2) var<storage,read> previous:array<Point>;
@group(0) @binding(3) var<storage,read> indices:array<u32>;
@group(0) @binding(4) var<storage,read> links:array<u32>;
@group(0) @binding(5) var<storage,read_write> result:array<Point>;
fn neighbors(v:u32,f:u32)->vec2<u32>{let a=indices[3u*f];let b=indices[3u*f+1u];let c=indices[3u*f+2u];if a==v{return vec2(b,c);}if b==v{return vec2(c,a);}return vec2(a,b);}
fn edge_face_count(a:u32,b:u32)->u32{var node=links[a];var count=0u;loop{if node==0u{break;}let n=neighbors(a,(node-1u)/3u);if n.x==b||n.y==b{count+=1u;}node=links[p.prev_n+node-1u];}return count;}
fn loop_vertex(v:u32)->vec3<f32>{
 var node=links[v];var count=0u;var boundary=0u;var sum=vec3(0.0);var rim=vec3(0.0);
 loop {if node==0u{break;}let n=neighbors(v,(node-1u)/3u);count+=1u;
  sum+=previous[n.x].curved.xyz+previous[n.y].curved.xyz;
  for(var k=0u;k<2u;k++){let other=n[k];if edge_face_count(v,other)==1u{boundary+=1u;rim+=previous[other].curved.xyz;}}
  node=links[p.prev_n+node-1u];
 }
 let center=previous[v].curved.xyz;
 if boundary==2u{return center*0.75+rim*0.125;}
 if boundary==0u&&count>0u{let n=f32(count);let beta=select(3.0/(8.0*n),3.0/16.0,count==3u);return center*(1.0-n*beta)+sum*(0.5*beta);}
 return center;
}
fn midpoint(a:u32,b:u32)->vec3<f32>{
 var node=links[a];var count=0u;var sum=vec3(0.0);
 loop{if node==0u{break;}let n=neighbors(a,(node-1u)/3u);if n.x==b{sum+=previous[n.y].curved.xyz;count+=1u;}else if n.y==b{sum+=previous[n.x].curved.xyz;count+=1u;}node=links[p.prev_n+node-1u];}
 let ends=previous[a].curved.xyz+previous[b].curved.xyz;if count==2u{return ends*0.375+sum*0.125;}return ends*0.5;
}
@compute @workgroup_size(128) fn cs(@builtin(global_invocation_id) id:vec3<u32>){
 let v=id.x;if v>=p.n{return;}let q=owner(v,p.d);let i=q.y;let j=q.z;let f=q.x;
 if i%2u==0u&&j%2u==0u {let old=grid(f,i/2u,j/2u,p.prev_d);result[v].base=previous[old].base;result[v].curved=vec4(loop_vertex(old),0.0);return;}
 var a=0u;var b=0u;
 if i%2u==1u&&j%2u==0u{a=grid(f,(i-1u)/2u,j/2u,p.prev_d);b=grid(f,(i+1u)/2u,j/2u,p.prev_d);}
 else if i%2u==0u{a=grid(f,i/2u,(j-1u)/2u,p.prev_d);b=grid(f,i/2u,(j+1u)/2u,p.prev_d);}
 else{a=grid(f,(i-1u)/2u,(j+1u)/2u,p.prev_d);b=grid(f,(i+1u)/2u,(j-1u)/2u,p.prev_d);}
 result[v].base=(previous[a].base+previous[b].base)*0.5;result[v].curved=vec4(midpoint(a,b),0.0);
}

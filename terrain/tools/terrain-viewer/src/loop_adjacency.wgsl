struct Params { v:u32,e:u32,f:u32,prev_d:u32,d:u32,prev_n:u32,n:u32,face_count:u32 }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> indices:array<u32>;
// 先頭n要素が頂点ごとの面リスト。残りが面の各角のnext。0は終端。
@group(0) @binding(2) var<storage,read_write> links:array<atomic<u32>>;
@compute @workgroup_size(128) fn cs(@builtin(global_invocation_id) id:vec3<u32>){
 let f=id.x;if f>=p.face_count{return;}
 for(var k=0u;k<3u;k++){let node=3u*f+k;let v=indices[node];let old=atomicExchange(&links[v],node+1u);atomicStore(&links[p.n+node],old);}
}

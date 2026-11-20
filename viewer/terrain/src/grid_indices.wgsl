// 全タイルで共有する格子の三角形インデックスをGPUで生成する。
struct Params{n:u32,pad0:u32,pad1:u32,pad2:u32}
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var<storage,read_write> indices:array<u32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>){
 let n=params.n;let cell=id.x;if cell>=n*n{return;}
 let a=(cell/n)*(n+1u)+cell%n;let at=cell*6u;
 indices[at]=a;indices[at+1u]=a+1u;indices[at+2u]=a+n+2u;indices[at+3u]=a;indices[at+4u]=a+n+2u;indices[at+5u]=a+n+1u;
}

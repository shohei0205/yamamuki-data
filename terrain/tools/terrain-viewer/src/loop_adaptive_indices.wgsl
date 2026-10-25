@group(0) @binding(2) var<storage,read> records:array<vec4<u32>>;
@group(0) @binding(3) var<storage,read_write> result:array<u32>;
@compute @workgroup_size(128) fn cs(@builtin(global_invocation_id) id:vec3<u32>){
 let f=id.x;if f>=p.face_count{return;}let r=records[f];
 for(var k=0u;k<3u;k++){let code=r[k+1u];result[3u*f+k]=grid(r.x,code&15u,code>>4u,p.d);}
}

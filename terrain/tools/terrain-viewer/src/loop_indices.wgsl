@group(0) @binding(2) var<storage,read_write> out_indices:array<u32>;
@compute @workgroup_size(128) fn cs(@builtin(global_invocation_id) id:vec3<u32>){
 let f=id.x;if f>=p.face_count{return;}let face=f/(p.d*p.d);var local=f%(p.d*p.d);var j=0u;
 loop {let width=2u*(p.d-j)-1u;if local<width{break;}local-=width;j+=1u;}
 let i=local/2u;var a=0u;var b=0u;var c=0u;
 if local%2u==0u {a=grid(face,i,j,p.d);b=grid(face,i+1u,j,p.d);c=grid(face,i,j+1u,p.d);}
 else {a=grid(face,i+1u,j,p.d);b=grid(face,i+1u,j+1u,p.d);c=grid(face,i,j+1u,p.d);}
 out_indices[3u*f]=a;out_indices[3u*f+1u]=b;out_indices[3u*f+2u]=c;
}

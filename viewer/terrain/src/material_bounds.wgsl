// 実際に描いた地形位置を32px四方に集約し、各画像の保守的な画面範囲を求める。
struct Config { size:vec4<u32>, commands:vec4<u32> }
@group(0) @binding(0) var positions:texture_2d<f32>;
@group(0) @binding(1) var<storage,read_write> blocks:array<vec4<f32>>;
@group(0) @binding(2) var<uniform> config:Config;
@group(0) @binding(3) var<storage,read> params:array<vec4<f32>>;
@group(0) @binding(4) var<storage,read_write> rectangles:array<vec4<u32>>;
var<workgroup> geographic:array<vec4<f32>,64>;
var<workgroup> screen:array<vec4<u32>,64>;
@compute @workgroup_size(8,8)
fn collect(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>,@builtin(local_invocation_index) id:u32) {
    var lo=vec2(3.402823e38);var hi=vec2(-3.402823e38);
    for(var dy=0u;dy<4u;dy++) {for(var dx=0u;dx<4u;dx++) {
        let pixel=group.xy*32u+local.xy*4u+vec2(dx,dy);
        if all(pixel<config.size.xy) {
            let p=textureLoad(positions,vec2<i32>(pixel),0);
            if p.w>0. {lo=min(lo,p.xy);hi=max(hi,p.xy);}
        }
    }}
    geographic[id]=vec4(lo,hi);workgroupBarrier();
    for(var stride=32u;stride>0u;stride/=2u) {
        if id<stride {let b=geographic[id+stride];let a=geographic[id];geographic[id]=vec4(min(a.xy,b.xy),max(a.zw,b.zw));}
        workgroupBarrier();
    }
    if id==0u {blocks[group.y*config.size.z+group.x]=geographic[0];}
}
@compute @workgroup_size(64)
fn bound(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) id:u32) {
    let area=params[group.x*config.commands.y];
    var lo=config.size.xy;var hi=vec2(0u);
    for(var i=id;i<config.size.z*config.size.w;i+=64u) {
        let b=blocks[i];
        if all(b.zw>=area.xy) && all(b.xy<area.xy+area.zw) {
            let pixel=vec2(i%config.size.z,i/config.size.z)*32u;
            lo=min(lo,pixel);hi=max(hi,min(pixel+32u,config.size.xy));
        }
    }
    screen[id]=vec4(lo,hi);workgroupBarrier();
    for(var stride=32u;stride>0u;stride/=2u) {
        if id<stride {let a=screen[id];let b=screen[id+stride];screen[id]=vec4(min(a.xy,b.xy),max(a.zw,b.zw));}
        workgroupBarrier();
    }
    if id==0u {rectangles[group.x]=screen[0];}
}

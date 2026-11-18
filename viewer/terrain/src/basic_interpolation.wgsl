// 基本描画は先に準備する。高品質補間の準備ができるまではバイリニアで表示する。
fn edge_footprint(image:texture_2d<f32>,uv:vec2<f32>)->f32 {
    let size=vec2<f32>(textureDimensions(image));
    return max(length(dpdx(uv)*size),length(dpdy(uv)*size));
}
struct EdgePair {old_color:vec4<f32>,new_color:vec4<f32>}
fn basic_sample(image:texture_2d<f32>,s:sampler,uv:vec2<f32>,mode:u32)->vec4<f32> {
    if mode==2u {
        let size=textureDimensions(image);
        return textureLoad(image,clamp(vec2<i32>(floor(uv*vec2<f32>(size))),vec2(0),vec2<i32>(size)-vec2(1)),0);
    }
    return textureSampleLevel(image,s,uv,0.);
}
fn edge_resample_pair(old_image:texture_2d<f32>,new_image:texture_2d<f32>,image_sampler:sampler,old_uv:vec2<f32>,new_uv:vec2<f32>,mode:u32,old_footprint:f32,new_footprint:f32,amount:f32)->EdgePair {
    var result=EdgePair(vec4(0.),vec4(0.));
    if amount<1. {result.old_color=basic_sample(old_image,image_sampler,old_uv,mode);}
    if amount>0. {result.new_color=basic_sample(new_image,image_sampler,new_uv,mode);}
    return result;
}

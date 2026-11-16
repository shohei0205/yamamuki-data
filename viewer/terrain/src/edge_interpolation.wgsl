// Scale2x型の近傍判定を、色差と連続的な三角形の重みへ拡張した独自方式。
// 元画像を拡大保存せず、各ピクセルの上下左右から斜めの接続を推定する。
fn edge_distance(a:vec4<f32>,b:vec4<f32>)->f32 {
    return length((a.rgb-b.rgb)*vec3(.65,1.,.5))+abs(a.a-b.a);
}
fn edge_sample(image:texture_2d<f32>, image_sampler:sampler, uv:vec2<f32>, enabled:bool)->vec4<f32> {
    let size=vec2<f32>(textureDimensions(image));
    let footprint=max(length(dpdx(uv)*size),length(dpdy(uv)*size));
    let base=textureSampleLevel(image,image_sampler,uv,0.);
    if !enabled || footprint>=1. {return base;}
    let p=uv*size;
    let center=vec2<i32>(floor(p));
    let maximum=vec2<i32>(textureDimensions(image))-vec2(1);
    let e=textureLoad(image,clamp(center,vec2(0),maximum),0);
    let b=textureLoad(image,clamp(center+vec2(0,-1),vec2(0),maximum),0);
    let d=textureLoad(image,clamp(center+vec2(-1,0),vec2(0),maximum),0);
    let f=textureLoad(image,clamp(center+vec2(1,0),vec2(0),maximum),0);
    let h=textureLoad(image,clamp(center+vec2(0,1),vec2(0),maximum),0);
    let q=fract(p)-vec2(.5);
    let vertical=select(b,h,q.y>=0.);
    let horizontal=select(d,f,q.x>=0.);
    let agreement=edge_distance(vertical,horizontal);
    let separation=min(edge_distance(b,h),edge_distance(d,f));
    let confidence=smoothstep(.015,.12,separation-agreement*2.);
    let corner=(vertical+horizontal)*.5;
    let coverage=smoothstep(-.035,.035,abs(q.x)+abs(q.y)-.5);
    let connected=mix(e,corner,coverage);
    // 1:1近傍では効果を連続的に戻す。写真の微小な色差は補正しない。
    let magnification=1.-smoothstep(.7,1.,footprint);
    return mix(base,connected,confidence*magnification);
}

// 色成分の5×5の流れから細線・面境界を復元し、単調三次補間と滑らかに合成する。
// RGBAへ拡張。提供元や画像の種類による分岐・色の再量子化は行わない。
// Copyright (c) 2021 Advanced Micro Devices, Inc. All rights reserved.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
// 元画素に固定した形状を4つの窓で重ねる。窓の外縁では値と傾きが0になる。
// 元画素中心ごとの方向は周囲5×5の流れと3×3の細部から決まり、画面ピクセルの位置では変えない。
fn edge_luma(c:vec4<f32>)->f32 {
    let rgb=sqrt(max(c.rgb,vec3(0.)));
    return rgb.g+.5*(rgb.r+rgb.b);
}
// 輝度が近い色同士の境界も、RGBと透明度の変化から方向を求める。
fn contour_channels(c:vec4<f32>)->vec4<f32> {
    return vec4(sqrt(max(c.rgb,vec3(0.))),c.a);
}
fn contour_tensor(a:vec4<f32>,b:vec4<f32>,c:vec4<f32>,d:vec4<f32>)->vec3<f32> {
    let aa=contour_channels(a);let bb=contour_channels(b);
    let cc=contour_channels(c);let dd=contour_channels(d);
    return contour_tensor_channels(aa,bb,cc,dd);
}
fn contour_tensor_channels(aa:vec4<f32>,bb:vec4<f32>,cc:vec4<f32>,dd:vec4<f32>)->vec3<f32> {
    let gx=bb-aa+dd-cc;let gy=cc-aa+dd-bb;
    return vec3(dot(gx,gx),dot(gy,gy),dot(gx,gy));
}
fn overlap_window(t:f32)->f32 {
    let a=clamp(1.-abs(t),0.,1.);
    return a*a*(3.-2.*a);
}

// 2画素先の断面を参照し、細線と広い面の境界を区別する。
fn profile_sample(a:vec4<f32>,b:vec4<f32>,c:vec4<f32>,d:vec4<f32>,e:vec4<f32>,f:vec4<f32>,g:vec4<f32>,h:vec4<f32>,i:vec4<f32>,q:vec2<f32>)->vec4<f32> {
    let x=abs(q.x); let y=abs(q.y);
    let top=mix(b,select(a,c,q.x>=0.),x);
    let middle=mix(e,select(d,f,q.x>=0.),x);
    let bottom=mix(h,select(g,i,q.x>=0.),x);
    return mix(middle,select(top,bottom,q.y>=0.),y);
}
fn profile_slope(a:vec4<f32>,b:vec4<f32>)->vec4<f32> {
    let d=a+b;
    let safe=select(vec4(1.),d,abs(d)>vec4(1e-8));
    return select(vec4(0.),2.*a*b/safe,a*b>vec4(0.));
}
// 輪郭と判定できない部分もバイリニアへ戻さず、色ごとの単調三次曲線で復元する。
fn profile_curve(a:vec4<f32>,b:vec4<f32>,c:vec4<f32>,d:vec4<f32>,u:f32)->vec4<f32> {
    let m0=profile_slope(b-a,c-b);let m1=profile_slope(c-b,d-c);
    let u2=u*u;let u3=u2*u;
    return clamp((2.*u3-3.*u2+1.)*b+(u3-2.*u2+u)*m0+(-2.*u3+3.*u2)*c+(u3-u2)*m1,min(b,c),max(b,c));
}
// 色の中間値を横切る位置を拾い、一つの局所境界を二次曲線として復元する。
fn contour_boundary_fit(neighbors:array<vec4<f32>,25>,low:vec3<f32>,axis:vec3<f32>,axis2:f32,normal:vec2<f32>,seed:f32)->vec4<f32> {
    let tangent=vec2(-normal.y,normal.x);
    var projection:array<f32,25>;
    let inverseAxis=1./axis2;
    projection[0]=dot(neighbors[0].rgb-low,axis)*inverseAxis;
    projection[1]=dot(neighbors[1].rgb-low,axis)*inverseAxis;
    projection[2]=dot(neighbors[2].rgb-low,axis)*inverseAxis;
    projection[3]=dot(neighbors[3].rgb-low,axis)*inverseAxis;
    projection[4]=dot(neighbors[4].rgb-low,axis)*inverseAxis;
    projection[5]=dot(neighbors[5].rgb-low,axis)*inverseAxis;
    projection[6]=dot(neighbors[6].rgb-low,axis)*inverseAxis;
    projection[7]=dot(neighbors[7].rgb-low,axis)*inverseAxis;
    projection[8]=dot(neighbors[8].rgb-low,axis)*inverseAxis;
    projection[9]=dot(neighbors[9].rgb-low,axis)*inverseAxis;
    projection[10]=dot(neighbors[10].rgb-low,axis)*inverseAxis;
    projection[11]=dot(neighbors[11].rgb-low,axis)*inverseAxis;
    projection[12]=dot(neighbors[12].rgb-low,axis)*inverseAxis;
    projection[13]=dot(neighbors[13].rgb-low,axis)*inverseAxis;
    projection[14]=dot(neighbors[14].rgb-low,axis)*inverseAxis;
    projection[15]=dot(neighbors[15].rgb-low,axis)*inverseAxis;
    projection[16]=dot(neighbors[16].rgb-low,axis)*inverseAxis;
    projection[17]=dot(neighbors[17].rgb-low,axis)*inverseAxis;
    projection[18]=dot(neighbors[18].rgb-low,axis)*inverseAxis;
    projection[19]=dot(neighbors[19].rgb-low,axis)*inverseAxis;
    projection[20]=dot(neighbors[20].rgb-low,axis)*inverseAxis;
    projection[21]=dot(neighbors[21].rgb-low,axis)*inverseAxis;
    projection[22]=dot(neighbors[22].rgb-low,axis)*inverseAxis;
    projection[23]=dot(neighbors[23].rgb-low,axis)*inverseAxis;
    projection[24]=dot(neighbors[24].rgb-low,axis)*inverseAxis;
    var m0=0.;var m1=0.;var m2=0.;var m3=0.;var m4=0.;var rhs=vec3(0.);
    // 法線の主軸と交差する20辺だけを使う。5×5の支持範囲は維持する。
    let vertical=abs(normal.y)>abs(normal.x);
    var minimum=1e9;var maximum=-1e9;
    for(var k=0u;k<20u;k+=1u) {
        let j=select((k/4u)*5u+k%4u,k,vertical);
        let step=select(1u,5u,vertical);
        let t0=projection[j];let t1=projection[j+step];
        minimum=min(minimum,t1-t0);maximum=max(maximum,t1-t0);
        if (t0-.5)*(t1-.5)>=0. || abs(t1-t0)<.02 {continue;}
        let fraction=clamp((.5-t0)/(t1-t0),0.,1.);
        let q=vec2(f32(j%5u)-2.,f32(j/5u)-2.)+select(vec2(fraction,0.),vec2(0.,fraction),vertical);
        let along=dot(tangent,q);let across=dot(normal,q);
        let w=(1.-smoothstep(.5,1.25,abs(across-seed)))*overlap_window(along/3.);
        let u=along*.5;let u2=u*u;
        m0+=w;m1+=w*u;m2+=w*u2;m3+=w*u2*u;m4+=w*u2*u2;
        rhs+=w*across*vec3(1.,u,u2);
    }
    // 主軸全体で同じ向きに変化する連続階調を段差へ置き換えない。
    if minimum>1e-4 || maximum< -1e-4 {return vec4(0.);}
    if m0<1. {return vec4(0.);}
    let ridge=m0*.001;
    let r0=vec3(m0+ridge,m1,m2);let r1=vec3(m1,m2+ridge,m3);let r2=vec3(m2,m3,m4+m0*.04);
    let det=dot(r0,cross(r1,r2));
    if det<1e-6*m0*m0*m0 {return vec4(0.);}
    let fit=vec3(dot(cross(r1,r2),rhs),dot(cross(r2,r0),rhs),dot(cross(r0,r1),rhs))/det;
    return vec4(clamp(fit,vec3(seed-.5,-1.,-1.),vec3(seed+.5,1.,1.)),1.);
}
struct ContourPatch {color:vec4<f32>,weight:f32,boundary:f32,direction:vec3<f32>}
fn contour_patch(a:vec4<f32>,b:vec4<f32>,c:vec4<f32>,d:vec4<f32>,e:vec4<f32>,f:vec4<f32>,g:vec4<f32>,h:vec4<f32>,i:vec4<f32>,offset:vec2<f32>,flow:vec4<f32>,neighbors:array<vec4<f32>,25>,mode:u32)->ContourPatch {
    let la=edge_luma(a);let lb=edge_luma(b);let lc=edge_luma(c);
    let ld=edge_luma(d);let le=edge_luma(e);let lf=edge_luma(f);
    let lg=edge_luma(g);let lh=edge_luma(h);let li=edge_luma(i);
    let local=contour_tensor(a,b,d,e)+contour_tensor(b,c,e,f)+contour_tensor(d,e,g,h)+contour_tensor(e,f,h,i);
    if local.x+local.y<1e-10 {
        return ContourPatch(e,0.,0.,vec3(0.));
    }
    let tensor=mix(local/max(local.x+local.y,1e-8),flow.xyz/max(flow.x+flow.y,1e-8),.75);
    let xx=tensor.x;let yy=tensor.y;let xy=tensor.z;
    let eigen=sqrt((xx-yy)*(xx-yy)+4.*xy*xy);
    let v=vec2(xx-yy+eigen,2.*xy);
    var normal=vec2(0.,1.);
    if dot(v,v)>1e-12 {normal=v*inverseSqrt(dot(v,v));}
    let coherence=eigen/max(xx+yy,1e-8);
    // この範囲では細線・面境界・曲線復元の重みがすべて0になる。
    if coherence<=.2 {return ContourPatch(e,0.,0.,tensor);}
    let distance=dot(normal,offset);
    // 近傍の少数色の重心と共分散から、線の位置・方向・幅を推定する。
    // 2画素先にも続く面は細線ではないため、断面曲線だけを使う。
    var low=a;var high=a;var lowL=la;var highL=la;
    if lb<lowL {low=b;lowL=lb;}
    if lb>highL {high=b;highL=lb;}
    if lc<lowL {low=c;lowL=lc;}
    if lc>highL {high=c;highL=lc;}
    if ld<lowL {low=d;lowL=ld;}
    if ld>highL {high=d;highL=ld;}
    if lf<lowL {low=f;lowL=lf;}
    if lf>highL {high=f;highL=lf;}
    if lg<lowL {low=g;lowL=lg;}
    if lg>highL {high=g;highL=lg;}
    if lh<lowL {low=h;lowL=lh;}
    if lh>highL {high=h;highL=lh;}
    if li<lowL {low=i;lowL=li;}
    if li>highL {high=i;highL=li;}
    if le<lowL {low=e;lowL=le;}if le>highL {high=e;highL=le;}
    let axis=high.rgb-low.rgb;let axis2=max(dot(axis,axis),1e-8);
    var sumHigh=0.;var residual=0.;
    {
        let q=a.rgb;
        let t=clamp(dot(q-low.rgb,axis)/axis2,0.,1.);
        sumHigh+=t;
        let fit=low.rgb+axis*t;
        residual=max(residual,dot(q-fit,q-fit)/axis2);
    }
    {
        let q=b.rgb;
        let t=clamp(dot(q-low.rgb,axis)/axis2,0.,1.);
        sumHigh+=t;
        let fit=low.rgb+axis*t;
        residual=max(residual,dot(q-fit,q-fit)/axis2);
    }
    {
        let q=c.rgb;
        let t=clamp(dot(q-low.rgb,axis)/axis2,0.,1.);
        sumHigh+=t;
        let fit=low.rgb+axis*t;
        residual=max(residual,dot(q-fit,q-fit)/axis2);
    }
    {
        let q=d.rgb;
        let t=clamp(dot(q-low.rgb,axis)/axis2,0.,1.);
        sumHigh+=t;
        let fit=low.rgb+axis*t;
        residual=max(residual,dot(q-fit,q-fit)/axis2);
    }
    {
        let q=e.rgb;
        let t=clamp(dot(q-low.rgb,axis)/axis2,0.,1.);
        sumHigh+=t;
        let fit=low.rgb+axis*t;
        residual=max(residual,dot(q-fit,q-fit)/axis2);
    }
    {
        let q=f.rgb;
        let t=clamp(dot(q-low.rgb,axis)/axis2,0.,1.);
        sumHigh+=t;
        let fit=low.rgb+axis*t;
        residual=max(residual,dot(q-fit,q-fit)/axis2);
    }
    {
        let q=g.rgb;
        let t=clamp(dot(q-low.rgb,axis)/axis2,0.,1.);
        sumHigh+=t;
        let fit=low.rgb+axis*t;
        residual=max(residual,dot(q-fit,q-fit)/axis2);
    }
    {
        let q=h.rgb;
        let t=clamp(dot(q-low.rgb,axis)/axis2,0.,1.);
        sumHigh+=t;
        let fit=low.rgb+axis*t;
        residual=max(residual,dot(q-fit,q-fit)/axis2);
    }
    {
        let q=i.rgb;
        let t=clamp(dot(q-low.rgb,axis)/axis2,0.,1.);
        sumHigh+=t;
        let fit=low.rgb+axis*t;
        residual=max(residual,dot(q-fit,q-fit)/axis2);
    }
    let bright=sumHigh<4.5;
    var mass=0.;var center=vec2(0.);var moments=vec3(0.);
    let tangent=vec2(-normal.y,normal.x);
    {
        let t=clamp(dot(a.rgb-low.rgb,axis)/axis2,0.,1.);
        let coverage=select(1.-t,t,bright);
        let q=vec2(-1.0,-1.0);
        let at=vec2(dot(normal,q),dot(tangent,q));
        mass+=coverage;center+=coverage*at;
        moments+=coverage*vec3(at.x*at.x,at.y*at.y,at.x*at.y);
    }
    {
        let t=clamp(dot(b.rgb-low.rgb,axis)/axis2,0.,1.);
        let coverage=select(1.-t,t,bright);
        let q=vec2(0.0,-1.0);
        let at=vec2(dot(normal,q),dot(tangent,q));
        mass+=coverage;center+=coverage*at;
        moments+=coverage*vec3(at.x*at.x,at.y*at.y,at.x*at.y);
    }
    {
        let t=clamp(dot(c.rgb-low.rgb,axis)/axis2,0.,1.);
        let coverage=select(1.-t,t,bright);
        let q=vec2(1.0,-1.0);
        let at=vec2(dot(normal,q),dot(tangent,q));
        mass+=coverage;center+=coverage*at;
        moments+=coverage*vec3(at.x*at.x,at.y*at.y,at.x*at.y);
    }
    {
        let t=clamp(dot(d.rgb-low.rgb,axis)/axis2,0.,1.);
        let coverage=select(1.-t,t,bright);
        let q=vec2(-1.0,0.0);
        let at=vec2(dot(normal,q),dot(tangent,q));
        mass+=coverage;center+=coverage*at;
        moments+=coverage*vec3(at.x*at.x,at.y*at.y,at.x*at.y);
    }
    {
        let t=clamp(dot(e.rgb-low.rgb,axis)/axis2,0.,1.);
        let coverage=select(1.-t,t,bright);
        let q=vec2(0.0,0.0);
        let at=vec2(dot(normal,q),dot(tangent,q));
        mass+=coverage;center+=coverage*at;
        moments+=coverage*vec3(at.x*at.x,at.y*at.y,at.x*at.y);
    }
    {
        let t=clamp(dot(f.rgb-low.rgb,axis)/axis2,0.,1.);
        let coverage=select(1.-t,t,bright);
        let q=vec2(1.0,0.0);
        let at=vec2(dot(normal,q),dot(tangent,q));
        mass+=coverage;center+=coverage*at;
        moments+=coverage*vec3(at.x*at.x,at.y*at.y,at.x*at.y);
    }
    {
        let t=clamp(dot(g.rgb-low.rgb,axis)/axis2,0.,1.);
        let coverage=select(1.-t,t,bright);
        let q=vec2(-1.0,1.0);
        let at=vec2(dot(normal,q),dot(tangent,q));
        mass+=coverage;center+=coverage*at;
        moments+=coverage*vec3(at.x*at.x,at.y*at.y,at.x*at.y);
    }
    {
        let t=clamp(dot(h.rgb-low.rgb,axis)/axis2,0.,1.);
        let coverage=select(1.-t,t,bright);
        let q=vec2(0.0,1.0);
        let at=vec2(dot(normal,q),dot(tangent,q));
        mass+=coverage;center+=coverage*at;
        moments+=coverage*vec3(at.x*at.x,at.y*at.y,at.x*at.y);
    }
    {
        let t=clamp(dot(i.rgb-low.rgb,axis)/axis2,0.,1.);
        let coverage=select(1.-t,t,bright);
        let q=vec2(1.0,1.0);
        let at=vec2(dot(normal,q),dot(tangent,q));
        mass+=coverage;center+=coverage*at;
        moments+=coverage*vec3(at.x*at.x,at.y*at.y,at.x*at.y);
    }
    center/=max(mass,1e-6);
    moments=moments/max(mass,1e-6)-vec3(center.x*center.x,center.y*center.y,center.x*center.y);
    let far0=profile_sample(neighbors[0],neighbors[2],neighbors[4],neighbors[10],neighbors[12],neighbors[14],neighbors[20],neighbors[22],neighbors[24],-normal);
    let far1=profile_sample(neighbors[0],neighbors[2],neighbors[4],neighbors[10],neighbors[12],neighbors[14],neighbors[20],neighbors[22],neighbors[24],normal);
    let v0=clamp(dot(far0.rgb-low.rgb,axis)/axis2,0.,1.);
    let v1=clamp(dot(far1.rgb-low.rgb,axis)/axis2,0.,1.);
    let outside=max(select(1.-v0,v0,bright),select(1.-v1,v1,bright));
    let colorConfidence=1.-smoothstep(.01,.08,residual);
    let lineConfidence=(1.-smoothstep(.25,.55,outside))*(1.-smoothstep(.12,.35,max(moments.x,0.)/max(moments.y,.05)));
    let strength=colorConfidence*lineConfidence*smoothstep(.8,1.6,mass)*smoothstep(.2,.75,coherence);
    let slope=clamp(moments.z/max(moments.y,.05),-.5,.5);
    let across=(distance-center.x-slope*(dot(tangent,offset)-center.y))*inverseSqrt(1.+slope*slope);
    let width=clamp(mass/sqrt(12.*max(moments.y,0.)+1.),.35,3.);
    let coverage=1.-smoothstep(width*.5-.25,width*.5+.25,abs(across));
    var lineColor=mix(select(high,low,bright),select(low,high,bright),coverage);
    var lineStrength=strength;
    // 両側の色が異なる面の境界は、輪郭に沿って広く、横切る方向は狭く再構成する。
    // 少なくとも一部に一定色の面がある場合だけ境界の再構成を行い、連続階調を曲げない。
    let opposing=length(far0.rgb-far1.rgb)/sqrt(axis2);
    let boundaryStrength=smoothstep(.45,.9,coherence)*smoothstep(.35,.8,flow.w)*smoothstep(.1,.35,opposing);
    if mode==1u {
        // 既存の細線復元が確かな箇所では、片側の境界で線全体を上書きしない。
        // 外側と内側の変化量を比較し、直線的な階調と実際の段差を区別する。
        let inner0=profile_sample(a,b,c,d,e,f,g,h,i,-normal);
        let inner1=profile_sample(a,b,c,d,e,f,g,h,i,normal);
        let innerJump=abs(dot(inner1.rgb-inner0.rgb,axis))/axis2;
        let outerJump=abs(dot(far1.rgb-far0.rgb,axis))/axis2;
        let stepConfidence=smoothstep(.6,.9,innerJump/max(outerJump,1e-6));
        let coherentStep=stepConfidence*smoothstep(.65,.95,coherence)*(1.-smoothstep(.04,.25,residual))*smoothstep(.04,.12,length(axis));
        let geometric=max(smoothstep(.35,.8,flow.w)*smoothstep(.45,.9,coherence)*colorConfidence,coherentStep)*(1.-strength);
        if geometric>1e-4 {
            let gradient=vec2(dot(f.rgb-d.rgb,axis),dot(h.rgb-b.rgb,axis))/(2.*axis2);
            let derivative=dot(gradient,normal);
            if abs(derivative)>.05 {
                let middle=dot(e.rgb-low.rgb,axis)/axis2;
                let seed=clamp((.5-middle)/derivative,-1.5,1.5);
                let fit=contour_boundary_fit(neighbors,low.rgb,axis,axis2,normal,seed);
                if fit.w>0. {
                    let along=dot(tangent,offset);let u=along*.5;
                    let curve=dot(fit.xyz,vec3(1.,u,u*u));let slope=.5*fit.y+.5*fit.z*along;
                    let distance=(dot(normal,offset)-curve)*inverseSqrt(1.+slope*slope)*sign(derivative);
                    let coverage=smoothstep(-.5,.5,distance);
                    let boundaryColor=mix(low,high,coverage);
                    lineColor=mix(lineColor,boundaryColor,geometric);
                    lineStrength=max(lineStrength,geometric);
                }
            }
        }
    }
    let window=overlap_window(offset.x)*overlap_window(offset.y);
    return ContourPatch(lineColor,lineStrength*window,boundaryStrength*window,tensor);
}
fn edge_footprint(image:texture_2d<f32>,uv:vec2<f32>)->f32 {
    let size=vec2<f32>(textureDimensions(image));
    return max(length(dpdx(uv)*size),length(dpdy(uv)*size));
}
// 新旧LODの重い補間本体を1回だけコンパイルする。参照する画像だけを切り替える。
struct EdgePair {old_color:vec4<f32>,new_color:vec4<f32>}
fn edge_pair_load(a:texture_2d<f32>,b:texture_2d<f32>,layer:u32,p:vec2<i32>)->vec4<f32> {
    if layer==0u {return textureLoad(a,p,0);}
    return textureLoad(b,p,0);
}
fn edge_pair_sample(a:texture_2d<f32>,b:texture_2d<f32>,s:sampler,layer:u32,uv:vec2<f32>)->vec4<f32> {
    if layer==0u {return textureSampleLevel(a,s,uv,0.);}
    return textureSampleLevel(b,s,uv,0.);
}
// Catmull–Romの双三次補間。方向判定は行わず4×4の16画素だけを使う。
fn cubic_curve(a:vec4<f32>,b:vec4<f32>,c:vec4<f32>,d:vec4<f32>,u:f32,mode:u32)->vec4<f32> {
    if mode==5u {
        let u2=u*u;let u3=u2*u;let v=1.-u;
        return (v*v*v*a+(3.*u3-6.*u2+4.)*b+(-3.*u3+3.*u2+3.*u+1.)*c+u3*d)/6.;
    }
    return b+.5*u*(c-a+u*(2.*a-5.*b+4.*c-d+u*(3.*(b-c)+d-a)));
}
fn cubic_pair_sample(a:texture_2d<f32>,b:texture_2d<f32>,layer:u32,uv:vec2<f32>,size:vec2<f32>,maximum:vec2<i32>,mode:u32)->vec4<f32> {
    let p=uv*size-.5;let anchor=vec2<i32>(floor(p));let delta=p-vec2<f32>(anchor);
    let row0=cubic_curve(edge_pair_load(a,b,layer,clamp(anchor+vec2(-1,-1),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(0,-1),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(1,-1),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(2,-1),vec2(0),maximum)),delta.x,mode);
    let row1=cubic_curve(edge_pair_load(a,b,layer,clamp(anchor+vec2(-1,0),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(0,0),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(1,0),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(2,0),vec2(0),maximum)),delta.x,mode);
    let row2=cubic_curve(edge_pair_load(a,b,layer,clamp(anchor+vec2(-1,1),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(0,1),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(1,1),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(2,1),vec2(0),maximum)),delta.x,mode);
    let row3=cubic_curve(edge_pair_load(a,b,layer,clamp(anchor+vec2(-1,2),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(0,2),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(1,2),vec2(0),maximum)),edge_pair_load(a,b,layer,clamp(anchor+vec2(2,2),vec2(0),maximum)),delta.x,mode);
    let color=cubic_curve(row0,row1,row2,row3,delta.y,mode);
    let alpha=clamp(color.a,0.,1.);
    return vec4(clamp(color.rgb,vec3(0.),vec3(alpha)),alpha);
}
fn edge_resample_pair(old_image:texture_2d<f32>,new_image:texture_2d<f32>,image_sampler:sampler,old_uv:vec2<f32>,new_uv:vec2<f32>,mode:u32,old_footprint:f32,new_footprint:f32,amount:f32)->EdgePair {
    var result=EdgePair(vec4(0.),vec4(0.));
    var layer=select(0u,1u,amount>=1.);
    let end=select(2u,1u,amount<=0.);
    loop {
        if layer>=end {break;}
        let dims=select(textureDimensions(old_image),textureDimensions(new_image),layer==1u);
        let size=vec2<f32>(dims);let maximum=vec2<i32>(dims)-vec2(1);
        let uv=select(old_uv,new_uv,layer==1u);
        let footprint=select(old_footprint,new_footprint,layer==1u);
        var sampled=vec4(0.);
        if mode==2u {
            sampled=edge_pair_load(old_image,new_image,layer,clamp(vec2<i32>(floor(uv*size)),vec2(0),maximum));
        } else if mode==0u || footprint>=1. {
            sampled=edge_pair_sample(old_image,new_image,image_sampler,layer,uv);
        } else if mode==4u || mode==5u {
            sampled=cubic_pair_sample(old_image,new_image,layer,uv,size,maximum,mode);
            if footprint>.7 {
                sampled=mix(sampled,edge_pair_sample(old_image,new_image,image_sampler,layer,uv),smoothstep(.7,1.,footprint));
            }
        } else {
    let p=uv*size-.5;
    let anchor=vec2<i32>(floor(p));
    let delta=p-vec2<f32>(anchor);
    var samples:array<vec4<f32>,36>;
    samples[0]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-2,-2),vec2(0),maximum));
    samples[1]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-1,-2),vec2(0),maximum));
    samples[2]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(0,-2),vec2(0),maximum));
    samples[3]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(1,-2),vec2(0),maximum));
    samples[4]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(2,-2),vec2(0),maximum));
    samples[5]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(3,-2),vec2(0),maximum));
    samples[6]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-2,-1),vec2(0),maximum));
    samples[7]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-1,-1),vec2(0),maximum));
    samples[8]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(0,-1),vec2(0),maximum));
    samples[9]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(1,-1),vec2(0),maximum));
    samples[10]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(2,-1),vec2(0),maximum));
    samples[11]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(3,-1),vec2(0),maximum));
    samples[12]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-2,0),vec2(0),maximum));
    samples[13]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-1,0),vec2(0),maximum));
    samples[14]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(0,0),vec2(0),maximum));
    samples[15]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(1,0),vec2(0),maximum));
    samples[16]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(2,0),vec2(0),maximum));
    samples[17]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(3,0),vec2(0),maximum));
    samples[18]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-2,1),vec2(0),maximum));
    samples[19]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-1,1),vec2(0),maximum));
    samples[20]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(0,1),vec2(0),maximum));
    samples[21]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(1,1),vec2(0),maximum));
    samples[22]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(2,1),vec2(0),maximum));
    samples[23]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(3,1),vec2(0),maximum));
    samples[24]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-2,2),vec2(0),maximum));
    samples[25]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-1,2),vec2(0),maximum));
    samples[26]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(0,2),vec2(0),maximum));
    samples[27]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(1,2),vec2(0),maximum));
    samples[28]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(2,2),vec2(0),maximum));
    samples[29]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(3,2),vec2(0),maximum));
    samples[30]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-2,3),vec2(0),maximum));
    samples[31]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(-1,3),vec2(0),maximum));
    samples[32]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(0,3),vec2(0),maximum));
    samples[33]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(1,3),vec2(0),maximum));
    samples[34]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(2,3),vec2(0),maximum));
    samples[35]=edge_pair_load(old_image,new_image,layer,clamp(anchor+vec2(3,3),vec2(0),maximum));
    // 元画素ごとの5×5方向判定を共有する。4パッチの窓は外縁で値と傾きが0になる。
    // 中央4×4が同色なら全パッチの寄与は0で、外周の色によらず一定値になる。
    var spread=vec4(0.);
    spread=max(spread,abs(samples[7]-samples[14]));
    spread=max(spread,abs(samples[8]-samples[14]));
    spread=max(spread,abs(samples[9]-samples[14]));
    spread=max(spread,abs(samples[10]-samples[14]));
    spread=max(spread,abs(samples[13]-samples[14]));
    spread=max(spread,abs(samples[15]-samples[14]));
    spread=max(spread,abs(samples[16]-samples[14]));
    spread=max(spread,abs(samples[19]-samples[14]));
    spread=max(spread,abs(samples[20]-samples[14]));
    spread=max(spread,abs(samples[21]-samples[14]));
    spread=max(spread,abs(samples[22]-samples[14]));
    spread=max(spread,abs(samples[25]-samples[14]));
    spread=max(spread,abs(samples[26]-samples[14]));
    spread=max(spread,abs(samples[27]-samples[14]));
    spread=max(spread,abs(samples[28]-samples[14]));
    if all(spread==vec4(0.)) {sampled=samples[14];} else {
    var cells:array<vec3<f32>,25>;
    // 共有する36画素の平方根を一度だけ求め、25セルへ再利用する。
    let channel0=contour_channels(samples[0]);
    let channel1=contour_channels(samples[1]);
    let channel2=contour_channels(samples[2]);
    let channel3=contour_channels(samples[3]);
    let channel4=contour_channels(samples[4]);
    let channel5=contour_channels(samples[5]);
    let channel6=contour_channels(samples[6]);
    let channel7=contour_channels(samples[7]);
    let channel8=contour_channels(samples[8]);
    let channel9=contour_channels(samples[9]);
    let channel10=contour_channels(samples[10]);
    let channel11=contour_channels(samples[11]);
    let channel12=contour_channels(samples[12]);
    let channel13=contour_channels(samples[13]);
    let channel14=contour_channels(samples[14]);
    let channel15=contour_channels(samples[15]);
    let channel16=contour_channels(samples[16]);
    let channel17=contour_channels(samples[17]);
    let channel18=contour_channels(samples[18]);
    let channel19=contour_channels(samples[19]);
    let channel20=contour_channels(samples[20]);
    let channel21=contour_channels(samples[21]);
    let channel22=contour_channels(samples[22]);
    let channel23=contour_channels(samples[23]);
    let channel24=contour_channels(samples[24]);
    let channel25=contour_channels(samples[25]);
    let channel26=contour_channels(samples[26]);
    let channel27=contour_channels(samples[27]);
    let channel28=contour_channels(samples[28]);
    let channel29=contour_channels(samples[29]);
    let channel30=contour_channels(samples[30]);
    let channel31=contour_channels(samples[31]);
    let channel32=contour_channels(samples[32]);
    let channel33=contour_channels(samples[33]);
    let channel34=contour_channels(samples[34]);
    let channel35=contour_channels(samples[35]);
    cells[0]=contour_tensor_channels(channel0,channel1,channel6,channel7);
    cells[1]=contour_tensor_channels(channel1,channel2,channel7,channel8);
    cells[2]=contour_tensor_channels(channel2,channel3,channel8,channel9);
    cells[3]=contour_tensor_channels(channel3,channel4,channel9,channel10);
    cells[4]=contour_tensor_channels(channel4,channel5,channel10,channel11);
    cells[5]=contour_tensor_channels(channel6,channel7,channel12,channel13);
    cells[6]=contour_tensor_channels(channel7,channel8,channel13,channel14);
    cells[7]=contour_tensor_channels(channel8,channel9,channel14,channel15);
    cells[8]=contour_tensor_channels(channel9,channel10,channel15,channel16);
    cells[9]=contour_tensor_channels(channel10,channel11,channel16,channel17);
    cells[10]=contour_tensor_channels(channel12,channel13,channel18,channel19);
    cells[11]=contour_tensor_channels(channel13,channel14,channel19,channel20);
    cells[12]=contour_tensor_channels(channel14,channel15,channel20,channel21);
    cells[13]=contour_tensor_channels(channel15,channel16,channel21,channel22);
    cells[14]=contour_tensor_channels(channel16,channel17,channel22,channel23);
    cells[15]=contour_tensor_channels(channel18,channel19,channel24,channel25);
    cells[16]=contour_tensor_channels(channel19,channel20,channel25,channel26);
    cells[17]=contour_tensor_channels(channel20,channel21,channel26,channel27);
    cells[18]=contour_tensor_channels(channel21,channel22,channel27,channel28);
    cells[19]=contour_tensor_channels(channel22,channel23,channel28,channel29);
    cells[20]=contour_tensor_channels(channel24,channel25,channel30,channel31);
    cells[21]=contour_tensor_channels(channel25,channel26,channel31,channel32);
    cells[22]=contour_tensor_channels(channel26,channel27,channel32,channel33);
    cells[23]=contour_tensor_channels(channel27,channel28,channel33,channel34);
    cells[24]=contour_tensor_channels(channel28,channel29,channel34,channel35);
    // 実画像の端をクランプしてできた平坦部を、本物の段差と判定しない。
    // 通常のタイル本体は隣接3画素の余白に覆われるため曲線復元が有効。
    let completeSupport=all(anchor>=vec2(2)) && all(anchor+vec2(3)<=maximum);
    let patchMode=select(0u,1u,completeSupport);
    var total=vec4(0.);
    var weight=0.;
    var boundaryWeight=0.;var boundaryTensor=vec3(0.);
    let energyMax0=max(max(max(max(max(max(max(max(max(max(max(max(max(max(max((cells[0].x+cells[0].y),(cells[1].x+cells[1].y)),(cells[2].x+cells[2].y)),(cells[3].x+cells[3].y)),(cells[5].x+cells[5].y)),(cells[6].x+cells[6].y)),(cells[7].x+cells[7].y)),(cells[8].x+cells[8].y)),(cells[10].x+cells[10].y)),(cells[11].x+cells[11].y)),(cells[12].x+cells[12].y)),(cells[13].x+cells[13].y)),(cells[15].x+cells[15].y)),(cells[16].x+cells[16].y)),(cells[17].x+cells[17].y)),(cells[18].x+cells[18].y));
    let energyMin0=min(min(min(min(min(min(min(min(min(min(min(min(min(min(min((cells[0].x+cells[0].y),(cells[1].x+cells[1].y)),(cells[2].x+cells[2].y)),(cells[3].x+cells[3].y)),(cells[5].x+cells[5].y)),(cells[6].x+cells[6].y)),(cells[7].x+cells[7].y)),(cells[8].x+cells[8].y)),(cells[10].x+cells[10].y)),(cells[11].x+cells[11].y)),(cells[12].x+cells[12].y)),(cells[13].x+cells[13].y)),(cells[15].x+cells[15].y)),(cells[16].x+cells[16].y)),(cells[17].x+cells[17].y)),(cells[18].x+cells[18].y));
    let flow0=vec4(cells[0]*1.0+cells[1]*2.0+cells[2]*2.0+cells[3]*1.0+cells[5]*2.0+cells[6]*4.0+cells[7]*4.0+cells[8]*2.0+cells[10]*2.0+cells[11]*4.0+cells[12]*4.0+cells[13]*2.0+cells[15]*1.0+cells[16]*2.0+cells[17]*2.0+cells[18]*1.0,(energyMax0-energyMin0)/max(energyMax0,1e-8)*(1.-smoothstep(1e-8,1e-6,energyMin0)));
    let patch0=contour_patch(samples[7],samples[8],samples[9],samples[13],samples[14],samples[15],samples[19],samples[20],samples[21],delta-vec2(0.0,0.0),flow0,array<vec4<f32>,25>(samples[0],samples[1],samples[2],samples[3],samples[4],samples[6],samples[7],samples[8],samples[9],samples[10],samples[12],samples[13],samples[14],samples[15],samples[16],samples[18],samples[19],samples[20],samples[21],samples[22],samples[24],samples[25],samples[26],samples[27],samples[28]),patchMode);
    total+=patch0.color*patch0.weight;weight+=patch0.weight;
    boundaryWeight+=patch0.boundary;boundaryTensor+=patch0.direction*patch0.boundary;
    let energyMax1=max(max(max(max(max(max(max(max(max(max(max(max(max(max(max((cells[1].x+cells[1].y),(cells[2].x+cells[2].y)),(cells[3].x+cells[3].y)),(cells[4].x+cells[4].y)),(cells[6].x+cells[6].y)),(cells[7].x+cells[7].y)),(cells[8].x+cells[8].y)),(cells[9].x+cells[9].y)),(cells[11].x+cells[11].y)),(cells[12].x+cells[12].y)),(cells[13].x+cells[13].y)),(cells[14].x+cells[14].y)),(cells[16].x+cells[16].y)),(cells[17].x+cells[17].y)),(cells[18].x+cells[18].y)),(cells[19].x+cells[19].y));
    let energyMin1=min(min(min(min(min(min(min(min(min(min(min(min(min(min(min((cells[1].x+cells[1].y),(cells[2].x+cells[2].y)),(cells[3].x+cells[3].y)),(cells[4].x+cells[4].y)),(cells[6].x+cells[6].y)),(cells[7].x+cells[7].y)),(cells[8].x+cells[8].y)),(cells[9].x+cells[9].y)),(cells[11].x+cells[11].y)),(cells[12].x+cells[12].y)),(cells[13].x+cells[13].y)),(cells[14].x+cells[14].y)),(cells[16].x+cells[16].y)),(cells[17].x+cells[17].y)),(cells[18].x+cells[18].y)),(cells[19].x+cells[19].y));
    let flow1=vec4(cells[1]*1.0+cells[2]*2.0+cells[3]*2.0+cells[4]*1.0+cells[6]*2.0+cells[7]*4.0+cells[8]*4.0+cells[9]*2.0+cells[11]*2.0+cells[12]*4.0+cells[13]*4.0+cells[14]*2.0+cells[16]*1.0+cells[17]*2.0+cells[18]*2.0+cells[19]*1.0,(energyMax1-energyMin1)/max(energyMax1,1e-8)*(1.-smoothstep(1e-8,1e-6,energyMin1)));
    let patch1=contour_patch(samples[8],samples[9],samples[10],samples[14],samples[15],samples[16],samples[20],samples[21],samples[22],delta-vec2(1.0,0.0),flow1,array<vec4<f32>,25>(samples[1],samples[2],samples[3],samples[4],samples[5],samples[7],samples[8],samples[9],samples[10],samples[11],samples[13],samples[14],samples[15],samples[16],samples[17],samples[19],samples[20],samples[21],samples[22],samples[23],samples[25],samples[26],samples[27],samples[28],samples[29]),patchMode);
    total+=patch1.color*patch1.weight;weight+=patch1.weight;
    boundaryWeight+=patch1.boundary;boundaryTensor+=patch1.direction*patch1.boundary;
    let energyMax2=max(max(max(max(max(max(max(max(max(max(max(max(max(max(max((cells[5].x+cells[5].y),(cells[6].x+cells[6].y)),(cells[7].x+cells[7].y)),(cells[8].x+cells[8].y)),(cells[10].x+cells[10].y)),(cells[11].x+cells[11].y)),(cells[12].x+cells[12].y)),(cells[13].x+cells[13].y)),(cells[15].x+cells[15].y)),(cells[16].x+cells[16].y)),(cells[17].x+cells[17].y)),(cells[18].x+cells[18].y)),(cells[20].x+cells[20].y)),(cells[21].x+cells[21].y)),(cells[22].x+cells[22].y)),(cells[23].x+cells[23].y));
    let energyMin2=min(min(min(min(min(min(min(min(min(min(min(min(min(min(min((cells[5].x+cells[5].y),(cells[6].x+cells[6].y)),(cells[7].x+cells[7].y)),(cells[8].x+cells[8].y)),(cells[10].x+cells[10].y)),(cells[11].x+cells[11].y)),(cells[12].x+cells[12].y)),(cells[13].x+cells[13].y)),(cells[15].x+cells[15].y)),(cells[16].x+cells[16].y)),(cells[17].x+cells[17].y)),(cells[18].x+cells[18].y)),(cells[20].x+cells[20].y)),(cells[21].x+cells[21].y)),(cells[22].x+cells[22].y)),(cells[23].x+cells[23].y));
    let flow2=vec4(cells[5]*1.0+cells[6]*2.0+cells[7]*2.0+cells[8]*1.0+cells[10]*2.0+cells[11]*4.0+cells[12]*4.0+cells[13]*2.0+cells[15]*2.0+cells[16]*4.0+cells[17]*4.0+cells[18]*2.0+cells[20]*1.0+cells[21]*2.0+cells[22]*2.0+cells[23]*1.0,(energyMax2-energyMin2)/max(energyMax2,1e-8)*(1.-smoothstep(1e-8,1e-6,energyMin2)));
    let patch2=contour_patch(samples[13],samples[14],samples[15],samples[19],samples[20],samples[21],samples[25],samples[26],samples[27],delta-vec2(0.0,1.0),flow2,array<vec4<f32>,25>(samples[6],samples[7],samples[8],samples[9],samples[10],samples[12],samples[13],samples[14],samples[15],samples[16],samples[18],samples[19],samples[20],samples[21],samples[22],samples[24],samples[25],samples[26],samples[27],samples[28],samples[30],samples[31],samples[32],samples[33],samples[34]),patchMode);
    total+=patch2.color*patch2.weight;weight+=patch2.weight;
    boundaryWeight+=patch2.boundary;boundaryTensor+=patch2.direction*patch2.boundary;
    let energyMax3=max(max(max(max(max(max(max(max(max(max(max(max(max(max(max((cells[6].x+cells[6].y),(cells[7].x+cells[7].y)),(cells[8].x+cells[8].y)),(cells[9].x+cells[9].y)),(cells[11].x+cells[11].y)),(cells[12].x+cells[12].y)),(cells[13].x+cells[13].y)),(cells[14].x+cells[14].y)),(cells[16].x+cells[16].y)),(cells[17].x+cells[17].y)),(cells[18].x+cells[18].y)),(cells[19].x+cells[19].y)),(cells[21].x+cells[21].y)),(cells[22].x+cells[22].y)),(cells[23].x+cells[23].y)),(cells[24].x+cells[24].y));
    let energyMin3=min(min(min(min(min(min(min(min(min(min(min(min(min(min(min((cells[6].x+cells[6].y),(cells[7].x+cells[7].y)),(cells[8].x+cells[8].y)),(cells[9].x+cells[9].y)),(cells[11].x+cells[11].y)),(cells[12].x+cells[12].y)),(cells[13].x+cells[13].y)),(cells[14].x+cells[14].y)),(cells[16].x+cells[16].y)),(cells[17].x+cells[17].y)),(cells[18].x+cells[18].y)),(cells[19].x+cells[19].y)),(cells[21].x+cells[21].y)),(cells[22].x+cells[22].y)),(cells[23].x+cells[23].y)),(cells[24].x+cells[24].y));
    let flow3=vec4(cells[6]*1.0+cells[7]*2.0+cells[8]*2.0+cells[9]*1.0+cells[11]*2.0+cells[12]*4.0+cells[13]*4.0+cells[14]*2.0+cells[16]*2.0+cells[17]*4.0+cells[18]*4.0+cells[19]*2.0+cells[21]*1.0+cells[22]*2.0+cells[23]*2.0+cells[24]*1.0,(energyMax3-energyMin3)/max(energyMax3,1e-8)*(1.-smoothstep(1e-8,1e-6,energyMin3)));
    let patch3=contour_patch(samples[14],samples[15],samples[16],samples[20],samples[21],samples[22],samples[26],samples[27],samples[28],delta-vec2(1.0,1.0),flow3,array<vec4<f32>,25>(samples[7],samples[8],samples[9],samples[10],samples[11],samples[13],samples[14],samples[15],samples[16],samples[17],samples[19],samples[20],samples[21],samples[22],samples[23],samples[25],samples[26],samples[27],samples[28],samples[29],samples[31],samples[32],samples[33],samples[34],samples[35]),patchMode);
    total+=patch3.color*patch3.weight;weight+=patch3.weight;
    boundaryWeight+=patch3.boundary;boundaryTensor+=patch3.direction*patch3.boundary;
    let row0=profile_curve(samples[7],samples[8],samples[9],samples[10],delta.x);
    let row1=profile_curve(samples[13],samples[14],samples[15],samples[16],delta.x);
    let row2=profile_curve(samples[19],samples[20],samples[21],samples[22],delta.x);
    let row3=profile_curve(samples[25],samples[26],samples[27],samples[28],delta.x);
    let base=profile_curve(row0,row1,row2,row3,delta.y);
    // 実画像の外端でクランプされた画素を、一定色の面と誤認しない。
    // タイル本体には隣接3画素の余白があるため、通常のタイル境界では有効になる。
    if any(anchor<vec2(2)) || any(anchor+vec2(3)>maximum) {boundaryWeight=0.;}
    var reconstructed=base;
    // 面境界の方向を連続的に合成し、再構成は4パッチで共有する。
    if boundaryWeight>1e-4 {
        let xx=boundaryTensor.x;let yy=boundaryTensor.y;let xy=boundaryTensor.z;
        let eigen=sqrt((xx-yy)*(xx-yy)+4.*xy*xy);
        let v=vec2(xx-yy+eigen,2.*xy);
        var normal=vec2(0.,1.);
        if dot(v,v)>1e-12 {normal=v*inverseSqrt(dot(v,v));}
        let tangent=vec2(-normal.y,normal.x);
        var sum=vec4(0.);var smoothWeights=0.;







        {
            let q=vec2(-1.0,-1.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[7]*w;smoothWeights+=w;
        }
        {
            let q=vec2(0.0,-1.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[8]*w;smoothWeights+=w;
        }
        {
            let q=vec2(1.0,-1.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[9]*w;smoothWeights+=w;
        }
        {
            let q=vec2(2.0,-1.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[10]*w;smoothWeights+=w;
        }


        {
            let q=vec2(-1.0,0.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[13]*w;smoothWeights+=w;
        }
        {
            let q=vec2(0.0,0.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[14]*w;smoothWeights+=w;
        }
        {
            let q=vec2(1.0,0.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[15]*w;smoothWeights+=w;
        }
        {
            let q=vec2(2.0,0.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[16]*w;smoothWeights+=w;
        }


        {
            let q=vec2(-1.0,1.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[19]*w;smoothWeights+=w;
        }
        {
            let q=vec2(0.0,1.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[20]*w;smoothWeights+=w;
        }
        {
            let q=vec2(1.0,1.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[21]*w;smoothWeights+=w;
        }
        {
            let q=vec2(2.0,1.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[22]*w;smoothWeights+=w;
        }


        {
            let q=vec2(-1.0,2.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[25]*w;smoothWeights+=w;
        }
        {
            let q=vec2(0.0,2.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[26]*w;smoothWeights+=w;
        }
        {
            let q=vec2(1.0,2.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[27]*w;smoothWeights+=w;
        }
        {
            let q=vec2(2.0,2.0)-delta;
            let across=dot(normal,q);let along=dot(tangent,q);
            let w=exp2(-3.*across*across-.35*along*along)*overlap_window(q.x/3.)*overlap_window(q.y/3.);
            sum+=samples[28]*w;smoothWeights+=w;
        }







        reconstructed=mix(base,sum/max(smoothWeights,1e-8),clamp(boundaryWeight,0.,1.));
    }
    let color=mix(reconstructed,total/max(weight,1e-8),clamp(weight,0.,1.));
    // 等倍付近の移行以外は、三次補間と固定した線形状の合成結果を使う。
    sampled=color;
    if footprint>.7 {
        sampled=mix(color,edge_pair_sample(old_image,new_image,image_sampler,layer,uv),smoothstep(.7,1.,footprint));
    }

    }

        }
        if layer==0u {result.old_color=sampled;} else {result.new_color=sampled;}
        layer+=1u;
    }
    return result;
}

// 組み込みsinhの微小値での誤差を、地球半径を掛けて増幅させない。
fn earthSinh(x:f32)->f32 {if abs(x)<.01 {return x*(1.+x*x*(1./6.+x*x/120.));}return sinh(x);}
fn earthSin(x:f32)->f32 {if abs(x)<.01 {return x*(1.+x*x*(-1./6.+x*x/120.));}return sin(x);}
fn earthSquare(x:f32)->f32{return x*x;}
// atan2の絶対誤差で拡大時の小さな角度差を失わない。
fn earthAngle(y:f32,x:f32)->f32{
 if x>0. && abs(y)<x*.01 {let a=y/x;return a*(1.+a*a*(-1./3.+a*a/5.));}
 return atan2(y,x);
}
// 球面上の地点を、注視点の東・上・南の座標へ移す。微小な角度差でも精度を保つ。
fn earthPosition(q:vec2<f32>,h:f32,earth:vec4<f32>,scale:f32)->vec3<f32>{
 if earth.w==0. {return vec3(q.x*scale,h,q.y*scale);}
 let r=earth.z;let s=earth.x;let c=earth.y;
 let dy=clamp(q.y/r,-20.,20.);let sh=earthSinh(dy);let cm1=2.*earthSquare(earthSinh(dy*.5));let den=1.+cm1-s*sh;
 let cp=c/den;let sp=(s*(1.+cm1)-sh)/den;
 let db=earthAngle(c*(s*cm1-sh)/den,c*cp+s*sp);let dl=q.x/r;let lm1=2.*earthSquare(earthSin(dl*.5));
 return vec3((r+h)*cp*earthSin(dl),h-(r+h)*(2.*earthSquare(earthSin(db*.5))+c*cp*lm1),-(r+h)*(earthSin(db)+s*cp*lm1));
}

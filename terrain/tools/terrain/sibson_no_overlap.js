
// 領域外データを持たず、端の高さ・傾きから実行時だけ線形外挿する。
function sample(tile,x,y){
 const r=tile.info.fieldBounds,cx=Math.max(r[0],Math.min(r[2],x)),cy=Math.max(r[1],Math.min(r[3],y)),f=sampleRaw(tile,cx,cy);
 return [f[0]+(x-cx)*f[1]+(y-cy)*f[2],f[1],f[2]];
}
function evaluate(owner,x,y,joined=$('join').value==='1'){
 if(!joined||uniformLevel)return sample(owner,x,y);
 const h=evaluateAnalytic(owner,x,y,true)[0],d=1;
 // 外挿した高さと混合重みを含む、同じ曲面の勾配で陰影を計算する。
 return [h,(evaluateAnalytic(owner,x+d,y,true)[0]-evaluateAnalytic(owner,x-d,y,true)[0])/(2*d),(evaluateAnalytic(owner,x,y+d,true)[0]-evaluateAnalytic(owner,x,y-d,true)[0])/(2*d)];
}

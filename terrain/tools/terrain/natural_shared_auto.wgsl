// 同品質は共通曲面を直接評価し、異品質が接する帯だけを接合する。
fn sharedDirect(q:vec2<f32>,t:Tile,old:bool)->vec3<f32>{if(old){return oldSample(q,t);}return evaluate(q,t);}
fn blendFields(q:vec2<f32>,owner:Tile,old:bool)->vec3<f32>{
 if(camera.flags.x==0u||owner.data.w<=1u){return sharedDirect(q,owner,old);}
 // 移行前の場は異なる格子へ保存した表示スナップショット。同品質でも接合する。
 if(old){return mixedBlendFields(q,owner,true);}
 // 同品質だけの近傍はCPUのメタデータで判定済み。頂点ごとの探索を省く。
 if((u32(owner.misc.w)&16u)!=0u){return sharedDirect(q,owner,false);}
 var mixed=false;
 for(var i=0u;i<owner.data.w;i+=1u){let t=tiles[neighbors[owner.data.z+i]];
  if(t.misc.x==owner.misc.x){continue;}
  let band=max(tileBlendBand(owner),tileBlendBand(t));
  if(all(q>=t.core.xy-vec2<f32>(band))&&all(q<=t.core.zw+vec2<f32>(band))){mixed=true;break;}
 }
 if(!mixed){return sharedDirect(q,owner,old);}
 return mixedBlendFields(q,owner,old);
}

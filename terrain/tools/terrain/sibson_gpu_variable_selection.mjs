// 読み込み済みの兄弟を、未読の兄弟のために親へ巻き戻さない。
export function selectCoverage({levels,map,desired,resident,visible,gridN,preferCachedChild=false}){
 const id=t=>[t.error,t.x,t.y].join('/'),wanted=new Map(desired.map(t=>[id(t),t])),split=new Set(),below=new Set(),cached=new Map();
 for(const t of desired)for(let level=0;level<t.level;level++){const ratio=levels[level].side/t.side;split.add([levels[level].error,Math.floor(t.x/ratio),Math.floor(t.y/ratio)].join('/'));}
 for(const v of resident.values()){const level=levels.findIndex(l=>l.error===v.tile.error);for(let l=0;l<level;l++){const ratio=levels[l].side/v.tile.side;below.add([levels[l].error,Math.floor(v.tile.x/ratio),Math.floor(v.tile.y/ratio)].join('/'));}const key=id(v.tile),old=cached.get(key);if(!old||v.tile.n>old.tile.n)cached.set(key,v);}
 function children(level,x,y){if(level+1>=levels.length)return[];const ratio=Math.round(levels[level].side/levels[level+1].side);return Array.from({length:ratio*ratio},(_,i)=>[x*ratio+i%ratio,y*ratio+Math.floor(i/ratio)]);}
 function fragment(owner,t,n){const side=t.bounds[2]-t.bounds[0],sourceSide=owner.tile.bounds[2]-owner.tile.bounds[0];return{...owner,drawBounds:t.bounds,drawN:Math.max(1,Math.min(n,Math.round(owner.tile.n*side/sourceSide)))};}
 function visit(level,x,y,inherited,insideTarget=false,inheritedLimit=Infinity,inheritedStep=0,waitingForTarget=false){const t=map.get([levels[level].error,x,y].join('/'));if(!t||!visible(t))return[];const own=cached.get(id(t)),owner=own||inherited,target=wanted.get(id(t)),active=insideTarget||!!target;if(!active&&!split.has(id(t)))return[];const waiting=target?(!own||own.tile.n<target.n):waitingForTarget;const limit=waiting?Infinity:target&&preferCachedChild?level+1:inheritedLimit;const step=waiting?0:preferCachedChild&&target?(t.bounds[2]-t.bounds[0])/target.n:inheritedStep;const density=n=>step>0?Math.min(n,Math.max(1,Math.round((t.bounds[2]-t.bounds[0])/step))):n;const reuseFine=!!target&&preferCachedChild&&level+1<levels.length&&children(level,x,y).some(([cx,cy])=>cached.has([levels[level+1].error,cx,cy].join('/')));
  if(split.has(id(t))||reuseFine||((waiting||!own)&&below.has(id(t))&&level<limit)){const result=[];for(const [cx,cy] of children(level,x,y))result.push(...visit(level+1,cx,cy,owner,active,limit,step,waiting));
   // 同じ親だけで埋めた範囲は一つにまとめる。部分的に詳細化済みなら分離する。
   if(result.length&&result.every(v=>v.offset===result[0].offset)){
    const source=result[0].tile.bounds;
    // 可視の子が一つだけでも、それを祖先タイル全体へ拡張してはならない。
    // 同じソースが祖先の全領域を実際に保持している場合だけ統合する。
    if(source[0]<=t.bounds[0]&&source[1]<=t.bounds[1]&&source[2]>=t.bounds[2]&&source[3]>=t.bounds[3])return[fragment(result[0],t,density(target?.n||gridN(t)))];
   }return result;
  }
  return owner?[fragment(owner,t,density(target?.n||gridN(t)))]:[];
 }
 return visit(0,0,0,null);
}

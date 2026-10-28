// 読み込み済みの兄弟を、未読の兄弟のために親へ巻き戻さない。
export function selectCoverage({levels,map,desired,resident,visible,gridN,preferCachedChild=false}){
 const id=t=>[t.error,t.x,t.y].join('/'),wanted=new Map(desired.map(t=>[id(t),t])),split=new Set(),below=new Set(),cached=new Map();
 for(const t of desired)for(let level=0;level<t.level;level++){const ratio=2**(t.level-level);split.add([levels[level].error,Math.floor(t.x/ratio),Math.floor(t.y/ratio)].join('/'));}
 for(const v of resident.values()){const level=levels.findIndex(l=>l.error===v.tile.error);for(let l=0;l<level;l++){const ratio=2**(level-l);below.add([levels[l].error,Math.floor(v.tile.x/ratio),Math.floor(v.tile.y/ratio)].join('/'));}const key=id(v.tile),old=cached.get(key);if(!old||v.tile.n>old.tile.n)cached.set(key,v);}
 function fragment(owner,t,n){const side=t.bounds[2]-t.bounds[0],sourceSide=owner.tile.bounds[2]-owner.tile.bounds[0];return{...owner,drawBounds:t.bounds,drawN:Math.max(1,Math.min(n,Math.round(owner.tile.n*side/sourceSide)))};}
 function visit(level,x,y,inherited,insideTarget=false,inheritedLimit=Infinity,inheritedStep=0){const t=map.get([levels[level].error,x,y].join('/'));if(!t||!visible(t))return[];const own=cached.get(id(t)),owner=own||inherited,target=wanted.get(id(t)),active=insideTarget||!!target;if(!active&&!split.has(id(t)))return[];const limit=target&&preferCachedChild?level+1:inheritedLimit;const step=preferCachedChild&&target?(t.bounds[2]-t.bounds[0])/target.n:inheritedStep;const density=n=>step>0?Math.min(n,Math.max(1,Math.round((t.bounds[2]-t.bounds[0])/step))):n;const reuseFine=!!target&&preferCachedChild&&level+1<levels.length&&[0,1,2,3].some(i=>cached.has([levels[level+1].error,x*2+i%2,y*2+Math.floor(i/2)].join('/')));
  if(split.has(id(t))||reuseFine||(!own&&below.has(id(t))&&level<limit)){const result=[];for(let dy=0;dy<2;dy++)for(let dx=0;dx<2;dx++)result.push(...visit(level+1,x*2+dx,y*2+dy,owner,active,limit,step));
   // 同じ親だけで埋めた範囲は一つにまとめる。部分的に詳細化済みなら分離する。
   if(result.length&&result.every(v=>v.offset===result[0].offset))return[fragment(result[0],t,density(target?.n||gridN(t)))];return result;
  }
  return owner?[fragment(owner,t,density(target?.n||gridN(t)))]:[];
 }
 return visit(0,0,0,null);
}

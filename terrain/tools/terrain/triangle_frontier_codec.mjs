// 独自形式TFC1。共有辺15本、頂点LRU12個、予測値と可変長整数だけで復元する。
// meshoptimizerの実装・形式には依存しない。面順と面内の巡回開始点は保存しない。
const MAGIC=0xc1,FULL=240;
function update(ea,eb,vc,state,a,b,c){
 for(let i=14;i>=3;i--){ea[i]=ea[i-3];eb[i]=eb[i-3];}
 ea[0]=b;eb[0]=a;ea[1]=c;eb[1]=b;ea[2]=a;eb[2]=c;
 state.edges=Math.min(15,state.edges+3);
 for(const v of [a,b,c]){let at=0;while(at<state.vertices&&vc[at]!==v)at++;if(at===state.vertices)state.vertices=Math.min(12,state.vertices+1);for(let i=Math.min(at,11);i>0;i--)vc[i]=vc[i-1];vc[0]=v;}
 state.last=c;
}
function token(vc,s,a,b,c){
 for(let i=0;i<s.vertices;i++)if(vc[i]===c)return[i,0];
 if(c===a+1)return[12,0];if(c===b+1)return[13,0];if(c===s.last+1)return[14,0];
 const fromB=Math.abs(c-b)<Math.abs(c-s.last),d=c-(fromB?b:s.last),z=d>=0?d*2:-d*2-1;
 const payload=z*2+(fromB?1:0);return[15,payload];
}
function length(v){return v<128?1:v<16384?2:3;}
export function encodeTriangles(src,reorder=true){
 if(!(src instanceof Uint16Array)||src.length%3)throw Error('uint16三角形一覧が必要です');
 const out=[MAGIC],ea=new Uint16Array(15),eb=new Uint16Array(15),vc=new Uint16Array(12),s={edges:0,vertices:0,last:0};
 const n=src.length/3,visited=new Uint8Array(n),adj=new Map();
 if(reorder)for(let i=0;i<n;i++)for(let j=0;j<3;j++){const a=src[i*3+j],b=src[i*3+(j+1)%3],key=a*65536+b;if(adj.has(key))throw Error('同方向の辺が重複しています');adj.set(key,[i,src[i*3+(j+2)%3]]);}
 const write=v=>{while(v>=128){out.push((v&127)|128);v>>>=7;}out.push(v);};
 let first=0;
 for(let step=0;step<n;step++){
  let chosen=-1,best=Infinity,a,b,c,edge=-1,tag,payload;
  if(reorder){
   for(let e=0;e<s.edges;e++){
    const candidate=adj.get(ea[e]*65536+eb[e]);if(!candidate||visited[candidate[0]])continue;
    const v=candidate[1],[t,p]=token(vc,s,ea[e],eb[e],v);
    let live=0;const f=candidate[0];for(let j=0;j<3;j++){const other=adj.get(src[f*3+(j+1)%3]*65536+src[f*3+j]);if(other&&!visited[other[0]])live++;}
    const score=(1+(t===15?length(p):0))*100+live*.1+e*.001;
    if(score<best){best=score;chosen=f;a=ea[e];b=eb[e];c=v;edge=e;tag=t;payload=p;}
   }
  }
  if(chosen<0){while(first<n&&visited[first])first++;chosen=reorder?first:step;a=src[chosen*3];b=src[chosen*3+1];c=src[chosen*3+2];
   for(let e=0;e<s.edges;e++){const x=ea[e],y=eb[e];let v;if(x===a&&y===b)v=c;else if(x===b&&y===c)v=a;else if(x===c&&y===a)v=b;else continue;const[t,p]=token(vc,s,x,y,v),cost=1+(t===15?length(p):0);if(cost<best){best=cost;edge=e;tag=t;payload=p;}}
   if(edge>=0){const x=ea[edge],y=eb[edge];c=x===a&&y===b?c:x===b&&y===c?a:b;a=x;b=y;}
  }
  visited[chosen]=1;
  if(edge>=0){out.push(edge*16+tag);if(tag===15)write(payload);}
  else{out.push(FULL);let previous=s.last;for(const v of [a,b,c]){const d=v-previous;write(d>=0?d*2:-d*2-1);previous=v;}}
  update(ea,eb,vc,s,a,b,c);
 }
 return new Uint8Array(out);
}
export function decodeTriangles(src,count){
 if(!Number.isInteger(count)||count<0||count%3||count>6000000||src[0]!==MAGIC)throw Error('形式または頂点番号数が不正です');
 const out=new Uint16Array(count),ea=new Uint16Array(15),eb=new Uint16Array(15),vc=new Uint16Array(12),s={edges:0,vertices:0,last:0};let cursor=1;
 const read=()=>{if(cursor>=src.length)throw Error('途中で切れた整数');const a=src[cursor++];if(a<128)return a;if(cursor>=src.length)throw Error('途中で切れた整数');const b=src[cursor++];if(b<128)return(a&127)|(b<<7);if(cursor>=src.length)throw Error('途中で切れた整数');const c=src[cursor++];if(c>=128)throw Error('整数が長すぎます');return(a&127)|((b&127)<<7)|(c<<14);};
 const signed=v=>v&1?-(v+1)/2:v/2;
 for(let i=0;i<count;i+=3){if(cursor>=src.length)throw Error('途中で切れた面');const code=src[cursor++];let a,b,c;
  if(code===FULL){a=s.last+signed(read());b=a+signed(read());c=b+signed(read());}
  else{if(code>FULL)throw Error('未定義の制御値');const e=code>>4,t=code&15;if(e>=s.edges)throw Error('存在しない共有辺');a=ea[e];b=eb[e];if(t<12){if(t>=s.vertices)throw Error('存在しない頂点');c=vc[t];}else if(t===12)c=a+1;else if(t===13)c=b+1;else if(t===14)c=s.last+1;else{const p=read();c=(p&1?b:s.last)+signed(p>>1);}}
  if(a<0||b<0||c<0||a>65535||b>65535||c>65535)throw Error('頂点番号が範囲外です');
  out[i]=a;out[i+1]=b;out[i+2]=c;update(ea,eb,vc,s,a,b,c);
 }
 if(cursor!==src.length)throw Error('余分なデータ');return out;
}

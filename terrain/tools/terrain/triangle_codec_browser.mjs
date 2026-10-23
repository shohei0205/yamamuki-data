import {createTfcDecoder} from './triangle_frontier_wasm.mjs';
import {decodeTriangles} from './triangle_frontier_codec.mjs';
import {MeshoptDecoder} from './meshopt_decoder.mjs';
const button=document.getElementById('browserBench'),result=document.getElementById('browserResult');
function canonical(a){const out=[];for(let i=0;i<a.length;i+=3){const f=[a[i],a[i+1],a[i+2]],k=f.indexOf(Math.min(...f));out.push([f[k],f[(k+1)%3],f[(k+2)%3]].join(','));}return out.sort().join(';');}
const bytes=s=>Uint8Array.from(atob(s),c=>c.charCodeAt(0));
button.onclick=async()=>{
 button.disabled=true;result.textContent='WASM読込・面の一致を確認中…';
 try{
  const region=document.getElementById('region').value,level=document.getElementById('level').value;
  const [raw,source]=await Promise.all([fetch('triangle_frontier_decoder.wasm').then(r=>r.arrayBuffer()),fetch('triangle-codec-browser-input.json').then(r=>r.json()),MeshoptDecoder.ready]);
  const wasm=await createTfcDecoder(raw);
  const rows=source.filter(r=>(region==='all'||r.region===region)&&(level==='all'||r.level===level)).map(r=>({...r,tfc:bytes(r.tfc),meshopt:bytes(r.meshopt)}));
  const official=(b,n)=>{const target=new Uint8Array(n*2);MeshoptDecoder.decodeIndexBuffer(target,n,2,b);return new Uint16Array(target.buffer);};
  for(const r of rows){const a=decodeTriangles(r.tfc,r.count),b=wasm(r.tfc,r.count),c=official(r.meshopt,r.count);if(a.length!==b.length||a.some((v,i)=>v!==b[i])||canonical(a)!==canonical(c))throw Error('復元した面が一致しません');}
  const timings=[];
  for(const[name,decode,key]of [['独自JS',decodeTriangles,'tfc'],['独自WASM',wasm,'tfc'],['公式WASM',official,'meshopt']]){
   for(let j=0;j<5;j++)for(const r of rows)decode(r[key],r.count);
   const samples=[];for(let j=0;j<15;j++){const start=performance.now();for(const r of rows)decode(r[key],r.count);samples.push(performance.now()-start);}samples.sort((a,b)=>a-b);timings.push(`${name} ${samples[7].toFixed(2)} ms`);
  }
  result.textContent=`ブラウザー実測（${region}／${level}）：${timings.join(' ／ ')}。${rows.length}タイルの面一致確認済み。入力コピー・出力確保を含み、ファイル読込・初期化は除外。`;
 }catch(e){result.textContent='計測に失敗：'+e.message;}finally{button.disabled=false;}
};

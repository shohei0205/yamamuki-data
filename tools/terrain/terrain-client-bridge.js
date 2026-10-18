/* 復元用 Worker を地形と追加レイヤーに分け、移動先の表示を待たせない。 */
'use strict';
const terrainClient=(()=>{
 const requests=new Map(),bases=new Map(),workers={};let next=0,timer=null;
 function tick(){updateDataSize();if(!transferStats.pending.size&&timer!==null){clearInterval(timer);timer=null;}}
 function ensureTimer(){if(timer===null)timer=setInterval(tick,250);}
 function cacheKey(url){const params=new URL(url,location.href).searchParams;for(const key of ['stage','format','fullscreen'])params.delete(key);params.sort();return params.toString();}
 function clearChannel(channel,error){
  for(const [id,request] of requests)if(request.channel===channel){request.reject(error);requests.delete(id);}
  for(const key of [...transferStats.pending.keys()])if(String(key).startsWith('sqlite-'+channel+'-'))transferStats.pending.delete(key);
  tick();
 }
 function createWorker(channel){
  const worker=new Worker('/terrain-client-worker.js');workers[channel]=worker;
  worker.onmessage=event=>{
   if(workers[channel]!==worker)return;
   const message=event.data,key='sqlite-'+channel+'-'+message.key;
   if(message.type==='progress'){
    let item=transferStats.pending.get(key);
    if(!item){item={started:performance.now(),received:0,phase:'waiting',compressed:null,label:message.label};transferStats.pending.set(key,item);}
    Object.assign(item,message.progress);ensureTimer();tick();
   }else if(message.type==='received'){
    transferStats.total+=message.bytes;transferStats.pending.delete(key);tick();
   }else if(message.type==='finish-progress'){
    transferStats.pending.delete(key);tick();
   }else{
    const request=requests.get(message.id);if(!request)return;requests.delete(message.id);
    if(message.type==='result'){
     if(channel==='terrain'){bases.set(request.key,message.payload);while(bases.size>2)bases.delete(bases.keys().next().value);}
     request.resolve(message.payload);
    }else request.reject(Error(message.error));
   }
  };
  worker.onerror=error=>{if(workers[channel]===worker){worker.terminate();delete workers[channel];clearChannel(channel,Error(error.message));}};return worker;
 }
 return {load(url){return new Promise((resolve,reject)=>{
  const id=++next,params=new URL(url,location.href).searchParams,channel=params.get('stage')==='full'?'layers':'terrain',key=cacheKey(url);
  // 古くなった追加レイヤーの計算を中断し、最後に表示した地域を優先する。
  if(channel==='layers'&&[...requests.values()].some(request=>request.channel===channel)){
   workers[channel].terminate();delete workers[channel];clearChannel(channel,Error('表示地域が変わったため取得を中断しました'));
  }
  const worker=workers[channel]||createWorker(channel);
  requests.set(id,{resolve,reject,channel,key});worker.postMessage({id,url,base:channel==='layers'?bases.get(key):undefined});
 });}};
})();

// 重い組み立てをUIスレッドから外し、以前の描画を維持する。
if(new URLSearchParams(location.search).get('prep')!=='js'){
 const worker=new Worker('/variable-worker.mjs',{type:'module'}),pending=new Map();let serial=0;
 worker.onmessage=event=>{const value=event.data,job=pending.get(value.id);if(!job)return;pending.delete(value.id);if(value.error)job.reject(Error(value.error));else job.resolve(value);};
 worker.onerror=event=>{for(const job of pending.values())job.reject(Error(event.message||'WASM Workerで失敗しました'));pending.clear();};
 window.VariableMeshWasm={build(tiles,join){return new Promise((resolve,reject)=>{const id=++serial;pending.set(id,{resolve,reject});worker.postMessage({id,join,tiles:tiles.map(t=>({info:t.info,data:t.data}))});});}};
}

// サーバー処理待ち・受信中・失敗時の表示更新を確認する。
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
(async()=>{
 const source=fs.readFileSync('tools/terrain/viewer.html','utf8');
 const text={textContent:''},timers=new Map();let now=0,id=0,respond,controller;
 const data={transferSize:{compressed:100,decoded:200}};
 const context=vm.createContext({data,document:{getElementById:()=>text},location:{href:'http://localhost/terrain'},performance:{now:()=>now},URL,AbortSignal,Uint8Array,Uint32Array,Float32Array,DataView,TextDecoder,Map,Number,JSON,
  fetch:()=>new Promise(resolve=>respond=resolve),
  setInterval:fn=>{timers.set(++id,fn);return id;},clearInterval:id=>timers.delete(id)});
 vm.runInContext(source.slice(source.indexOf('const transferStats='),source.indexOf('async function restoreTerrain'))+'\nglobalThis.api={fetchTerrain,transferStats};',context);
 const request=context.api.fetchTerrain('/terrain-data?z=10&stage=terrain');
 assert.match(text.textContent,/地形 縮尺 10：サーバー処理待ち 0.0 秒/);
 now=2500;for(const tick of timers.values())tick();
 assert.match(text.textContent,/サーバー処理待ち 2.5 秒/);assert.match(text.textContent,/表示データ：100 B/);
 const header=Buffer.from(JSON.stringify({metadata:{polygon:true},arrays:[]}));const padded=Buffer.concat([header,Buffer.alloc((-header.length%4+4)%4,32)]);
 const bytes=Buffer.alloc(8+padded.length);bytes.write('YTB1');bytes.writeUInt32LE(padded.length,4);padded.copy(bytes,8);
 respond(new Response(new ReadableStream({start(c){controller=c;c.enqueue(bytes.subarray(0,8));}}),{headers:{'Content-Length':String(bytes.length)}}));
 await new Promise(resolve=>setImmediate(resolve));
 assert.match(text.textContent,/受信中/);assert.match(text.textContent,/取得中 1 件：8 B/);assert.match(text.textContent,/取得予定/);
 now=5000;for(const tick of timers.values())tick();assert.match(text.textContent,/受信中 5.0 秒/);
 controller.enqueue(bytes.subarray(8));controller.close();await request;
 assert.equal(timers.size,0);assert.equal(context.api.transferStats.pending.size,0);assert.doesNotMatch(text.textContent,/取得中|処理待ち/);
 const failure=context.api.fetchTerrain('/terrain-data?stage=full');
 respond(new Response(JSON.stringify({error:'検証用の失敗'}),{status:500}));await assert.rejects(failure,/検証用の失敗/);
 assert.equal(timers.size,0);assert.match(text.textContent,/表示データ：100 B/);
 console.log('処理待ちの経過時間、受信サイズ、縮尺、取得予定サイズ、完了・失敗時の後片付けを確認しました。');
})().catch(error=>{console.error(error);process.exitCode=1;});

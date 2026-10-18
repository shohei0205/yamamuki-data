/* 選択した DB の必要な部分だけを読み、標高を 16 bit で復号する。 */
import SQLiteFactory from './vendor/wa-sqlite-1.0.0/dist/wa-sqlite.mjs';
import * as SQLite from './vendor/wa-sqlite-1.0.0/src/sqlite-api.js';
import * as VFS from './vendor/wa-sqlite-1.0.0/src/VFS.js';
import JxlFactory from './vendor/icodec-0.6.0/dist/jxl-dec.js';

class SelectedFileVFS extends VFS.Base {
  name = 'selected-file';
  files = new Map();
  handles = new Map();
  reader = new FileReaderSync();
  readBytes = 0;
  readFailure = '';
  xOpen(name, id, flags, out) {
    const file = this.files.get(name);
    if (!file || flags & (VFS.SQLITE_OPEN_READWRITE | VFS.SQLITE_OPEN_CREATE)) return VFS.SQLITE_CANTOPEN;
    this.handles.set(id, file);
    out.setInt32(0, VFS.SQLITE_OPEN_READONLY, true);
    return VFS.SQLITE_OK;
  }
  xRead(id, data, offset) {
    try {
      const bytes = new Uint8Array(this.reader.readAsArrayBuffer(this.handles.get(id).slice(offset, offset + data.length)));
      data.fill(0);
      data.set(bytes);
      this.readBytes += bytes.length;
      return bytes.length === data.length ? VFS.SQLITE_OK : VFS.SQLITE_IOERR_SHORT_READ;
    } catch(e) { this.readFailure=e.name;return VFS.SQLITE_IOERR_READ; }
  }
  xClose(id) { this.handles.delete(id); return VFS.SQLITE_OK; }
  xFileSize(id, out) { out.setBigInt64(0, BigInt(this.handles.get(id).size), true); return VFS.SQLITE_OK; }
  xAccess(name, flags, out) { out.setInt32(0, this.files.has(name) ? 1 : 0, true); return VFS.SQLITE_OK; }
  xDeviceCharacteristics() { return VFS.SQLITE_IOCAP_IMMUTABLE; }
}

globalThis._icodec_ImageData = (data, width, height, depth) => ({data, width, height, depth});
let sqlite, vfs, decoder;
let waterDB;
const connections = [], tileSources = new Map();
async function initialize() {
  if (sqlite) return;
  sqlite = SQLite.Factory(await SQLiteFactory(globalThis.terrainWasm?.sqlite ? {wasmBinary:globalThis.terrainWasm.sqlite} : {}));
  vfs = new SelectedFileVFS();
  sqlite.vfs_register(vfs, false);
  decoder = await JxlFactory(globalThis.terrainWasm?.jxl ? {wasmBinary:globalThis.terrainWasm.jxl} : {});
}
async function openFiles(files) {
  await initialize();
  for (const db of connections) await sqlite.close(db);
  connections.length = 0;
  tileSources.clear();
  vfs.files.clear();
  vfs.readBytes = 0;
  vfs.readFailure = '';
  const tiles = [], levels = new Set();
  let firstConfig,incomplete=false;
  for (const [i, file] of files.entries()) {
    if (file.size < 100) throw Error('SQLite ファイルを選んでください');
    const name = `/height-${i}.sqlite3`;
    vfs.files.set(name, file);
    const db = await sqlite.open_v2(name, SQLite.SQLITE_OPEN_READONLY, vfs.name);
    connections.push(db);
    await sqlite.exec(db, 'PRAGMA query_only=ON; PRAGMA cache_size=-4096; PRAGMA trusted_schema=OFF;');
    let config;
    await sqlite.exec(db, "SELECT value FROM metadata WHERE key='config'", row => { config = JSON.parse(row[0]); });
    if (config?.tileSize !== 256 || config.normals !== false || !config.heightEncoding?.startsWith('16 bit linear gray')) throw Error('全国 JPEG XL ハイトマップの DB を選んでください');
    await sqlite.exec(db,"SELECT value FROM metadata WHERE key='snapshot'",row=>{incomplete ||= JSON.parse(row[0]).incomplete===true;});
    firstConfig ??= config;
    postMessage({progress:`${file.name}：L${config.minLevel}〜L${config.maxLevel} の一覧を読み込み中…`});
    await sqlite.exec(db, 'SELECT z,x,y,height_min,height_scale,length(height_jxl)+length(valid_mask_zstd) FROM tiles ORDER BY z,x,y', row => {
      const [z,x,y,min,scale] = row, key = `${z}/${x}/${y}`;
      if (!Number.isInteger(z) || z < 7 || z > 14 || !Number.isInteger(x) || !Number.isInteger(y) || x<0 || y<0 || x>=2**z || y>=2**z || !Number.isFinite(min) || !Number.isFinite(scale) || scale<=0) throw Error('標高タイルの情報が不正です');
      if (tileSources.has(key)) throw Error('同じタイルを含む DB が重複しています');
      tileSources.set(key, db);
      levels.add(z);
      tiles.push([...row, 0]);
      if(tiles.length%2048===0)postMessage({progress:`${file.name}：${tiles.length.toLocaleString()} タイルを確認中（L${z}）`});
    });
    postMessage({progress: `${file.name}：${tiles.length.toLocaleString()} タイルの一覧を確認`});
  }
  if (!tiles.length) throw Error('標高タイルがありません');
  const sorted = [...levels].sort((a,b)=>a-b);
  if (sorted.some((z,i)=>i && z!==sorted[i-1]+1)) throw Error('レベルが連続する DB を選んでください');
  return {version:1,tileSize:256,incomplete,sourceSHA256:null,config:{...firstConfig,minLevel:sorted[0],maxLevel:sorted.at(-1)},tiles,files:files.map(f=>({name:f.name,bytes:f.size})),readBytes:vfs.readBytes};
}
async function loadTile(key) {
  const db = tileSources.get(key);
  if (!db) throw Error('タイルがありません');
  const [z,x,y] = key.split('/').map(Number);
  let result;
  for await (const stmt of sqlite.statements(db, 'SELECT height_jxl,valid_mask_zstd FROM tiles WHERE z=? AND x=? AND y=?')) {
    sqlite.bind_collection(stmt, [z,x,y]);
    if (await sqlite.step(stmt) === SQLite.SQLITE_ROW) {
      const [jxl,mask] = sqlite.row(stmt), image = decoder.decode(jxl);
      if (typeof image === 'string') throw Error(image);
      if (image.width !== 256 || image.height !== 256 || image.depth !== 16 || image.data.byteLength !== 256*256*8) throw Error('16 bit 標高を復号できませんでした');
      const view = new DataView(image.data.buffer,image.data.byteOffset,image.data.byteLength), values = new Uint16Array(65536);
      for (let i=0;i<values.length;i++) values[i] = view.getUint16(i*8,true);
      result = {values,mask:new Uint8Array(mask),packedBytes:jxl.length+mask.length,readBytes:vfs.readBytes};
    }
  }
  if (!result) throw Error('タイルがありません');
  return result;
}
async function openWater(file) {
  await initialize();
  if(waterDB) await sqlite.close(waterDB);
  vfs.files.clear();
  vfs.files.set('/water.sqlite3',file);
  waterDB=await sqlite.open_v2('/water.sqlite3',SQLite.SQLITE_OPEN_READONLY,vfs.name);
  await sqlite.exec(waterDB,'PRAGMA query_only=ON; PRAGMA cache_size=-4096; PRAGMA trusted_schema=OFF;');
  const layers=[];
  await sqlite.exec(waterDB,'SELECT DISTINCT layer FROM tiles',row=>layers.push(row[0]));
  if(!layers.length||layers.some(layer=>!['rivers','water','sea'].includes(layer)))throw Error('海岸・河川・湖沼のベクター SQLite を選んでください');
  return {layers};
}
async function loadWater(request) {
  if(!waterDB)throw Error('水域 SQLite を選んでください');
  const {z,x0,x1,y0,y1}=request;
  if(![10,11,12].includes(z)||![x0,x1,y0,y1].every(Number.isInteger)||x0<0||y0<0||x1<x0||y1<y0||x1>=2**z||y1>=2**z||(x1-x0+1)*(y1-y0+1)>65536)throw Error('水域の範囲が不正です');
  const descriptors=[],bodies=[];
  for(const [layer,level]of [['rivers',request.riverZ||z],['water',request.riverZ||z],['sea',request.seaZ||z]]){
    if(![10,11,12].includes(level))throw Error('水域のレベルが不正です');
    const ratio=2**(level-z),bounds=[Math.floor(x0*ratio),Math.floor((x1+1)*ratio-1e-8),Math.floor(y0*ratio),Math.floor((y1+1)*ratio-1e-8)];
    for await(const stmt of sqlite.statements(waterDB,'SELECT x,y,data FROM tiles WHERE layer=? AND z=? AND x BETWEEN ? AND ? AND y BETWEEN ? AND ? ORDER BY x,y')){
      sqlite.bind_collection(stmt,[layer,level,...bounds]);
      while(await sqlite.step(stmt)===SQLite.SQLITE_ROW){const [x,y,data]=sqlite.row(stmt),body=new Uint8Array(data);descriptors.push({layer,z:level,x,y,bytes:body.length});bodies.push(body);}
    }
  }
  const header=new TextEncoder().encode(JSON.stringify(descriptors)),packet=new Uint8Array(8+header.length+bodies.reduce((n,b)=>n+b.length,0)),view=new DataView(packet.buffer);
  view.setUint32(0,0x31515359,true);view.setUint32(4,header.length,true);packet.set(header,8);let offset=8+header.length;
  for(const body of bodies){packet.set(body,offset);offset+=body.length;}
  return {packet};
}
// 同じ SQLite 接続への問い合わせとデコーダーの利用を直列にする。
let queue = Promise.resolve();
onmessage = ({data}) => {
  queue = queue.then(async () => {
    try {
      const result = data.type==='open' ? await openFiles(data.files) : data.type==='water-open' ? await openWater(data.file) : data.type==='water' ? await loadWater(data.request) : await loadTile(data.key);
      const transfer = result.values ? [result.values.buffer,result.mask.buffer] : result.packet ? [result.packet.buffer] : [];
      postMessage({id:data.id,result},transfer);
    } catch(e) { postMessage({id:data.id,error:vfs?.readFailure?'ファイルを読み出せません。生成中・更新中の DB は使えないため、完成済み DB または閲覧用コピーを選び直してください。':e.message}); }
  });
};

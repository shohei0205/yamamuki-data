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
    } catch { return VFS.SQLITE_IOERR_READ; }
  }
  xClose(id) { this.handles.delete(id); return VFS.SQLITE_OK; }
  xFileSize(id, out) { out.setBigInt64(0, BigInt(this.handles.get(id).size), true); return VFS.SQLITE_OK; }
  xAccess(name, flags, out) { out.setInt32(0, this.files.has(name) ? 1 : 0, true); return VFS.SQLITE_OK; }
  xDeviceCharacteristics() { return VFS.SQLITE_IOCAP_IMMUTABLE; }
}

globalThis._icodec_ImageData = (data, width, height, depth) => ({data, width, height, depth});
let sqlite, vfs, decoder;
const connections = [], tileSources = new Map();
async function initialize() {
  if (sqlite) return;
  sqlite = SQLite.Factory(await SQLiteFactory());
  vfs = new SelectedFileVFS();
  sqlite.vfs_register(vfs, false);
  decoder = await JxlFactory();
}
async function openFiles(files) {
  await initialize();
  for (const db of connections) await sqlite.close(db);
  connections.length = 0;
  tileSources.clear();
  vfs.files.clear();
  vfs.readBytes = 0;
  const tiles = [], levels = new Set();
  let firstConfig;
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
    firstConfig ??= config;
    await sqlite.exec(db, 'SELECT z,x,y,height_min,height_scale,length(height_jxl)+length(valid_mask_zstd) FROM tiles ORDER BY z,x,y', row => {
      const [z,x,y,min,scale] = row, key = `${z}/${x}/${y}`;
      if (!Number.isInteger(z) || z < 7 || z > 14 || !Number.isInteger(x) || !Number.isInteger(y) || x<0 || y<0 || x>=2**z || y>=2**z || !Number.isFinite(min) || !Number.isFinite(scale) || scale<=0) throw Error('標高タイルの情報が不正です');
      if (tileSources.has(key)) throw Error('同じタイルを含む DB が重複しています');
      tileSources.set(key, db);
      levels.add(z);
      tiles.push([...row, 0]);
    });
    postMessage({progress: `${file.name}：${tiles.length.toLocaleString()} タイルの一覧を確認`});
  }
  if (!tiles.length) throw Error('標高タイルがありません');
  const sorted = [...levels].sort((a,b)=>a-b);
  if (sorted.some((z,i)=>i && z!==sorted[i-1]+1)) throw Error('レベルが連続する DB を選んでください');
  return {version:1,tileSize:256,sourceSHA256:null,config:{...firstConfig,minLevel:sorted[0],maxLevel:sorted.at(-1)},tiles,files:files.map(f=>({name:f.name,bytes:f.size})),readBytes:vfs.readBytes};
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
// 同じ SQLite 接続への問い合わせとデコーダーの利用を直列にする。
let queue = Promise.resolve();
onmessage = ({data}) => {
  queue = queue.then(async () => {
    try {
      const result = data.type==='open' ? await openFiles(data.files) : await loadTile(data.key);
      const transfer = result.values ? [result.values.buffer,result.mask.buffer] : [];
      postMessage({id:data.id,result},transfer);
    } catch(e) { postMessage({id:data.id,error:e.message}); }
  });
};

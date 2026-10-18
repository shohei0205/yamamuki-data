/* ファイルはアップロードせず、専用 Worker へ渡す。 */
window.SQLiteHeightSource = class {
  constructor(progress) {
    this.worker = new Worker('/sqlite-height-worker.js',{type:'module'});
    this.requests = new Map(); this.serial = 0;
    this.worker.onmessage = ({data}) => {
      if (data.progress) { progress(data.progress); return; }
      const request = this.requests.get(data.id);
      if (!request) return;
      this.requests.delete(data.id);
      if (data.error) request.reject(Error(data.error)); else request.resolve(data.result);
    };
    this.worker.onerror = () => { this.close(); progress('ブラウザー内の標高処理を開始できませんでした'); };
  }
  request(type, payload) {
    return new Promise((resolve,reject) => {
      const id=++this.serial; this.requests.set(id,{resolve,reject});
      this.worker.postMessage({id,type,...payload});
    });
  }
  open(files) { return this.request('open',{files}); }
  tile(key) { return this.request('tile',{key}); }
  close() {
    this.worker.terminate();
    for (const request of this.requests.values()) request.reject(Error('標高ファイルを閉じました'));
    this.requests.clear();
  }
};

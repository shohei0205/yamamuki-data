/* ファイルはアップロードせず、専用 Worker へ渡す。 */
window.SQLiteHeightSource = class {
  constructor(progress) {
    if(window.terrainStandaloneWorker){const url=URL.createObjectURL(new Blob([window.terrainStandaloneWorker],{type:'text/javascript'}));this.worker=new Worker(url);this.workerURL=url;}else this.worker = new Worker('/sqlite-height-worker.js',{type:'module'});
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
  openWater(file) { return this.request('water-open',{file}); }
  water(request) { return this.request('water',{request}); }
  close() {
    this.worker.terminate();
    if(this.workerURL)URL.revokeObjectURL(this.workerURL);
    for (const request of this.requests.values()) request.reject(Error('標高ファイルを閉じました'));
    this.requests.clear();
  }
};

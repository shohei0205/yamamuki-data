# ブラウザーで標高 SQLite を読むための依存物

- [icodec 0.6.0](https://github.com/Kaciras/icodec): npm の `icodec-0.6.0.tgz` から JPEG XL デコーダーの JS と WASM のみを同梱。MIT。内部の libjxl は 0.11.0。libjxl・Brotli・Highway のライセンスも同梱する。
- [wa-sqlite 1.0.0](https://github.com/rhashimoto/wa-sqlite): npm の `wa-sqlite-1.0.0.tgz` から同期版 JS/WASM と API/VFS のソースを同梱。MIT。SQLite は public domain。

JS の改行を LF に統一し、同一オリジンから読み込む。選択ファイルを送信しない。Worker の FileReaderSync による部分読み出しと、整数 16 bit の JPEG XL 復号に使う。配布用地形データや法線マップは含まない。

## SHA-256

- `icodec-0.6.0/brotli-LICENSE`: `3d180008e36922a4e8daec11c34c7af264fed5962d07924aea928c38e8663c94`
- `icodec-0.6.0/dist/jxl-dec.js`: `7a3111bc23e2b60c51e04b75ef65747bdf26763333a03e162a5c36859e0253be`
- `icodec-0.6.0/dist/jxl-dec.wasm`: `26872532f9552cd4c16395b15eceddf88aa7ab5b18357841d322aa43793df24e`
- `icodec-0.6.0/highway-LICENSE`: `43070e2d4e532684de521b885f385d0841030efa2b1a20bafb76133a5e1379c1`
- `icodec-0.6.0/libjxl-LICENSE`: `8405932022a556380c2d8c272eff154a923feb197233f348ce5f7334fb0a5ede`
- `icodec-0.6.0/libjxl-PATENTS`: `91915f8ae056a68a3c5bdf05d9f6f78bb6903e27a8ca3a8434c9e4ac87300575`
- `icodec-0.6.0/LICENSE`: `82caa1908f92456c29f3f757374bf5020e17c82ed9495c7969c7f3a651b6493b`
- `wa-sqlite-1.0.0/dist/wa-sqlite.mjs`: `3a3dfad37c128c2079ce88afff3f1cf461516868162c33a5adb6ad6c2d34fb87`
- `wa-sqlite-1.0.0/dist/wa-sqlite.wasm`: `5384bc7d80d7981c2516f3ad6b02d886629985d178e5037e765d655f440bbf9f`
- `wa-sqlite-1.0.0/LICENSE`: `547f9e97e461bd369cc64b6fc12d724e8b02764b8db7812691f29f0ec572be79`
- `wa-sqlite-1.0.0/src/sqlite-api.js`: `2ffbbcf50c6ef3a0e1e38523c58e68e37bfe622819630dc33bfee131e0d00d4a`
- `wa-sqlite-1.0.0/src/sqlite-constants.js`: `b9c7bac079cbf3a9e7da9e3e2adf56743fc1255f418193b287c5381d56ad8019`
- `wa-sqlite-1.0.0/src/VFS.js`: `0a59381f231c933abdb9db223ad7d5c27d3757e7030dc44da4853e614f49e3d2`

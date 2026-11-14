# 地形のSQLite形式 版1

[地形データ](../README.md) / [共通SQLiteコンテナ](../../formats/sqlite.md)

この文書は `kind='terrain'`、`format_version=1` の読み取り契約。コンテナの `user_version=1` とデータセット形式版、データの配布識別子 `data_version` は別物である。地表と同居できる。未知のデータセット形式版・encodingは推測で復号しない。

## テーブル

```sql
SELECT dataset_key,id,title,data_version,metadata_json
FROM datasets WHERE kind='terrain' AND format_version=1;
SELECT level_key,z,settings_json FROM terrain_levels
WHERE dataset_key=:dataset ORDER BY z;
SELECT x,y,encoding,payload FROM terrain_tiles
WHERE dataset_key=:dataset AND level_key=:level;
```

`level_key` はDB内のキー。XYZズームは `terrain_levels.z` を使う。同じデータセットで同一zを重複定義しない。`terrain_tiles` の主キーは `(dataset_key,level_key,x,y)`。x・yは0以上 `2^z` 未満。タイルが存在しない領域を、標高0の新規タイルとして合成しない。

`settings_json` の `errorDownMetres`・`errorUpMetres` は元DEMに対する下・上の誤差許容量。`interpolation='sibson'`、`xyStepMetres=0.0625`、`heightStepMetres=0.125`、`maxParentDepth=2`、`maxDependencies=16` を記録する。`anchor` は生成側の独立化方針であり、復号器は各BLOBの実際の親参照を優先する。

## 座標と整数

XYZの北西が原点、Xは東、Yは南へ増加する。投影幅 `W=40075016.68557849` m。EPSG:3857の東距・北距そのものではなく、`X=easting+W/2`、`Y=W/2-northing` とした正の世界座標を使う。辺長は `W/2^z`、範囲は `[x*辺長,y*辺長,(x+1)*辺長,(y+1)*辺長]`。

正規XYZは3個の符号付き32bit整数。X・Yを16で割り、Zを8で割るとm単位となる。**固定16bit座標ではない**。局所原点と差分を整数符号化することで容量を削減する。タイル外の制御点は自然近傍の正しい支持領域として必要なので、タイルの四角形で切り捨てない。

全球のX・Yをfloat32に変換してから原点を引くと精度を失う。整数をfloat64でmへ戻してタイル原点を引き、その後にGPU用float32へ変換する。

## 外側BLOB：NTS1

`encoding='natural-controls-v1'`。全整数・floatのバイト順はlittle endian。圧縮ファイル全体のSHA-256はmanifest、復元点のSHA-256はこのヘッダーに記録する。

| オフセット | 型 | 内容 |
|---:|---|---|
| 0 | 4 bytes | ASCII `NTS1` |
| 4 | i32 | 制御点本文の局所原点X（1/16m） |
| 8 | i32 | 制御点本文の局所原点Y（1/16m） |
| 12 | u32 | 復元点数N（1〜262144） |
| 16 | u32 | 直接親タイル数P（0〜4） |
| 20 | 32 bytes | 全点XYZのSHA-256（生の32byte） |
| 52 | P×12 bytes | 親の `(z,x,y)`、各u32。格納順がslot番号 |
| 52+12P | u32 | 続く制御点本文のバイト長L |
| 56+12P | L bytes | NQC1またはNDC1 |

Lは24〜16777216。余分な末尾は認めない。親は同一データセットのXYZで参照する。別データセットの整数キーをBLOBに含めないので、コンテナ統合時にも書き換え不要。すべての親は自身より粗いzであり、最大2本の辺をたどるまでに独立タイルへ到達する。推移的参照のタイル数は自身を含め最大16。循環、欠損親、上限超過はエラー。

SHA-256は復元した順序の各点X・Y・Zをi32 little endianで連結して計算する。点を勝手にソートすると検査値と親のindex参照が変わる。

## 制御点本文

先頭24byteは4byte magic、u32のN、u32のP、u32の新規点数F、f32のXY量子化幅（必ず0.0625）、f32のZ量子化幅（必ず0.125）。NとPは外側と一致させる。続いて列ごとに `u32バイト長 + 整数ストリーム` を置き、末尾まで読み切る。

### NQC1：独立

P=0、F=N、列はX・Y・Zの3本。各列N個の符号付き整数で、先頭0から累積和を取る。X・Yには外側の整数原点を加える。Zには原点を加えない。これが正規XYZになる。

### NDC1：差分

7列をこの順に保存する。

| 列 | 個数 | 内容 |
|---|---:|---|
| 0 | N | 点の種別。0=継承、1=新規 |
| 1 | N−F | 継承点の親slot |
| 2 | N−F | 親slotごとの直前indexとの差分。初期値0 |
| 3 | N−F | 親Zに加える整数差分 |
| 4 | F | 新規Xの直前新規点との差分 |
| 5 | F | 新規Yの直前新規点との差分 |
| 6 | F | 新規Zの直前新規点との差分 |

列0の順に点を復元する。継承点のX・Yは親点の正規整数をそのまま使い、Zに列3を加える。新規点は独立本文と同様に新規点だけで累積和を取り、X・Yに子の原点を加える。親indexは0以上、親点数未満、かつ2^24未満。計算はi64でoverflowを検査してからi32へ変換する。

### 整数ストリーム

最初に個数をunsigned LEB128で記録する。最大128個ごとに `LEB128のブロック長 + ブロック本文` を保存する。LEB128は1byte下位7bitを順に連結し、上位bitが1なら続く。最大10byte、最後は0か1。各ブロックの先頭1byteが方式を指定する。

| 方式 | 本文 |
|---:|---|
| 0（FOR） | ZigZag基準値のLEB128、幅u8、各値と基準値の差を固定幅で格納。幅0〜63 |
| 1（ゼロ連続） | 0なら続くLEB128個の0を出力。非0のLEB128値vならZigZag逆変換(v−1)を1個出力 |
| 2（Rice） | kのu8（0〜32）、有効bit数のLEB128、bit本文。各ZigZag整数を商の0連続＋1とk bitの剰余で復元 |

ZigZag逆変換は `(v>>1) ^ -(v&1)`。bit本文はbyte内も整数内も最下位bitから読む。bit数を超えて読まない。固定幅の末尾paddingは値として数えず、Riceでは有効bitをちょうど読み切る。ブロックごとの個数は残り個数と128の小さい方であり、別の個数ヘッダーはない。未知の方式・過大な個数・ゼロ長run・overflow・余分なbyteは拒否する。

## 復号と描画

1. コンテナのapplication_id・user_version、データセット形式版、収録レベルとencodingを検査する。
2. NTS1を読み、親依存を検査する。キャッシュ済みの親であっても深さ・参照数の上限確認は省かない。
3. 粗い親から正規XYZを復号し、点数・SHA-256を確認する。キャッシュキーはデータセット＋z/x/y。
4. 制御点のDelaunay接続と完全なVoronoiセルから自然近傍の探索補助情報を作る。表示時に制御点と補助情報をGPUへ転送する。
5. GPUで曲面を評価し、画面占有面積に応じたパッチ分割で描画する。離れたタイルに固定1024分割を強制しない。
6. 異なるLOD、部分境界、LOD遷移の接合は現在・以前の両曲面を共通境界で評価する。初回読込を標高0から持ち上げる補間にしない。

自然近傍は、評価位置に新しく挿入したVoronoiセルが各既存セルから奪う面積を正規化したSibson補間。任意の4点・近い点だけに置き換えない。表示用の傾き調整は別設定であり、制御点Zを変更しない。形式版1の基準曲面は調整量0。正式ビューワーは傾き調整量を変更できる。

参照実装は [整数復号](../common/src/sibson_native_decoder.rs)、[コンテナ読取](../common/src/sqlite.rs)、[自然近傍](../common/src/natural_surface.rs)、[GPU探索情報](../common/src/natural_regional_gpu.rs)、[GPU描画](../../viewer/terrain/src/sibson_gpu_controls.wgsl)。

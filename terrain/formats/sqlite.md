# 地形のSQLite形式 版1

[地形データ](../README.md) / [共通SQLiteコンテナ](../../formats/sqlite.md)

この文書は `kind='terrain'`、`format_version=1` の読み取り契約。コンテナの `user_version=1` とデータセット形式版、データの配布識別子 `data_version` は別物である。地表と同居できる。未知のデータセット形式版・encodingは推測で復号しない。

タイル行の`encoding`には`natural-controls-v1`を使う。`settings_json.encoding`を記録する場合は、タイル行と同じ値にする。

## 版について

| 対象 | 版・識別子 |
|---|---|
| 共通コンテナ | `PRAGMA user_version=1` |
| 地形データセット | `datasets.kind='terrain'`、`format_version=1` |
| 自然近傍制御点 | `natural-controls-v1`、magic `NTS1` |

manifestの`dataSchemaVersion`は共通コンテナの版であり、1を使う。地形形式版はSQLite内の`datasets.format_version`で判定し、対応する版は1のみとする。NTS1はタイルの符号化方式を表す識別子。

## テーブル

```sql
SELECT dataset_key,id,title,data_version,metadata_json
FROM datasets WHERE kind='terrain' AND format_version=1;
SELECT level_key,z,settings_json FROM terrain_levels
WHERE dataset_key=:dataset ORDER BY z;
SELECT x,y,encoding,payload FROM terrain_tiles
WHERE dataset_key=:dataset AND level_key=:level;
```

`level_key` はDB内のキー。XYZズームは `terrain_levels.z` を使う。同じデータセットで同一zを重複定義しない。登録した各ズームにはタイルを1枚以上収録する。`terrain_tiles` の主キーは `(dataset_key,level_key,x,y)`。x・yは0以上 `2^z` 未満。タイルが存在しない領域を、標高0の新規タイルとして合成しない。

`settings_json` の `errorDownMetres`・`errorUpMetres` は元DEMに対する下・上の誤差許容量。粗細レベル間の高さの大小制約ではない。地域試験データでは `metadata.generation.scope="window-test"` とし、`metadata.generation.sourceWindowZ14` に元DEMの対象範囲 `[x最小,y最小,x最大,y最大]`（右下を含まない）を記録できる。粗いXYZタイルの外形全域が元DEMで監査済みとは限らない。両者が同じ値でもよく、ビューワーは生成器の既定値を決め打ちせず、各レベルに記録された大きい方の誤差で自動LODを評価する。制御点レベルでは `interpolation='sibson'`、`xyStepMetres=0.0625`、`heightStepMetres=0.125`、`maxParentDepth=2`、`maxDependencies=16` を記録する。`anchor` は生成側の独立化方針であり、復号器は各BLOBの実際の親参照を優先する。


## 座標と整数

XYZの北西が原点、Xは東、Yは南へ増加する。投影幅 `W=40075016.68557849` m。EPSG:3857の東距・北距そのものではなく、`X=easting+W/2`、`Y=W/2-northing` とした正の世界座標を使う。辺長は `W/2^z`、範囲は `[x*辺長,y*辺長,(x+1)*辺長,(y+1)*辺長]`。

各タイルには3点以上の制御点を収録する。XYの組はタイル内で一意とし、一直線上に並ばない3点を含める。制御点の正規XYZは3個の符号付き32bit整数。X・Yを16で割り、Zを8で割るとm単位となる。**固定16bit座標ではない**。局所原点と差分を整数符号化することで容量を削減する。タイル外の制御点は自然近傍の正しい支持領域として必要なので、タイルの四角形で切り捨てない。

全球のX・Yをfloat32に変換してから原点を引くと精度を失う。整数をfloat64でmへ戻してタイル原点を引き、その後にGPU用float32へ変換する。

## 外側BLOB：NTS1

`encoding='natural-controls-v1'`。全整数・floatのバイト順はlittle endian。圧縮ファイル全体のSHA-256はmanifest、復元点のSHA-256はこのヘッダーに記録する。

| オフセット | 型 | 内容 |
|---:|---|---|
| 0 | 4 bytes | ASCII `NTS1` |
| 4 | i32 | 制御点本文の局所原点X（1/16m） |
| 8 | i32 | 制御点本文の局所原点Y（1/16m） |
| 12 | u32 | 復元点数N（3以上、u32の表現範囲） |
| 16 | u32 | 直接親タイル数P（0〜4） |
| 20 | 32 bytes | 全点XYZのSHA-256（生の32byte） |
| 52 | P×12 bytes | 親の `(z,x,y)`、各u32。格納順がslot番号 |
| 52+12P | u32 | 続く制御点本文のバイト長L |
| 56+12P | L bytes | NQC1・NDC1・NQZ1・NDZ1 |

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

### NQZ1・NDZ1：大きなRiceブロック＋Zstandard

NQZ1は独立点、NDZ1は差分点。先頭24byteの項目、整数列の意味と並び、正規XYZの復元方法は、それぞれNQC1・NDC1と同じ。続くヘッダーは次のとおり。

| オフセット | 型 | 内容 |
|---:|---|---|
| 24 | u8 | Riceブロック長のlog2。7・9・10・12（128・512・1024・4096個） |
| 25 | 3 bytes | 予約領域。必ず0 |
| 28 | u32 | Zstandard展開後の本文長R |
| 32 | 残りすべて | 単一の標準Zstandardフレーム。辞書不要 |

展開後の本文は、列ごとの `u32バイト長 + 整数ストリーム`。各列の個数、各ブロックの方式0〜2、bit順序などは従来形式と同じで、ブロックごとの最大個数だけをオフセット24の値へ置き換える。Zstandard展開後にNQC1・NDC1の先頭24byteを連結し、指定ブロック長で復号する方法でも実装できる。

展開前にN・P・F・量子化幅・予約領域・ブロック長を検査し、Rを `31*N+64` 以下に制限する。展開した長さはRと一致し、フレームはちょうど末尾まで読み切ること。連結した別フレームや余分な末尾、切れたフレーム、整数列の個数不一致は拒否する。復元点数・整数XYZ・順序・SHA-256・親依存の検査は従来と同じで、省略しない。

生成器はZstandardレベル3で複数のブロック長を比較する。NTS1の親リストと整数点を変えず、本文が32byte以上小さくなる場合だけNQZ1・NDZ1を採用する。小さいタイルや圧縮効果のないタイルはNQC1・NDC1を維持する。同じレベルに複数形式が混在してよい。古い形式も継続して復号できる。

### 整数ストリーム

最初に個数をunsigned LEB128で記録する。NQC1・NDC1では最大128個ごとに `LEB128のブロック長 + ブロック本文` を保存する。LEB128は1byte下位7bitを順に連結し、上位bitが1なら続く。最大10byte、最後は0か1。各ブロックの先頭1byteが方式を指定する。

| 方式 | 本文 |
|---:|---|
| 0（FOR） | ZigZag基準値のLEB128、幅u8、各値と基準値の差を固定幅で格納。幅0〜63 |
| 1（ゼロ連続） | 0なら続くLEB128個の0を出力。非0のLEB128値vならZigZag逆変換(v−1)を1個出力 |
| 2（Rice） | kのu8（0〜32）、有効bit数のLEB128、bit本文。各ZigZag整数を商の0連続＋1とk bitの剰余で復元 |

ZigZag逆変換は `(v>>1) ^ -(v&1)`。bit本文はbyte内も整数内も最下位bitから読む。bit数を超えて読まない。固定幅の末尾paddingは値として数えず、Riceでは有効bitをちょうど読み切る。ブロックごとの個数は残り個数と128の小さい方であり、別の個数ヘッダーはない。未知の方式・過大な個数・ゼロ長run・overflow・余分なbyteは拒否する。

## 制御点の復号と描画

1. コンテナのapplication_id・user_version、データセット形式版、収録レベルとencodingを検査する。
2. NTS1を読み、親依存を検査する。キャッシュ済みの親であっても深さ・参照数の上限確認は省かない。
3. 粗い親から正規XYZを復号し、点数・SHA-256を確認する。キャッシュキーはデータセット＋z/x/y。
4. 制御点のDelaunay接続と完全なVoronoiセルから自然近傍の探索補助情報を作る。表示時に制御点と補助情報をGPUへ転送する。
5. GPUで曲面を評価し、画面占有面積に応じたパッチ分割で描画する。離れたタイルに固定1024分割を強制しない。
6. 異なるLOD、部分境界、LOD遷移の接合は現在・以前の両曲面を共通境界で評価する。初回読込を標高0から持ち上げる補間にしない。

自然近傍は、評価位置に新しく挿入したVoronoiセルが各既存セルから奪う面積を正規化したSibson補間。任意の4点・近い点だけに置き換えない。表示用の傾き調整は別設定であり、制御点Zを変更しない。通常のSibson基準曲面は調整量0。固定係数1の誤差保証はmetadataに記録した倍率1で評価する。正式ビューワーは傾き調整量を変更できる。

参照実装は [整数復号](../common/src/sibson_native_decoder.rs)、[コンテナ読取](../common/src/sqlite.rs)、[自然近傍](../common/src/natural_surface.rs)、[GPU探索情報](../common/src/natural_regional_gpu.rs)、[GPU描画](../../viewer/terrain/src/sibson_gpu_controls.wgsl)。


## 係数1固定の滑らかな曲面

`settings_json.slopeMode="local-hermite-v1"` は、H1をそのまま使う方式。局所勾配はDelaunayの一周近傍から距離の二乗の逆数を重みにした最小二乗法で求め、山頂・窪地では0とする。同一円上の四角形の対角辺は勾配推定から除く。評価位置q、Sibson面積座標w_i、点p_i、標高z_i、勾配g_i、距離r_iを使い、H0=Σw_i z_i、D=Σw_i/r_i、α=(Σw_i r_i)/D、β=Σw_i r_i²、T=Σ(w_i/r_i)(z_i+g_i·(q−p_i)) として H1=(αH0+βT/D)/(α+β) を復元する。制御点上はその標高をそのまま返す。表示調整量sではH0+(H1−H0)sとし、誤差保証はs=1を対象とする。法線は最終標高の位置変化から求める。全体倍率の初期値と監査値は1。局所極値で通常Sibson面へ戻さず、頂点へ近づくと傾きが0へ収束する滑らかな面を維持する。

`metadata.generation.fixedSlopeAdjustment` に方式名・`certifiedMultiplier=1`・`storedCoefficientBytes=0` を記録する。`peakGuardMetres=0.5` は、元DEMの格子で判定する余分な突起の許容量。評価画素の高さから周囲8画素の最高値を引いた正の値を突出量とし、補間面の突出量が元DEMの突出量を0.5mより多く超えないことを監査する。8近傍すべてに有効DEMがある画素を対象にし、元DEMタイル間も画素座標で接続する。

標高誤差の許容値を満たしていても形状監査に失敗すれば、頂点の上振れや周囲の下振れを戻すDEM点を追加する。追加削減では、元の局所山頂・窪地とその隣接点を保護し、余分な突起を再発させる候補を拒否する。元DEMの画素より細かなあらゆる突起を検出する保証ではなく、山頂でのゼロ勾配・制御点付近の滑らかさの試験と併せて評価する。


## 小起伏を省いた地形の誤差基準

レベルの `settings_json.errorReference` は `raw-dem`（省略時の従来解釈）または `smoothed-dem`。`smoothed-dem` の `errorUpMetres`・`errorDownMetres` は、細かな起伏を除いたDEMに対する許容誤差であり、原DEMとの差の上限ではない。標高曲面の復元方法・座標・固定1の補間方式は変わらない。ビューワーは記録された制御点から描画し、元DEMや平滑化の再実行を必要としない。

`metadata.generation.sourceSmoothing.mode="ridge-preserving-gaussian-v2"` は主要地形を保護する前処理。`config` に `sigma_pixels`・`max_vertical_change_metres`・`feature_radius_pixels`・`feature_prominence_metres` を記録する。対象はz14・256pxのDEM格子。既定は4画素・15m・12画素・3mで、係数の保存は不要。

半径 `ceil(3σ)` の正規化したガウスでσと2σの二つの曲面を求める。それぞれの離散カーネルの二次モーメントをv_f・v_cとし、`S=(v_c*G_f−v_f*G_c)/(v_c−v_f)` で広い平面・二次曲面の変形を抑える。主要な稜線・谷筋は二スケールのHessian主方向・曲率の一致・稜線からの距離で保護し、半径内の有意な極値も保護する。保護量wで `raw+clamp((S−raw)*(1−w),−上限,+上限)` とし、元DEMの3×3最小・最大にも制限する。欠損は欠損のまま、出力は1cm単位へ揃える。

`windowZ14` は処理する半開区間、`sourceGenerationId` と `rawInputManifestSha256` は元入力の識別子。`targetSha256` は対象窓を北から南・西から東へ並べた標高のSHA-256。標高はcm単位のi32 little endian、欠損はi32最小値。検証器は元DEMを再取得せず固定済み原本を照合し、同じ設定で再計算した対象標高ハッシュが一致することを確認する。未知の前処理をビューワーで推測して適用しない。

`rmsChangeMetres`・`maximumChangeMetres` は前処理による原DEMからの変更量。最高標高や特徴点数は前処理の監査値で、制御点削減後の誤差は全国生成器の全画素監査で検証する。点の標高は共通の1/8m量子化を使うため、原DEMのcm単位の山頂高と表示曲面の山頂高には量子化差が生じ得る。

### 全国へ分割適用した平滑化の監査情報

全国生成の `generation.sourceSmoothing` も `mode="ridge-preserving-gaussian-v2"` と同じ `config` を使う。原本の範囲全体を一枚の巨大画像にせず、z14の8×8タイルを処理窓、周囲1タイルを補助領域として計算する。配布ファイルには平滑化前の原DEMや保護マスクを含めない。

地域試験の `windowZ14`・`targetSha256` の代わりに、全国版は `chunkTiles=8`・`haloTiles=1`・`tiles`・`targetTileInventorySha256` を記録する。`sourceGenerationId`・`rawInputManifestSha256` は平滑化前の原本、`targetTileInventorySha256` は平滑化済み基準の一覧を識別する。後者は `(y,x)` の昇順で、各タイルのx・yをu32 little-endian、作業キャッシュ本文のSHA-256を小文字ASCII64文字として連結したバイト列のSHA-256である。作業本文は各画素についてi32 little-endianのセンチメートル標高とu8の保護フラグを並べ、Zstandardレベル3で圧縮する。欠損標高はi32最小値、保護フラグは0または1。これは生成の監査情報であり、ビューワーがこの作業キャッシュを読む必要はない。

全レベルの `slopeMode="local-hermite-v1"`・`errorReference="smoothed-dem"` と `generation.errorEvaluationMultiplier=1` に従って描画する。`generation.fixedSlopeAdjustment.peakGuardMetres=0.5` は、最終曲面の画素中心における周囲8画素への突出量が平滑化DEMの突出量を超えて増える許容値であり、タイルの上下許容誤差とは別の検査である。

制御点数には262,144点という固定上限を設けない。NTS1および制御点本文の点数欄はu32として検証し、復元配列のサイズはオーバーフローを確認して確保する。バイト長・列の個数・親参照・SHA-256・非退化な接続の検査は省かない。ビューワーは実機GPUのバッファ上限を検査し、点数を減らして読み替えない。

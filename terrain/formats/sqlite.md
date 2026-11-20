# 地形のSQLite形式 版1

[地形データ](../README.md) / [共通SQLiteコンテナ](../../formats/sqlite.md)

この文書は `kind='terrain'`、`format_version=1` の読み取り契約。コンテナの `user_version=1` とデータセット形式版、データの配布識別子 `data_version` は別物である。地表と同居できる。未知のデータセット形式版・encodingは推測で復号しない。

同じデータセットに制御点とハイトマップをレベルごとに収録できる。1レベル内のencodingは統一する。全レベルの`settings_json.encoding`を必須とし、タイル行の`encoding`と一致させる。

## 版について

| 対象 | 版・識別子 |
|---|---|
| 共通コンテナ | `PRAGMA user_version=1` |
| 地形データセット | `datasets.kind='terrain'`、`format_version=1` |
| 自然近傍制御点 | `natural-controls-v1`、magic `NTS1` |
| ハイトマップ（16bit可逆） | `heightmap-u16-zstd-v1`、magic `HMP1` |
| ハイトマップ（8bit非可逆・旧量子化） | `heightmap-u8-webp-v1`、magic `HMW1` |
| ハイトマップ（8bit非可逆・最低最高標高） | `heightmap-u8-webp-minmax-v1`、magic `HMW2` |

manifestの`dataSchemaVersion`は共通コンテナの版であり、1を使う。地形形式版はSQLite内の`datasets.format_version`で判定し、対応する版は1のみとする。NTS1・HMP1・HMW1・HMW2はタイルの符号化方式を表す識別子であり、地形データセットの形式版とは独立する。

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

`settings_json` の `errorDownMetres`・`errorUpMetres` は元DEMに対する下・上の誤差許容量。粗細レベル間の高さの大小制約ではない。地域試験データでは `metadata.generation.scope="window-test"` とし、`metadata.generation.sourceWindowZ14` に元DEMの対象範囲 `[x最小,y最小,x最大,y最大]`（右下を含まない）を記録できる。粗いXYZタイルの外形全域が元DEMで監査済みとは限らない。両者が同じ値でもよく、ビューワーは生成器の既定値を決め打ちせず、各レベルに記録された大きい方の誤差で自動LODを評価する。制御点レベルでは `interpolation='sibson'`、`xyStepMetres=0.0625`、`heightStepMetres=0.125`、`maxParentDepth=2`、`maxDependencies=16` を記録する。`anchor` は生成側の独立化方針であり、復号器は各BLOBの実際の親参照を優先する。

国土地理院制御点データは、基本生成のz6〜z13に加え、z6の認証済み点から削減したz5・z4も収録できる。追加レベルの上下許容誤差はz5=625m、z4=1250m。z4・z6・z9・z12を独立アンカーにし、z5の差分はz4を参照できる。NTS1の構造は変更しない。ビューワーは最小zを6に固定せず、収録レベルと各レベルの誤差で選択する。

## 座標と整数

XYZの北西が原点、Xは東、Yは南へ増加する。投影幅 `W=40075016.68557849` m。EPSG:3857の東距・北距そのものではなく、`X=easting+W/2`、`Y=W/2-northing` とした正の世界座標を使う。辺長は `W/2^z`、範囲は `[x*辺長,y*辺長,(x+1)*辺長,(y+1)*辺長]`。

制御点の正規XYZは3個の符号付き32bit整数。X・Yを16で割り、Zを8で割るとm単位となる。**固定16bit座標ではない**。局所原点と差分を整数符号化することで容量を削減する。タイル外の制御点は自然近傍の正しい支持領域として必要なので、タイルの四角形で切り捨てない。

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

## 制御点の復号と描画

1. コンテナのapplication_id・user_version、データセット形式版、収録レベルとencodingを検査する。
2. NTS1を読み、親依存を検査する。キャッシュ済みの親であっても深さ・参照数の上限確認は省かない。
3. 粗い親から正規XYZを復号し、点数・SHA-256を確認する。キャッシュキーはデータセット＋z/x/y。
4. 制御点のDelaunay接続と完全なVoronoiセルから自然近傍の探索補助情報を作る。表示時に制御点と補助情報をGPUへ転送する。
5. GPUで曲面を評価し、画面占有面積に応じたパッチ分割で描画する。離れたタイルに固定1024分割を強制しない。
6. 異なるLOD、部分境界、LOD遷移の接合は現在・以前の両曲面を共通境界で評価する。初回読込を標高0から持ち上げる補間にしない。

自然近傍は、評価位置に新しく挿入したVoronoiセルが各既存セルから奪う面積を正規化したSibson補間。任意の4点・近い点だけに置き換えない。表示用の傾き調整は別設定であり、制御点Zを変更しない。形式版1の基準曲面は調整量0。正式ビューワーは傾き調整量を変更できる。

参照実装は [整数復号](../common/src/sibson_native_decoder.rs)、[コンテナ読取](../common/src/sqlite.rs)、[自然近傍](../common/src/natural_surface.rs)、[GPU探索情報](../common/src/natural_regional_gpu.rs)、[GPU描画](../../viewer/terrain/src/sibson_gpu_controls.wgsl)。

## ハイトマップ

### レベル設定

全レベルの`settings_json.encoding`を必須とする。タイル行の`encoding`も同じ値でなければならない。`errorDownMetres`と`errorUpMetres`は正の有限値で、LOD選択では大きい方を使う。形式によって単位を変更しない。

制御点レベルの設定は前述のとおり。ハイトマップレベルには以下を記録する。

| 項目 | 値・意味 |
|---|---|
| `encoding` | `heightmap-u16-zstd-v1`、`heightmap-u8-webp-v1`、`heightmap-u8-webp-minmax-v1` |
| `cellsPerTile` | 8〜256の2の累乗。タイル一辺のセル数N |
| `halo` | 1。タイルの外へ保持する余白の標高点数 |
| `heightStepMetres` | u16版のみ。0.001〜100の有限値。既定0.125m |
| `minimumHeightStepMetres` | 旧WebP版（HMW1）のみ。タイルごとの量子化幅の下限。0.001〜100の有限値 |
| `heightScale` | HMW1は`per-tile`、HMW2は`min-max` |
| `webpQuality` | WebP版のみ。0〜100の整数。圧縮品質であり標高精度の保証ではない |
| `interpolation` | `bilinear` |
| `anchor` | true。独立タイル |
| `maxParentDepth` / `maxDependencies` | 0 / 1 |
| `coverageAudit` | 生成器による元DEMの表現可能画素数・未表現画素数 |

`coverageAudit.representedDemPixels`は補間できた有効DEM画素数、`unrepresentedDemPixels`は格子の欠損によって補間できない有効DEM画素数。`errorDomain='represented-valid-dem-pixels'`を記録する。生成器が計測した上下誤差は前者上の値であり、未表現画素を誤差0と扱ったり、全有効DEMを覆う保証と読み替えたりしない。表示用メッシュによる近似誤差は別に扱う。

### 標高格子

内部は両端を含む(N+1)×(N+1)点。余白を含む保存寸法は(N+3)×(N+3)点。N=256なら259×259点となる。行優先で、Xは東、Yは南へ増加する。保存配列の(i,j)の世界座標は次のとおり。

```text
side = 40075016.68557849 / 2^z
X = x*side + (i-halo)*side/N
Y = y*side + (j-halo)*side/N
```

u16版の標高はlittle endianのu16。65535は欠損であり、海面の0mと区別する。それ以外は`heightMetres=(base+value)*heightStepMetres`。baseは符号付きi32の量子化単位で、m単位ではない。全点欠損のタイルも表現でき、その領域の標高は不明である。

隣接タイルは同じ世界座標で同じ元DEMを標本化する。u16版は基準標高がタイルごとに異なってよいが、共有点の復元標高を一致させる。WebP版はタイル単位の量子化と非可逆圧縮により共有点に差が生じ得るため、ビューワーの接合処理で閉じる。標高差がu16に入らない場合は失敗させ、飽和や桁あふれで保存しない。

基準曲面は四隅のバイリニア補間。正の重みを持つ点に欠損があれば、その評価は欠損となる。余白を法線用の差分評価に使う。海面が確認できる場合は有効値として0mを保存する。

### HMP1ヘッダー

すべてlittle endian。整数幅とバイト数は以下のとおり。

| オフセット | 型 | 内容 |
|---:|---|---|
| 0 | 4 bytes | ASCII `HMP1` |
| 4 | u32 | セル数N |
| 8 | u32 | halo（1） |
| 12 | u32 | 保存点数(N+3)^2 |
| 16 | i32 | 基準標高base（量子化単位） |
| 20 | f64 | heightStepMetres |
| 28 | u32 | 復号後バイト数。保存点数×2 |
| 32 | u32 | 圧縮本文のバイト数L |
| 36 | 32 bytes | 復号後u16配列のSHA-256 |
| 68 | u32 | 予約。0以外は拒否 |
| 72 | L bytes | u16配列をZstandardで可逆圧縮した1フレーム |

全BLOBは最大2MiB、復号後配列は最大134,162byte。余分な末尾、連結フレーム、過大な復号結果、1MiBを超えるZstandardウィンドウは拒否する。寸法・量子化幅はレベル設定との一致も検査する。

### HMW1ヘッダーとWebP本文

格子の座標と寸法はHMP1と共通。標高をタイルごとの0〜255階調へ割り当て、同じ階調をRGBの各チャンネルに入れてWebP非可逆で圧縮する。alphaは0が欠損、255が有効で、二値の欠損マスクを可逆に保持する。階調255も有効値。復号標高は`(base+R)*step`。alpha=0のRGBは無視する。量子化・WebP圧縮による誤差を含め、実際に復号した標高でDEM監査を行う。

生成器は有効標高の最小・最大から`step=max(minimumHeightStepMetres,(max-min)/254)`、`base=floor(min/step)`を求め、基準値の切捨て分も含め0〜255へ収める。全点欠損ならbase=0、stepは指定下限。標高をクリップして範囲へ収めない。

| オフセット | 型 | 内容 |
|---:|---|---|
| 0 | 4 bytes | ASCII `HMW1` |
| 4 / 8 / 12 | u32×3 | セル数N / halo=1 / 保存点数(N+3)^2 |
| 16 | i32 | 基準標高base（量子化単位） |
| 20 | f64 | タイルごとのstep。0.001〜100m、レベルの指定下限以上 |
| 28 | u32 | 復号後RGBAバイト数。保存点数×4 |
| 32 | u32 | WebP本文のバイト数L |
| 36 | 32 bytes | 圧縮したWebP本文そのもののSHA-256 |
| 68 | u32 | WebP品質0〜100。レベル設定との一致が必要 |
| 72 | L bytes | RIFF/WEBP形式の静止画1枚 |

全BLOBは最大2MiB。RIFFに記載された本文長と実際の長さを一致させ、余分な末尾を拒否する。WebP寸法が(N+3)×(N+3)であることを復号用メモリ確保前に検査する。復号後RGBAは最大268,324byte。alphaが0・255以外の場合は拒否する。WebP本文のSHA-256は格子の量子化前・圧縮前の配列のハッシュではない。

### HMW2ヘッダーと最低・最高標高による復元

`heightmap-u8-webp-minmax-v1`は各タイルの最低・最高標高をf64で直接保存する。対象は余白を含む全有効格子点。0.125m等の量子化幅の下限や整数基準値への丸めを行わず、最低標高を階調0、最高標高を255へ割り当てる。

```text
code = round((height-minHeightMetres)/(maxHeightMetres-minHeightMetres)*255)
height = minHeightMetres + R/255*(maxHeightMetres-minHeightMetres)
```

最低＝最高のタイルは階調0を保存し、復号時はその一定標高を使う。全点欠損なら最低・最高をともに0とし、全alphaを0にする。最低＝最高でも有効値と欠損を区別する。WebPとalphaの扱いはHMW1と同じで、階調255も有効値。非可逆圧縮によって実際の復号画素が0・255に一致する保証はない。

| オフセット | 型 | 内容 |
|---:|---|---|
| 0 | 4 bytes | ASCII `HMW2` |
| 4 / 8 / 12 | u32×3 | セル数N / halo=1 / 保存点数(N+3)^2 |
| 16 | f64 | 最低標高minHeightMetres |
| 24 | f64 | 最高標高maxHeightMetres |
| 32 | u32 | 復号後RGBAバイト数。保存点数×4 |
| 36 | u32 | WebP本文バイト数L |
| 40 | 32 bytes | WebP本文のSHA-256 |
| 72 | u32 | WebP品質0〜100。レベル設定と一致が必要 |
| 76 | u32 | 予約。0以外は拒否 |
| 80 | L bytes | RIFF/WEBP形式の静止画1枚 |

最低・最高標高は有限値で最低≦最高、差も有限値とする。差/255は0〜100m。HMW1と同じ長さ・寸法・alpha・SHA-256検査を行う。`minimumHeightStepMetres`は使わない。GPUには最低標高と`(最高-最低)/255`を渡し、標高と法線を復元する。レベル別の誤差はWebP復号後の実測値を記録する。旧HMW1も引き続き読み込める。

### 親参照と分割配布

ハイトマップに親参照はない。制御点形式のNTS1が参照できる親は、同じデータセットの制御点形式だけ。生成時に選択されない親、またはハイトマップへ切り替えた親を参照する制御点タイルは、XYZを変えず独立NQC1へ符号化し直す。親に残る点の順序や整数ハッシュは変更しない。

ズームで分割するとき、ハイトマップのBLOBはそのままコピーできる。制御点形式は既存の親依存検査を続け、分割後に親が欠ける場合は拒否する。地表等と統合するときもdataset_keyをBLOBに含めないため書き換え不要。

## 読み込みと検証

1. 共通コンテナとデータセットの形式版を確認する。
2. レベル設定のencodingと固有項目を検査する。未知の形式は拒否する。
3. タイルのXYZとencodingを検査し、指定された復号器へ渡す。
4. 制御点はNTS1の依存・点数・ハッシュ検査を行う。格子はヘッダー・長さ・圧縮フレーム・ハッシュ・レベル設定との一致を検査する。
5. 格子の標高値をGPUへ送り、Compute Shaderで頂点標高・法線を評価する。三角形インデックスもGPUで生成する。
6. カメラ、LOD、描画用の評価区画、接合、遮蔽判定、オンライン地図・地表の貼付は共通処理を使う。格子にDelaunay/Voronoiの生成やSibson補間は行わない。

自然近傍の傾き補正は制御点形式に適用する。ハイトマップの基準曲面はバイリニアで、傾き補正によって格子標高を変更しない。欠損の有効性はGPUの評価格子にも保持し、欠損を0mの面として描かない。

## JSONの例

ハイトマップレベルの設定例（誤差とcoverageAuditは生成器の実測値を入れる）。

```json
{"encoding":"heightmap-u16-zstd-v1","cellsPerTile":256,"halo":1,"heightStepMetres":0.125,"interpolation":"bilinear","anchor":true,"maxParentDepth":0,"maxDependencies":1,"errorDownMetres":20,"errorUpMetres":20}
```

最低・最高標高方式の設定例（誤差は実測値へ置き換える）。

```json
{"encoding":"heightmap-u8-webp-minmax-v1","cellsPerTile":256,"halo":1,"heightScale":"min-max","webpQuality":90,"interpolation":"bilinear","anchor":true,"maxParentDepth":0,"maxDependencies":1,"errorDownMetres":20,"errorUpMetres":20}
```

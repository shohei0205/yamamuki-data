# 地形のSQLite形式 版2

[地形データ](../README.md) / [共通SQLiteコンテナ](../../formats/sqlite.md) / [地形形式版1](sqlite.md)

## 概要

同じ地形データセットに、レベルごとに自然近傍制御点とハイトマップを収録する。1レベル内では形式を統一する。既存の地形形式版1は変更せず、読み取りも引き続き対応する。

## 版について

| 対象 | 版・識別子 |
|---|---|
| 共通コンテナ | `PRAGMA user_version=1` |
| 地形データセット | `datasets.kind='terrain'`、`format_version=2` |
| 自然近傍制御点 | `natural-controls-v1`。本文は[版1](sqlite.md)と同じ |
| ハイトマップ（16bit可逆） | `heightmap-u16-zstd-v1`、magic `HMP1` |
| ハイトマップ（8bit非可逆・旧量子化） | `heightmap-u8-webp-v1`、magic `HMW1` |
| ハイトマップ（8bit非可逆・最低最高標高） | `heightmap-u8-webp-minmax-v1`、magic `HMW2` |

形式版1しか実装していないアプリは版2を推測で開かない。manifestの`dataSchemaVersion`は共通コンテナの版であり、この追加でも1を維持する。地形形式版はSQLite内の`datasets.format_version`で判定する。

## 項目

テーブルは共通コンテナの`terrain_levels(dataset_key,level_key,z,settings_json)`と`terrain_tiles(dataset_key,level_key,x,y,encoding,payload)`を使う。XYZの向き・投影幅・座標範囲は版1と同じ。`level_key`はDB内のキー、`z`はXYZズーム。同一データセットで同じzを重複させない。

### レベル設定

全レベルの`settings_json.encoding`を必須とする。タイル行の`encoding`も同じ値でなければならない。`errorDownMetres`と`errorUpMetres`は正の有限値で、LOD選択では大きい方を使う。形式によって単位を変更しない。

制御点レベルは版1の設定を引き継ぐ。ハイトマップレベルには以下を追加する。

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
4. 制御点は版1の依存・点数・ハッシュ検査を行う。格子はヘッダー・長さ・圧縮フレーム・ハッシュ・レベル設定との一致を検査する。
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

## 変更履歴

- 版2（2026-11-19、未公開）: レベルごとのencoding、u16/Zstandard可逆・u8/WebP非可逆（旧量子化・最低最高標高）の独立ハイトマップ、制御点形式との混在、GPU格子生成を追加。

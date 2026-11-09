# 地表画像のSQLite形式 版1

この仕様は `datasets.kind = 'surface'` かつ `datasets.format_version = 1` のデータセットに適用する。ファイルの識別、データセット台帳、共通メタデータ、配布manifestは [山むきSQLiteコンテナ仕様](../../formats/sqlite.md) に従う。

## レイヤーとレベルを読み込む

```sql
SELECT layer_key, id, role, draw_order, source_names_json, settings_json
FROM surface_layers
WHERE dataset_key = :dataset
ORDER BY draw_order, layer_key;

SELECT z, lod, tile_pixels
FROM surface_levels
WHERE dataset_key = :dataset AND layer_key = :layer
ORDER BY z;
```

| テーブル | キーと列の意味 |
| --- | --- |
| `surface_layers` | 主キーは整数の `(dataset_key, layer_key)`。`id` はデータセット内で一意の文字列。`role` は `rgb`・`mask`・`hillshade`。`draw_order` は整数。`source_names_json` は文字列配列、`settings_json` はオブジェクト |
| `surface_levels` | 主キーは `(dataset_key, layer_key, z)`。`z` は実XYZズームで整数0～30。`lod` は互換性のために残す文字列で、同じレイヤー内で一意。`tile_pixels` は正方形タイルの一辺で整数1～4096 |
| `surface_tiles` | 主キーは整数の `(dataset_key, layer_key, z, x, y)`。`encoding` は画像形式ID、`payload` は画像ファイル全体のBLOB |

下から上へ `(draw_order, layer_key)` の昇順に描く。レベル名は実XYZズームの `z` に基づき、`z10` のように表示する。新しく生成するSQLiteは `lod` にも同じ文字列を格納する。既存ファイルの `lod` に旧ラベルが入っていても、表示名とXYZズームの判定には `z` を使う。

地表では、表示中のレイヤーの `source_names_json` を読み、同じデータセットの `sources[].name` と照合する。帰属表示には `attribution` を用い、出典や利用条件の詳細には `source`、`license`、`changes` を使う。複数レイヤーで同じ出典を使う場合は一度表示すればよい。

## レイヤーの種類を判別する

`surface_layers.id` はレイヤーが表す内容を識別する機械可読のID。以下のIDは地表形式の版1で予約し、データセットや配色が変わっても同じ意味で使う。アプリは大文字・小文字を区別して完全一致で比較する。

| `id` | 必須の `role` | 内容 |
| --- | --- | --- |
| `forest` | `mask` | 森林の被覆濃度。0は森林なし、255は全面が森林 |
| `water` | `mask` | 水域の被覆濃度。0は水域なし、255は全面が水域。海・河川・湖沼を含む |
| `hillshade` | `hillshade` | 標高から作った陰影。画素値と `neutral` の差から、下に描いた色の明るさを調整する |

`id` で表示名・表示スイッチ・アプリ独自の配色を選び、`role` で画素の読み方と合成方法を選ぶ。`layer_key` は同じデータセット内でレベルやタイルを引くためのキー。`draw_order` は描画順だけを示す。色・描画順・キーの数値・データセット名からレイヤーの種類を推定しない。

レイヤーの収録は任意。たとえば `forest` が無ければ、そのデータセットに森林レイヤーは無い。未知のIDは、対応する `role` があれば汎用レイヤーとして描画できる。予約IDに表と異なる `role` を組み合わせた行は仕様違反として扱い、その種類のレイヤーとして認識しない。

## 座標と画面上の配置

EPSG:3857のXYZタイルで、原点は左上、Xは東向き、Yは南向き。経度 `lon` と緯度 `lat` は度。緯度を ±85.0511287798° に制限し、次でワールド座標を求める。

```text
u = (lon + 180) / 360
v = (1 - asinh(tan(lat * π / 180)) / π) / 2
n = 2^z
x = floor(u * n)
y = floor(v * n)
```

日付変更線の両端を区別して描く場合はXを折り返し、折り返さない表示では端を `0 .. n-1` に制限する。Yは折り返さない。タイル `(z,x,y)` の地理範囲は `[x/n, (x+1)/n] × [y/n, (y+1)/n]`。TMSのようなYの反転は不要。`tile_pixels` はこの範囲内のサンプル数であり、タイルの地理範囲を変えない。

タイル内の正規化位置は `(u*n-x, v*n-y)`。画素 `(i,j)` の中心は `((i+0.5)/tile_pixels, (j+0.5)/tile_pixels)`。手動でバイリニア補間するなら連続画素位置を `uv*tile_pixels-0.5` として求める。境界をまたぐ補間には隣接タイルの端の画素を使える。対応する隣接タイルがなければ、そのレイヤーの欠損規則を適用する。

## 表示するLODを決める

固定LODはレイヤーの `surface_levels` から選ぶ。自動LODでは `2^z * tile_pixels` をワールド全幅あたりの画素数として比較できる。たとえば真上表示で画面幅 `W` ピクセルにワールドの幅 `Δu` が映るなら、目安は `W/Δu`。目安を満たす最も粗い収録レベルを選び、足りなければ最も細かい収録レベルを使う。傾けた画面では位置ごとの投影倍率を用いてもよい。自動LODのしきい値や切り替え時間はビューワーの設定。

ネイティブビューワーの自動LODでは、`completeTileExtent` を持つデータセットで選択したレベルにタイル行がないとき、その場所を収録する最も細かい親タイルを使う。

森林、水域、陰影が別レベルでも、同じワールド位置に対し、それぞれの `z` と `tile_pixels` でサンプルする。レベル間を滑らかに切り替えるときは、マスク値や陰影値を混ぜてから一度だけ合成する。RGBの透明画像を混ぜるときはプリマルチプライドRGBAを使う。

## タイルを取得・復号する

```sql
SELECT t.encoding, t.payload, l.tile_pixels
FROM surface_tiles AS t
JOIN surface_levels AS l USING(dataset_key, layer_key, z)
WHERE t.dataset_key = :dataset AND t.layer_key = :layer
  AND t.z = :z AND t.x = :x AND t.y = :y;
```

| `encoding` | 復号方法 |
| --- | --- |
| 1 | PNG。グレースケールPNGの1ビット・4ビットなどは0～255に展開する |
| 2 | WebP。グレースケール相当のマスクと陰影は復号後のGチャンネルを使う |
| 3 | JPEG。RGB画像として復号する |

幅と高さが両方とも `tile_pixels` に一致することを確認する。未知のエンコーディング、破損BLOB、寸法不一致は復号エラーとして扱う。行が存在しない場合は、読み込み中や復号失敗と区別して扱う。初回は台帳・レイヤー・レベルだけ読み、可視タイルのBLOBを必要になったときに取得する。タイルキャッシュのキーには少なくともファイル、データセット、レイヤー、`z/x/y` を含める。

## レイヤーを合成する

各式は0～255の色成分を用いる。RGBA配列は `[R,G,B,A]`。以下の設定名は `settings_json` のキー。

`mask` の `color` と `background` は、データセットが提供する表示用の既定色。アプリは独自の配色で森林色、水域色、地色を指定してよい。どの配色を使っても、濃度値の意味と合成順は同じ。

| 役割 | 設定と描画 |
| --- | --- |
| `rgb` | 復号したRGB(A)を描く。RGBは不透明。タイルがなければ透明 |
| `mask` | `color` のRGBAをマスク値 `m/255` で着色する。`background` があればそのRGBAの上に着色を合成する。マスク本体のタイルがなければ、このレイヤーは透明 |
| `hillshade` | `neutral` と `strength` を使い、ここまで描いた色に陰影を掛ける。アルファ値は維持。タイルがなければ中立値 |

`mask` の `sampleBits` は元の濃度の段階数を示す。4ビットPNGの値 `k` は `m = 17*k`。可逆WebPも同じ濃度を8ビットに展開済みで格納する。すでに展開された値をもう一度17倍する必要はない。`color[3]` を `a_c` とすると、着色のアルファは `(a_c/255)*(m/255)`。`background` が不透明で着色も不透明色の場合は次の式で各色成分を計算できる。

```text
out = round(background * (1 - m/255) + color * m/255)
gain = 1 + strength * (shade - neutral) / neutral
shaded = clamp(round(out * gain), 0, 255)
```

一般の透明レイヤーは source-over 合成を使う。前景と背景をそれぞれプリマルチプライド色 `P_s, P_d`、アルファを `A_s, A_d` とすると `P = P_s + (1-A_s)*P_d`、`A = A_s + (1-A_s)*A_d`。不透明色だけを扱う場合は通常の色の線形補間でよい。参照式は格納されたRGBチャンネル値に直接適用する。

格納済みの森林タイルでは全画素を描き、森林濃度0の箇所に地色を表示する。ESAデータセットで `completeTileExtent` が `true` の場合、森林・水域・陰影の収録タイル全域を同じ地理範囲として生成済み。粗いレベルの描画範囲を細かいレベルから推定する必要はない。タイル行がない場所はレイヤー単位の欠損規則に従う。既存の版1ファイルに補助レイヤーの `coverage` があれば読み飛ばす。

全国データの照合例は [データセットのREADME](../esa_worldcover/README.md#全国データの収録内容) を参照する。

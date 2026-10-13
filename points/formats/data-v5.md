# 地点データフォーマット (版5)

manifest の `schemaVersion` が `5`、データ本体（地点データ）の `dataSchemaVersion` が `5` の版の形式を説明する。この版は 2026-10-07 から配っている。版5 からは、manifest の版とデータ本体の版を別々に数える。

前の形式は [版4](data-v4.md) を参照する。版ごとの違いは、この文書の「変更履歴」にまとめている。

Release のタグ・ファイル名・置き場所など、データごとの配り方は各データの README（例: [OSM 山頂](../osm-peaks/README.md#置き場所)）に書く。この文書の例は、OSM 山頂の正式版 `osm-peaks-20261007T071401Z-37585754546-1` から取った。

## 読み方

1. 使う版の manifest を取る（Release の添付ファイル、または開発版の最新版）。
2. `schemaVersion` が `5`、`dataSchemaVersion` が `5` であることを確かめる。
3. `downloadUrl` からデータ本体を取り、展開する前に `sizeBytes` と `sha256` を照合する。
4. gzip を展開し、展開後の大きさを `uncompressedSizeBytes` と、配列の件数を `pointCount` と照合する。
5. 山頂だけを使うときは、`type` が `peak` の地点を選ぶ（`type` の無い地点は種別不明）。知らない種別や項目は無視する。

## manifest

次は、OSM 山頂の Release `osm-peaks-20261007T071401Z-37585754546-1` に添付した `manifest.json` の実物。

```json
{
  "schemaVersion": 5,
  "dataSchemaVersion": 5,
  "name": "山頂",
  "version": "20261007T071401Z-37585754546-1",
  "fileName": "osm-peaks.json.gz",
  "downloadUrl": "https://github.com/shohei0205/yamamuki-data/releases/download/osm-peaks-20261007T071401Z-37585754546-1/osm-peaks.json.gz",
  "sha256": "01c732e53edd96840111cd17361f2a4d5484e1c2887606663c554377047ee7ee",
  "sizeBytes": 505256,
  "uncompressedSizeBytes": 3203003,
  "pointCount": 14035,
  "sourceTimestamp": "2026-10-06T20:21:06Z",
  "latestPointTimestamp": "2026-10-06T19:13:49Z",
  "sourceUrl": "https://download.geofabrik.de/asia/japan-261006.osm.pbf",
  "license": "ODbL-1.0",
  "attribution": "© OpenStreetMap contributors"
}
```

| 項目 | 型 | 必須 | 内容 |
|---|---|---|---|
| `schemaVersion` | integer | 必須 | manifest の形式の版。`5` |
| `dataSchemaVersion` | integer | 新規生成では必須 | データ本体の形式の版。manifest の版とは別に数える。今は `5` |
| `name` | string | 任意 | データセットの表示名 |
| `version` | string | 必須 | データの版。英数字で始まり、英数字・ピリオド・ハイフン・下線でできる |
| `fileName` | string | 必須 | データ本体のファイル名。データセットごとに決める（OSM 山頂は `osm-peaks.json.gz`） |
| `downloadUrl` | string | 必須 | データ本体の HTTPS URL |
| `sha256` | string | 必須 | gzip ファイルの SHA-256（小文字の 16 進数 64 文字） |
| `sizeBytes` | integer | 必須 | gzip ファイルのバイト数 |
| `uncompressedSizeBytes` | integer | 必須 | 展開後の JSON のバイト数 |
| `pointCount` | integer | 必須 | 地点の件数。正の整数で、展開した配列の件数と同じ |
| `sourceTimestamp` | string | 任意 | 元データ全体の基準日時（UTC、ISO 8601）。不明なら省略 |
| `latestPointTimestamp` | string または null | 任意 | 収録地点の元データ上の最終編集日時の最大値。現地調査日や生成日時ではない。`sourceTimestamp` があるときは、それより後にしない |
| `sourceUrl` | string | 任意 | 元データの URL |
| `license` | string | 必須 | 利用条件。OSM 由来は `ODbL-1.0` |
| `attribution` | string | 必須 | 表示すべき出典。OSM 由来は `© OpenStreetMap contributors` |
| `supplementSources` | object | 任意 | 補足（日本百名山のタグなど）の出典。出典 ID ごとに `source`・`license`・`attribution`・`changes` を書く |

- `pointCount` と `latestPointTimestamp` は、地点データだけで使う項目。ほかの種類の件数や日時は入れない。
- 機械検証用の [manifest の版5 のスキーマ](../../schemas/manifest-v5.schema.json)（JSON Schema Draft 2020-12）では、地点データを `#/$defs/pointManifest` で確かめる。日時の前後関係や、本体の大きさ・ハッシュ・件数はスキーマでは確かめられないので、別に確かめる。
- 共通の項目の制約は [ルート README の「manifest.json」](../../README.md#manifestjson) にある。

## データ本体

gzip で圧縮した UTF-8（BOM なし）の JSON 配列で、1 要素が 1 地点。配列には 1 地点以上を入れる。gzip のヘッダーには日時とファイル名を入れない。山頂以外の地点（駐車場・登山口など）も入れられる共通の形式。

次は、上の Release のデータの先頭の 1 件。

```json
{"id":"165028676","type":"peak","osmId":165028676,"name":"奥穂高岳","latitude":36.289206,"longitude":137.6479807,"elevationM":3190.0,"nameReading":"おくほたかだけ","aliases":[],"wikipediaUrl":"https://ja.wikipedia.org/wiki/%E7%A9%82%E9%AB%98%E5%B2%B3","wikidataUrl":"https://www.wikidata.org/wiki/Q31702740"}
```

OpenStreetMap と関連付けていない地点は、たとえば次のように書く（説明用の架空のデータ）。

```json
{"id":"example-tower","type":"landmark","name":"展望塔","latitude":35.0,"longitude":139.0}
```

| 項目 | 型 | 必須 | 内容 |
|---|---|---|---|
| `id` | string | 必須 | データセット内で一意な、空でない ID。取得元を表す接頭辞は付けない。OSM 由来の山頂は OSM ノード ID の文字列（「ID とデータセット」を参照） |
| `type` | string または null | 任意 | 地点の種別（「地点の種別」を参照） |
| `osmId` | integer または null | 任意 | 関連する OSM ノード ID（正の整数）。OSM と関連付けていない地点には無い |
| `name` | string | 必須 | 表示名。空にしない |
| `latitude` | number | 必須 | WGS 84 の緯度（度）。−90 以上 90 以下 |
| `longitude` | number | 必須 | WGS 84 の経度（度）。−180 以上 180 以下 |
| `elevationM` | number または null | 任意 | 地点そのものの標高（m）。建物の高さなどは入れない。負の値もありうる |
| `nameReading` | string または null | 任意 | 表示名の読み |
| `aliases` | string の配列 または null | 任意 | 別名。空の名前・表示名と同じ名前・重複は含めない |
| `wikipediaUrl` | string または null | 任意 | Wikipedia 記事の HTTPS URL。記事の本文は入れない |
| `wikidataUrl` | string または null | 任意 | Wikidata 項目の HTTPS URL |
| `tags` | string の配列 または null | 任意 | 分類ラベル（例: `日本百名山`）。空の文字列・重複は含めない |
| `graphic` | object または null | 任意 | 地点に表示する画像（「地点ごとのグラフィック」を参照） |

- 必須の項目には `null` を入れない。任意の項目は、省略と `null` を同じ意味に扱う。`aliases`・`tags` の省略は空の配列、`type` の省略は種別不明、`graphic` の省略は画像なし、`graphic.scale` の省略は `1.0` の意味。
- 定義していない項目は置かない。数値に `NaN` や無限大は使わない。名前・読み・別名・タグの前後に空白を入れない。
- 項目の取り方・配列の並び順・同じ地点が重なったときの扱いは、データごとの README に書く（OSM 山頂は OSM ノード ID の昇順）。
- 機械検証用の [地点データの版5 のスキーマ](../schemas/data-v5.schema.json)（JSON Schema Draft 2020-12）は、展開後の配列に当てはめる。URL の `format` の検査を有効にして使う。`id` の重複、別名と表示名の一致、数値が有限かは、スキーマでは確かめられないので別に確かめる。

### 地点の種別

`type` は 1 地点に 1 つの文字列で、小文字の英字で始め、続きには小文字の英数字と下線を使う。今決めている値は次のとおり。

| 値 | 意味 |
|---|---|
| `peak` | 山頂（火山の山頂を含む） |
| `parking` | 駐車場 |
| `trailhead` | 登山口 |
| `landmark` | 塔などの目印 |
| `mountain_hut` | 山小屋・避難小屋 |
| `campsite` | キャンプ場・テント場 |
| `water_source` | 水場 |
| `toilet` | トイレ |
| `viewpoint` | 展望地点 |
| `pass` | 峠 |
| `junction` | 登山道の分岐 |

- 種別は、版を変えずに増やすことがある。増やしたときは、この表を更新する。
- 読み手は、知らない種別を種別不明として扱い、その地点の名前や位置は使う。知らない種別があっても、データ全体を捨てない。
- 新しく作る山頂のデータには、必ず `peak` を書く。

### ID とデータセット

- `id` は、名前や位置が変わっても変えない。消した地点の `id` を、別の地点に使い回さない。
- OSM 山頂では、ノード番号を先頭に 0 を付けない文字列にする。手で作る地点には、`example-tower` のような短い値を付ける。
- 地点は「データセットの名前と `id` の組」で見分ける。1 つのデータセットを複数のファイルに分けるときも、データセット全体で `id` を一意にする。別のデータセットとは、同じ `id` が重なってもよい。

### 地点ごとのグラフィック

`graphic` は、地点に表示する画像を SVG の本文として持つ。画像を別に取りに行く必要はなく、manifest の `sizeBytes` と `sha256` で、SVG を含むデータ全体を確かめられる。同じ SVG を複数の地点で使うときも、地点ごとに本文を書く。

```json
{
  "id": "fuji",
  "type": "peak",
  "name": "富士山",
  "latitude": 35.3606,
  "longitude": 138.7274,
  "graphic": {
    "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><path d=\"M10 90 L50 10 L90 90 Z\"/></svg>",
    "scale": 1.5
  }
}
```

| 項目 | 型 | 必須 | 内容 |
|---|---|---|---|
| `svg` | string | 必須 | SVG の本文。UTF-8 で 100,000 バイト以内。引用符や改行は JSON の文字列としてエスケープする |
| `scale` | number または null | 任意 | 標準の表示の大きさに対する、幅と高さの倍率。0 より大きい有限の数。省略すると `1.0` |

- SVG は静止画で、SVG の名前空間と有効な `viewBox` を必ず持つ。縦横の大きさや比は決めていない。表示するときは、`viewBox` の縦横比を保つ。
- 使えるのは、パス・基本図形・グループ・グラデーション・クリッピング。文字はパスにし、スクリプト・アニメーション・外部参照・埋め込み画像・CSS・フォントは使わない。許す要素と属性の一覧は [graphics.py](../osm-peaks/scripts/graphics.py) にある。
- 読み手は、SVG を表示できないときや描画に失敗したときに、`type` に合う標準のアイコンなどに切り替えるとよい。
- SVG の著作権と利用条件は地点データとは別に決まる。出典と利用条件は、データごとの README に書く。

## 出典とライセンス

利用条件と出典は、manifest の `license` と `attribution` に書く。使うときは `attribution` の出典を表示する（OSM 由来なら「© OpenStreetMap contributors」）。`tags` などの補足を付けた場合、その出典と利用条件は manifest の `supplementSources` に書く。

## 変更履歴

### 版5（2026-10-07、#36）

版4 との違いは次のとおり。

| | 版4 | 版5 |
|---|---|---|
| データ本体の版 | manifest の版と同じ番号（別の項目は無い） | `dataSchemaVersion` に別に書く |
| 件数の項目 | `mountainCount` | `pointCount` |
| 最終編集日時の項目 | `latestMountainTimestamp` | `latestPointTimestamp` |
| `sourceTimestamp` | 必須 | 任意 |
| 足した manifest の項目 | | `dataSchemaVersion`・`name`・`supplementSources` |
| 足した地点の項目 | | `id`（必須）・`type`・`tags`・`graphic` |
| `osmId` | 必須 | 任意（OSM 由来でない地点を入れられるように） |

- 地点の `osmId`・`name`・`latitude`・`longitude`・`elevationM`・`nameReading`・`aliases`・`wikipediaUrl`・`wikidataUrl` は、名前も意味も版4 と同じ。
- 版4 だけを知る読み手でも、知らない項目を無視すれば版5 の地点を読める。ただし、山頂以外の種別（`type` が `peak` 以外）や `osmId` の無い地点が混ざると、版4 の前提が成り立たない。
- OSM 山頂では、版5 に合わせて Release のタグを `peaks-*` から `osm-peaks-*` に、データ本体のファイル名を `japan-mountains.json.gz` から `osm-peaks.json.gz` に変えた。同じ元データ（2026-10-06 版）から作った版4 と版5 の Release は、同じ 14,035 件で、山ごとの値も同じ。

### 版4（2026-10-03、#26）

- manifest に `downloadUrl` を足した。地点の項目は版3 と同じ。

### 版3

- manifest に `latestMountainTimestamp` を足した。地点の項目は版2 と同じ。

### 版2

- 地点に `nameReading`・`aliases`・`wikipediaUrl`・`wikidataUrl` を足した。名前・位置・標高はそのまま。

### 版1

- manifest は `schemaVersion`・`version`・`fileName`・`sha256`・`sizeBytes`・`uncompressedSizeBytes`・`mountainCount`・`sourceTimestamp` と出典の項目、地点は `osmId`・`name`・`latitude`・`longitude`・`elevationM` の 5 つ。

版1〜3 は #6（2026-10-01 マージ）の中で作られ、Release は残っていない。版1〜4 の詳細と根拠は [版4](data-v4.md#変更履歴) にある。

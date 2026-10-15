# 地点データフォーマット

[地点データの README に戻る](../README.md)

## 概要

地点 JSON の形式と各版の変更を説明する。取得と検証に使う manifest は [manifest フォーマット](../../formats/manifest.md) を参照する。

Release のタグ・ファイル名・置き場所など、データごとの配り方は各データの README（例: [OSM 山頂](../osm-peaks/README.md#置き場所)）に書く。

## 版について

地点 JSON の版は本体の `schemaVersion` で確認する。版5以前の本体は配列のため、manifest の `dataSchemaVersion` を使う。版6では本体の `schemaVersion` と manifest の `dataSchemaVersion` を一致させる。現行版は `6`。manifest 自体の版とは独立して管理する。manifest 版4以前は `dataSchemaVersion` がなく、地点データの版は manifest の `schemaVersion` と同じ。

版内の変更は `版1.1` のように小数点以下で区別する。小数点以下は文書上の番号で、JSON の版の値には含めない。

項目表の「版の履歴」は、フィールドの追加や変更があった版を示す。スキーマは版ごとに分けて管理する。

| 形式の版 | 機械検証用スキーマ | 規格 |
|---|---|---|
| 6 | [data-v6.schema.json](../schemas/data-v6.schema.json) | JSON Schema Draft 2020-12 |
| 5 | [data-v5.schema.json](../schemas/data-v5.schema.json) | JSON Schema Draft 2020-12 |
| 1〜4 | スキーマなし。変更履歴を参照 | — |

## 項目

gzip で圧縮した UTF-8（BOM なし）の JSON object。`points` に 1 地点以上の地点配列を格納する。版5以前のトップレベルは地点配列だった。gzip のヘッダーには日時とファイル名を入れない。山頂以外の地点（駐車場・登山口など）も入れられる共通の形式。

| 項目 | 型 | 必須 | 内容 | 版の履歴 |
|---|---|---|---|---|
| `schemaFormat` | string | 必須 | JSON の形式の識別子。`yamamuki-points` 固定。他の形式と区別する | [版6] 追加 |
| `schemaVersion` | integer | 必須 | JSON の形式の版。現行版は `6`。manifest 自体の `schemaVersion` とは独立 | [版6] 追加 |
| `metadata` | object | 任意 | 文書全体の共通情報。[metadata フォーマット](../../formats/metadata.md) を参照。配布 manifest に同じ object をコピーする | [版6] 追加 |
| `points` | array | 必須 | 地点の配列。1 地点以上 | [版6] 追加 |

### 共通情報（metadata）

共通項目の型・構造・制約は [metadata フォーマット](../../formats/metadata.md) を参照する。地点 JSON では `metadata` と各子項目は任意。生成時は実際の地点数を `pointCount` に設定する。各地点の `points[].tags` は文字列配列で、共通情報の `metadata.tags` とは用途が異なる。

### 地点ごとの項目

`id` と `relatedIds` の値の制約は [地点の共通項目スキーマ](../schemas/point-common-v1.schema.json) を参照する。必須・省略・`null` の扱いは各形式で定める。

以下は `points` 配列の各要素に指定する。

| 項目 | 型 | 必須 | 内容 | 版の履歴 |
|---|---|---|---| --- |
| `id` | string | 必須 | データセットをまたいで一意な、前後に空白のない空でない ID。接頭辞を含む ID の形式・作り方は各データセットの文書で定める | [版5] 追加 |
| `type` | string または null | 任意 | 地点の種別（「地点の種別」を参照） | [版5] 追加 |
| `osmId` | integer または null | 廃止 | 版5以前の OSM ノードへの関連付け。版6では `relatedIds` を使う | [版1] 追加<br>[版5] 任意化<br>[版6] 廃止 |
| `relatedIds` | string の配列 | 任意 | 関連する他の地点のグローバル ID。省略・空配列は関連付けなし。重複・自分自身の ID は指定しない | [版6] 追加 |
| `name` | string | 必須 | 表示名。空にしない | [版1] 追加 |
| `nameReading` | string・string の配列・null | 任意 | 表示名の読み。複数ある場合は重複のない1件以上の文字列配列とし、順序は記載順を保つ。空文字・前後の空白は使わない | [版2] 追加<br>[版5] 任意化<br>[版6] 複数の読みに対応 |
| `latitude` | number | 必須 | WGS 84 の緯度（度）。−90 以上 90 以下 | [版1] 追加 |
| `longitude` | number | 必須 | WGS 84 の経度（度）。−180 以上 180 以下 | [版1] 追加 |
| `elevationM` | number または null | 任意 | 地点そのものの標高（m）。建物の高さなどは入れない。負の値もありうる | [版1] 追加<br>[版5] 任意化 |
| `aliases` | object の配列 または null | 任意 | 別名と読み（「別名のよみがな」を参照）。空の名前・表示名と同じ名前・名前の重複は含めない | [版2] 文字列配列を追加<br>[版5] 任意化<br>[版6] object 配列に変更 |
| `wikipediaUrl` | string または null | 任意 | Wikipedia 記事の HTTPS URL。記事の本文は入れない | [版2] 追加<br>[版5] 任意化 |
| `wikidataUrl` | string または null | 任意 | Wikidata 項目の HTTPS URL | [版2] 追加<br>[版5] 任意化 |
| `tags` | string の配列 または null | 任意 | 分類ラベル（例: `日本百名山`）。空の文字列・重複は含めない | [版5] 追加 |
| `provenance` | object または null | 任意 | 地点の取得元・元資料の分類・標高の出典（「地点ごとの出典情報」を参照） | [版5.1] 追加 |
| `graphic` | object または null | 任意 | 地点に表示する画像（「地点ごとのグラフィック」を参照） | [版5] 追加 |

- 必須の項目には `null` を入れない。任意の項目は、省略と `null` を同じ意味に扱う。`aliases`・`tags` の省略は空の配列、`type` の省略は種別不明、`graphic` の省略は画像なし、`graphic.scale` の省略は `1.0` の意味。
- 定義していない項目は置かない。数値に `NaN` や無限大は使わない。名前・読み・別名・タグの前後に空白を入れない。
- 配列の並び順は保証しない。地点の識別には `id` を使い、表示や処理に必要な順番は利用する側で並べ替える。
- 項目の取り方・同じ地点が重なったときの扱いは、データごとの README に書く。
- 概要表のスキーマは、展開後の JSON 全体に当てはめる（版6は object、版5は配列）。URL の `format` の検査を有効にして使う。`id` の重複、別名と表示名の一致、数値が有限かは、スキーマでは確かめられないので別に確かめる。

#### 地点の種別

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

- 種別は JSON の `dataSchemaVersion` を変えずに増やすことがある。追加時はこの表を更新し、文書上の `版5.1` のような番号・変更日・対応 PR を変更履歴に記載する。
- 読み手は、知らない種別を種別不明として扱い、その地点の名前や位置は使う。知らない種別があっても、データ全体を捨てない。
- 新しく作る山頂のデータには、必ず `peak` を書く。

#### ID とデータセット

- すべての地点データで、`id` の維持は努力義務とし、永続性を保証しない。名前や位置が変わっても可能な限り同じ ID を維持するが、取得元や生成方法によっては、同じ地点でも ID が変わる場合がある。
- 消した地点の `id` を、別の地点に意図的に使い回さない。ID の作り方と維持方針は、各データセットの文書に記載する。
- 地点は `id` だけで見分ける。異なる地点に、データセットをまたいで同じ `id` を使わない。取得元ごとの接頭辞で名前空間を分ける。同じ元の地点を複数のデータセットや更新版に収録する場合は、同じ `id` を使う。

#### 地点間の関連付け

`relatedIds` に、関連する他の地点のグローバル ID を列挙する。OSM に限らず、GSI・手動作成などの異なるデータセットの地点を関連付けられる。例として GSI の地点から `"relatedIds": ["osm:123456", "custom:sample"]` と指定する。

関連付けは一方向。相手側の関連付け、同一地点であること、名称や標高の一致を自動では保証しない。関連先を含むデータセットが読み込まれていなくても保持する。自分自身の ID と重複は指定しない。

OSM の地点自身の ID は `osm:<ノード番号>` とし、自分自身を `relatedIds` に入れない。版5以前の `osmId` は旧版の読み込みでのみ扱い、新規出力には含めない。

#### 地点ごとの出典情報

`provenance` は、その地点の取得元・元資料の分類・標高の出典を記録するオブジェクト。省略または `null` は、地点ごとの出典情報がないことを表す。オブジェクト内の各項目は任意で、確認できる情報だけを記録する。空のオブジェクト `{}` も指定できる。

| 項目 | 型 | 必須 | 内容 | 版の履歴 |
|---|---|---|---|---|
| `dataSource` | string | 任意 | 地点を採用した取得元を表す識別子 | [版5.1] 追加 |
| `classification` | string | 任意 | 元資料での地点の分類。数値の分類コードも文字列で記録する | [版5.1] 追加 |
| `elevationSource` | string | 任意 | `elevationM` に採用した標高の取得元を表す識別子 | [版5.1] 追加 |

各項目の省略は、その情報が記録されていないことを表す。項目を指定する場合は、`null`・空文字列・前後に空白のある文字列は使わない。定義していない項目は追加しない。識別子・分類コードの値と意味、不明・未取得の場合の値は、各データセットの文書で定義する。GSI 山頂版の値は [地点ごとの出典情報](../gsi-peaks/README.md#地点ごとの出典情報)を参照する。

`classification` は元資料の分類を保持するための値であり、共通の地点種別 `type` とは別に扱う。`dataSource` と `elevationSource` は異なる取得元を指定できる。資料全体の出典・ライセンスは manifest に記載し、`provenance` には地点ごとの識別子を記録する。

次は、ベクトルタイルの山名注記と DEM1A の標高を採用した場合の `provenance` の例。

```json
{
  "dataSource": "vector",
  "classification": "810",
  "elevationSource": "dem1a_png"
}
```

#### 地点ごとのグラフィック

`graphic` は、地点に表示する画像を SVG の本文として持つ。画像を別に取りに行く必要はなく、manifest の `sizeBytes` と `sha256` で、SVG を含むデータ全体を確かめられる。同じ SVG を複数の地点で使うときも、地点ごとに本文を書く。

| 項目 | 型 | 必須 | 内容 | 版の履歴 |
|---|---|---|---|---|
| `svg` | string | 必須 | SVG の本文。UTF-8 で 100,000 バイト以内。引用符や改行は JSON の文字列としてエスケープする | [版5] 追加 |
| `scale` | number または null | 任意 | 標準の表示の大きさに対する、幅と高さの倍率。0 より大きい有限の数。省略すると `1.0` | [版5] 追加 |

- SVG は静止画で、SVG の名前空間と有効な `viewBox` を必ず持つ。縦横の大きさや比は決めていない。表示するときは、`viewBox` の縦横比を保つ。
- 使えるのは、パス・基本図形・グループ・グラデーション・クリッピング。文字はパスにし、スクリプト・アニメーション・外部参照・埋め込み画像・CSS・フォントは使わない。許す要素と属性の一覧は [graphics.py](../osm-peaks/scripts/graphics.py) にある。
- 読み手は、SVG を表示できないときや描画に失敗したときに、`type` に合う標準のアイコンなどに切り替えるとよい。
- SVG の著作権と利用条件は地点データとは別に決まる。出典と利用条件は、データごとの README に書く。

## 読み込みと検証

現行形式では、各 object に未知の項目を許可し、読み手はそれを解釈しない。既知の項目の型・必須条件・重複などの制約は維持する。

配列内の各地点を項目表に従って読み込む。対応する形式の版は manifest の `dataSchemaVersion` で確認する。manifest とデータ本体の取得・サイズ・ハッシュ・件数の照合は [地点データの取得・利用手順](../README.md#データを取得して利用する) を参照する。

JSON Schema では確認できない ID の重複や別名と表示名の一致も検証する。山頂だけを使う場合は `type` が `peak` の地点を選び、未知の種別は種別不明として扱う。

### 出典とライセンス

利用条件と出典は、地点 JSON の `metadata.sources` と manifest の `metadata.sources` に出典ごとに記載する。利用時には各出典の `license` に従い、`attribution` を表示する。元データと補足の出典は同じ一覧に保持する。

地点名と別名の `nameReading` は、1つの読みを文字列、複数の読みを文字列配列で記録できる。例えば白馬岳は `"nameReading": ["しろうまだけ", "はくばだけ"]` とする。読みの空配列・重複・空文字・前後の空白は認めない。`metadata.tags` の `nameReading` も同じく文字列または文字列配列を使う。

## JSON の例

### 版6

任意項目と子 object の項目を含む、形式説明用の架空の地点。URL は説明用の値。

```json
{
  "schemaFormat": "yamamuki-points",
  "schemaVersion": 6,
  "metadata": {
    "sources": [
      {
        "source": "見本の地名資料 https://example.com/source",
        "license": "CC BY 4.0 https://creativecommons.org/licenses/by/4.0/",
        "attribution": "見本資料の著作者",
        "changes": "地点名・座標・標高を抽出",
        "lastestTimeStamp": "2026-10-14T00:00:00Z",
        "name": "元資料"
      },
      {
        "source": "見本の山名一覧 https://example.com/supplement",
        "license": "CC0-1.0",
        "attribution": "見本の補足資料の作成者",
        "changes": "読み・別名・タグを追加",
        "lastestTimeStamp": "2026-10-14T01:00:00Z",
        "name": "補足資料"
      }
    ],
    "tags": [
      {
        "name": "日本百名山",
        "nameReading": "にほんひゃくめいざん"
      }
    ],
    "pointCount": 1
  },
  "points": [
    {
      "id": "osm:123456",
      "type": "peak",
      "name": "見本山",
      "nameReading": ["みほんやま", "みほんさん"],
      "latitude": 35.0,
      "longitude": 139.0,
      "elevationM": 1000.0,
      "aliases": [
        {
          "name": "見本岳",
          "nameReading": "みほんだけ"
        }
      ],
      "wikipediaUrl": "https://example.com/wiki/sample-mountain",
      "wikidataUrl": "https://example.com/wikidata/sample-mountain",
      "graphic": {
        "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\"><circle cx=\"5\" cy=\"5\" r=\"4\"/></svg>",
        "scale": 1.5
      },
      "tags": [
        "日本百名山"
      ],
      "provenance": {
        "dataSource": "osm",
        "classification": "natural=peak",
        "elevationSource": "osm:ele"
      },
      "relatedIds": [
        "gsi-vector:sample",
        "custom:sample"
      ]
    }
  ]
}
```


各例は形式説明用の架空の地点。版5の例は版5.1の任意項目も含む。版5以前のデータ本体は JSON 配列で、版2〜4の項目は同じ。版5以前の版の番号は JSON 本体に含めない。

### 版5

```json
[
  {
    "id": "123456",
    "type": "peak",
    "osmId": 123456,
    "name": "見本山",
    "nameReading": "みほんやま",
    "latitude": 35.0,
    "longitude": 139.0,
    "elevationM": 1000,
    "aliases": [
      "見本岳"
    ],
    "wikipediaUrl": "https://example.com/wiki/sample-mountain",
    "wikidataUrl": "https://example.com/wikidata/sample-mountain",
    "tags": [
      "日本百名山"
    ],
    "graphic": {
      "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\"><circle cx=\"5\" cy=\"5\" r=\"4\"/></svg>",
      "scale": 1.5
    },
    "provenance": {
      "dataSource": "osm",
      "classification": "natural=peak",
      "elevationSource": "osm:ele"
    }
  }
]
```

#### OSM と関連付けない地点と SVG 画像

```json
[
  {
    "id": "example-tower",
    "type": "landmark",
    "name": "展望塔",
    "latitude": 35.0,
    "longitude": 139.0,
    "graphic": {
      "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><path d=\"M10 90 L50 10 L90 90 Z\"/></svg>",
      "scale": 1.5
    },
    "nameReading": "てんぼうとう",
    "elevationM": 1000,
    "aliases": [
      "見本の塔"
    ],
    "wikipediaUrl": "https://example.com/wiki/sample-tower",
    "wikidataUrl": "https://example.com/wikidata/sample-tower",
    "tags": [
      "展望施設"
    ],
    "provenance": {
      "dataSource": "manual",
      "classification": "tower",
      "elevationSource": "survey"
    }
  }
]
```

### 版4

```json
[
  {
    "osmId": 123456,
    "name": "見本山",
    "nameReading": "みほんやま",
    "latitude": 35.0,
    "longitude": 139.0,
    "elevationM": 1000,
    "aliases": [
      "見本岳"
    ],
    "wikipediaUrl": "https://example.com/wiki/sample-mountain",
    "wikidataUrl": "https://example.com/wikidata/sample-mountain"
  }
]
```

### 版3

```json
[
  {
    "osmId": 123456,
    "name": "見本山",
    "nameReading": "みほんやま",
    "latitude": 35.0,
    "longitude": 139.0,
    "elevationM": 1000,
    "aliases": [
      "見本岳"
    ],
    "wikipediaUrl": "https://example.com/wiki/sample-mountain",
    "wikidataUrl": "https://example.com/wikidata/sample-mountain"
  }
]
```

### 版2

```json
[
  {
    "osmId": 123456,
    "name": "見本山",
    "nameReading": "みほんやま",
    "latitude": 35.0,
    "longitude": 139.0,
    "elevationM": 1000,
    "aliases": [
      "見本岳"
    ],
    "wikipediaUrl": "https://example.com/wiki/sample-mountain",
    "wikidataUrl": "https://example.com/wikidata/sample-mountain"
  }
]
```

### 版1

```json
[
  {
    "osmId": 123456,
    "name": "見本山",
    "latitude": 35.0,
    "longitude": 139.0,
    "elevationM": 1000
  }
]
```

### 別名のよみがな

`aliases` は、別名ごとの object を持つ配列。名前と読みを同じ要素で保持する。版5以前は文字列配列だった。版6で読みがない別名も `{"name":"別名"}` の形で記載する。

| 項目 | 型 | 必須 | 内容 | 版の履歴 |
|---|---|---|---|---|
| `aliases[].name` | string | 必須 | 別名。空文字・前後の空白・表示名と同じ名前・名前の重複は使わない | [版6] 追加 |
| `aliases[].nameReading` | string・string の配列・null | 任意 | 別名の読み。省略・`null` は読みなし。空文字・前後の空白は使わない | [版6] 追加 |

```json
{
  "aliases": [{"name": "柴安嵓", "nameReading": "しばやすぐら"}]
}
```

## 変更履歴

### 版6（2026-10-14）

- `osmId` を廃止し、複数のデータセットの地点へ関連付けられる `relatedIds` 配列を追加する。

- 地点 ID の名前空間をグローバルに変更する。OSM の地点は `osm:<ノード番号>` とする。版5以前の数値文字列 ID とは異なる。

- 地点と補足の `id`・`relatedIds` の値の制約を共通スキーマにまとめる。ID は前後の空白を認めない。

- 必須の `schemaFormat: "yamamuki-points"` を追加し、形式を識別する。未公開の版6に統合する。

- トップレベルを地点配列から object に変更し、従来の配列を必須の `points` に格納する。版5の読み込み処理では版6を読めないため、利用前に対応が必要。
- 本体に必須の `schemaVersion: 6` を追加する。manifest の `dataSchemaVersion` と一致させる。manifest 自体の `schemaVersion` は `6` とする。
- `metadata` に任意の `pointCount` を追加し、実際の地点数を保持する。配布 manifest の `metadata` 内の同名項目と一致させる。出典ごとの最終更新日時は `metadata.sources[].lastestTimeStamp` に保持し、不明なら省略する。
- `metadata` に任意の `sources` を追加し、元データと補足の出典情報を出典名ごとに地点 JSON に保持する。配布 manifest の `metadata` 内の同名項目と一致させる。
- `metadata` に任意の `tags` 配列 を追加し、タグ共通の `nameReading` を保持する。タグ情報も manifest の `metadata` に保持する。各地点の `points[].tags` は従来の文字列配列を維持する。
- `aliases` を文字列配列から object 配列に変更し、各要素の `name` と任意の `nameReading` に別名と読みをまとめる。
- 機械検証用の [data-v6.schema.json](../schemas/data-v6.schema.json) を追加し、版5のスキーマも残す。

### 版5.1（2026-10-14）

- 任意の `provenance` を追加し、取得元・元資料の分類・標高の出典を地点ごとに記録できるようにする。既存の地点 JSON はそのまま読める。manifest の `dataSchemaVersion` は `5` のままとする。

### 版5（2026-10-07、#36）

地点 JSON の版4 との違いは次のとおり。

| | 版4 | 版5 |
|---|---|---|
| 足した地点の項目 | | `id`（必須）・`type`・`tags`・`graphic` |
| `osmId` | 必須 | 任意（OSM 由来でない地点を入れられるように） |

- 地点の `osmId`・`name`・`latitude`・`longitude`・`elevationM`・`nameReading`・`aliases`・`wikipediaUrl`・`wikidataUrl` は、名前も意味も版4 と同じ。
- 版4 だけを知る読み手でも、知らない項目を無視すれば版5 の地点を読める。ただし、山頂以外の種別（`type` が `peak` 以外）や `osmId` の無い地点が混ざると、版4 の前提が成り立たない。
- OSM 山頂では、版5 に合わせて Release のタグを `peaks-*` から `osm-peaks-*` に、データ本体のファイル名を `japan-mountains.json.gz` から `osm-peaks.json.gz` に変えた。同じ元データ（2026-10-06 版）から作った版4 と版5 の Release は、同じ 14,035 件で、山ごとの値も同じ。

版1〜3 は、最初の生成・公開処理（#6、2026-10-01 マージ）の中で順に作られた。版1〜3 の Release は今は残っていない。

manifest の変更は [manifest の変更履歴](../../formats/manifest.md#変更履歴) を参照する。

### 版4（2026-10-03、#26）

- 地点 JSON の項目は版3 と同じ。
- `osmId`・`name`・`latitude`・`longitude`・`elevationM`・`nameReading`・`aliases`・`wikipediaUrl`・`wikidataUrl` の 9 項目をすべての地点に記載し、省略しない。
- `osmId` は必須の整数で、データ内で重複しない。標高・読み・解説リンクが不明な場合は `null`、別名がない場合は `[]` を使う。
- `id`・`type`・`tags`・`graphic` は持たない。

### 版3（2026-10-01、#6）

- 地点 JSON の項目は版2 と同じ。

### 版2（2026-10-01、#6）

- 地点に `nameReading`・`aliases`・`wikipediaUrl`・`wikidataUrl` を足した。名前・位置・標高はそのまま。
- リンク先の記事の本文は入れない（見るには通信が要る）。

### 版1（2026-10-01、#6）

- `osmId`・`name`・`latitude`・`longitude`・`elevationM` の 5 項目。
- 版1 の仕様を書いた資料は残っていない。上の項目は、#6 の検査処理（`peaks/scripts/check_release.py` の `validate`）が版1 に求める項目から読み取ったもの。

タグ共通の読みは地点ごとに繰り返さず、地点文書の`metadata` 内の任意の `tags` に記載する。地点の `tags` は文字列配列のまま。

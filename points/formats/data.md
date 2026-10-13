# 地点データフォーマット

[地点データの README に戻る](../README.md)

## 概要

地点 JSON の形式と各版の変更を説明する。取得と検証に使う manifest は [manifest フォーマット](../../formats/manifest.md) を参照する。

Release のタグ・ファイル名・置き場所など、データごとの配り方は各データの README（例: [OSM 山頂](../osm-peaks/README.md#置き場所)）に書く。

## 版について

地点 JSON の版は manifest の `dataSchemaVersion` で確認する。現行版は `5`。manifest 自体の版とは独立して管理する。manifest 版4以前は `dataSchemaVersion` がなく、地点データの版は manifest の `schemaVersion` と同じ。

版内の変更は `版1.1` のように小数点以下で区別する。小数点以下は文書上の番号で、JSON の版の値には含めない。

項目表の「版の履歴」は、フィールドの追加や変更があった版を示す。スキーマは版ごとに分けて管理する。

| 形式の版 | 機械検証用スキーマ | 規格 |
|---|---|---|
| 5 | [data-v5.schema.json](../schemas/data-v5.schema.json) | JSON Schema Draft 2020-12 |
| 1〜4 | スキーマなし。変更履歴を参照 | — |

## 項目

gzip で圧縮した UTF-8（BOM なし）の JSON 配列で、1 要素が 1 地点。配列には 1 地点以上を入れる。gzip のヘッダーには日時とファイル名を入れない。山頂以外の地点（駐車場・登山口など）も入れられる共通の形式。

| 項目 | 型 | 必須 | 内容 | 版の履歴 |
|---|---|---|---| --- |
| `id` | string | 必須 | データセット内で一意な、空でない ID。取得元を表す接頭辞は付けない。ID の作り方は各データセットの文書を参照 | [版5] 追加 |
| `type` | string または null | 任意 | 地点の種別（「地点の種別」を参照） | [版5] 追加 |
| `osmId` | integer または null | 任意 | 関連する OSM ノード ID（正の整数）。版1〜4では必須だったが、版5から任意。OSM と関連付けていない地点には無い | [版1] 追加<br>[版5] 任意化 |
| `name` | string | 必須 | 表示名。空にしない | [版1] 追加 |
| `latitude` | number | 必須 | WGS 84 の緯度（度）。−90 以上 90 以下 | [版1] 追加 |
| `longitude` | number | 必須 | WGS 84 の経度（度）。−180 以上 180 以下 | [版1] 追加 |
| `elevationM` | number または null | 任意 | 地点そのものの標高（m）。建物の高さなどは入れない。負の値もありうる | [版1] 追加<br>[版5] 任意化 |
| `nameReading` | string または null | 任意 | 表示名の読み | [版2] 追加<br>[版5] 任意化 |
| `aliases` | string の配列 または null | 任意 | 別名。空の名前・表示名と同じ名前・重複は含めない | [版2] 追加<br>[版5] 任意化 |
| `wikipediaUrl` | string または null | 任意 | Wikipedia 記事の HTTPS URL。記事の本文は入れない | [版2] 追加<br>[版5] 任意化 |
| `wikidataUrl` | string または null | 任意 | Wikidata 項目の HTTPS URL | [版2] 追加<br>[版5] 任意化 |
| `tags` | string の配列 または null | 任意 | 分類ラベル（例: `日本百名山`）。空の文字列・重複は含めない | [版5] 追加 |
| `graphic` | object または null | 任意 | 地点に表示する画像（「地点ごとのグラフィック」を参照） | [版5] 追加 |

- 必須の項目には `null` を入れない。任意の項目は、省略と `null` を同じ意味に扱う。`aliases`・`tags` の省略は空の配列、`type` の省略は種別不明、`graphic` の省略は画像なし、`graphic.scale` の省略は `1.0` の意味。
- 定義していない項目は置かない。数値に `NaN` や無限大は使わない。名前・読み・別名・タグの前後に空白を入れない。
- 配列の並び順は保証しない。地点の識別には `id` を使い、表示や処理に必要な順番は利用する側で並べ替える。
- 項目の取り方・同じ地点が重なったときの扱いは、データごとの README に書く。
- 概要表のスキーマは、展開後の配列に当てはめる。URL の `format` の検査を有効にして使う。`id` の重複、別名と表示名の一致、数値が有限かは、スキーマでは確かめられないので別に確かめる。

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

- 種別は JSON の `dataSchemaVersion` を変えずに増やすことがある。追加時はこの表を更新し、文書上の `版5.1` のような番号・変更日・対応 PR を変更履歴に記載する。
- 読み手は、知らない種別を種別不明として扱い、その地点の名前や位置は使う。知らない種別があっても、データ全体を捨てない。
- 新しく作る山頂のデータには、必ず `peak` を書く。

### ID とデータセット

- すべての地点データで、`id` の維持は努力義務とし、永続性を保証しない。名前や位置が変わっても可能な限り同じ ID を維持するが、取得元や生成方法によっては、同じ地点でも ID が変わる場合がある。
- 消した地点の `id` を、別の地点に意図的に使い回さない。ID の作り方と維持方針は、各データセットの文書に記載する。
- 地点は「データセットの名前と `id` の組」で見分ける。1 つのデータセットを複数のファイルに分けるときも、データセット全体で `id` を一意にする。別のデータセットとは、同じ `id` が重なってもよい。

### OSM との関連付け

手動作成の地点でも任意の `osmId` を指定できる。`id` はデータセット内の地点を見分ける識別子、`osmId` は関連する OSM ノードへの参照として扱う。

実データでは確認した OSM ノード番号を指定する。`osmId` は正の整数とし、関連付けがない場合は省略または `null` とする。文字列は使わない。

手動作成の地点は OSM の関連先を変更・削除しても元の ID を維持する。関連付けだけでデータの自動統合・上書きは行わない。

`osmId` はノードの番号を表す。way・relation は同じ番号のノードと区別できないため、この項目には指定しない。それらとの関連付けが必要になった時点で種別の表し方を追加する。

### 地点ごとのグラフィック

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

配列内の各地点を項目表に従って読み込む。対応する形式の版は manifest の `dataSchemaVersion` で確認する。manifest とデータ本体の取得・サイズ・ハッシュ・件数の照合は [地点データの取得・利用手順](../README.md#データを取得して利用する) を参照する。

JSON Schema では確認できない ID の重複や別名と表示名の一致も検証する。山頂だけを使う場合は `type` が `peak` の地点を選び、未知の種別は種別不明として扱う。

### 出典とライセンス

利用条件と出典は、manifest の `license` と `attribution` に書く。使うときは `attribution` の出典を表示する（OSM 由来なら「© OpenStreetMap contributors」）。`tags` などの補足を付けた場合、その出典と利用条件は manifest の `supplementSources` に書く。各項目の意味は [manifest の仕様](../../formats/manifest.md) を参照する。

## JSON の例

各例は形式説明用の架空の地点。データ本体は JSON 配列で、版2〜4の項目は同じ。版の番号は JSON 本体に含めない。

### 版5

```json
[
  {
    "id": "123456",
    "type": "peak",
    "osmId": 123456,
    "name": "見本山",
    "latitude": 35.0,
    "longitude": 139.0,
    "elevationM": 1000,
    "nameReading": "みほんやま",
    "aliases": [],
    "wikipediaUrl": null,
    "wikidataUrl": null
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
    "latitude": 35.0,
    "longitude": 139.0,
    "elevationM": 1000,
    "nameReading": "みほんやま",
    "aliases": [],
    "wikipediaUrl": null,
    "wikidataUrl": null
  }
]
```

### 版3

```json
[
  {
    "osmId": 123456,
    "name": "見本山",
    "latitude": 35.0,
    "longitude": 139.0,
    "elevationM": 1000,
    "nameReading": "みほんやま",
    "aliases": [],
    "wikipediaUrl": null,
    "wikidataUrl": null
  }
]
```

### 版2

```json
[
  {
    "osmId": 123456,
    "name": "見本山",
    "latitude": 35.0,
    "longitude": 139.0,
    "elevationM": 1000,
    "nameReading": "みほんやま",
    "aliases": [],
    "wikipediaUrl": null,
    "wikidataUrl": null
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

## 変更履歴

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

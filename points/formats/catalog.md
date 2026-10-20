# 地点カタログフォーマット

[地点データの README に戻る](../README.md#公開データのカタログ)

## 概要

地点カタログの形式と各版の変更を説明する。Pages に掲載された地点データについて、取得先とダウンロード後の検証に必要な manifest を一覧する。

これはアプリなどが地点データを取得するための一覧。公開処理が各データの manifest と公開履歴を引き継ぐために使う管理ファイル `/catalog.json` とは別のファイル。

## 版について

地点カタログの版は `schemaVersion` で確認する。現行版は `2`。ほかの形式の版とは独立して管理する。

版内の変更は `版1.1` のように小数点以下で区別する。小数点以下は文書上の番号で、JSON の版の値には含めない。

項目表の「版の履歴」は、フィールドの追加や変更があった版を示す。スキーマは版ごとに分けて管理する。

| 形式の版 | 機械検証用スキーマ | 規格 |
|---|---|---|
| 2 | [catalog-v2.schema.json](../schemas/catalog-v2.schema.json) | JSON Schema Draft 2020-12 |
| 1 | [catalog-v1.schema.json](../schemas/catalog-v1.schema.json) | JSON Schema Draft 2020-12 |

## 項目

| 項目 | JSON の型 | 必須 | 内容・制約 | 版の履歴 |
|---|---|---|---|---|
| `schemaVersion` | integer | 必須 | カタログの形式の版。`2` | [版1] 追加 |
| `datasets` | array | 必須 | 公開済み地点データの一覧。該当するデータがなければ `[]` | [版1] 追加 |
| `datasets[].id` | string | 必須 | データセットの識別子（例: `osm-peaks`）。小文字の英字で始まり、小文字の英数字・下線・ハイフンで構成する。同じカタログ内で重複させない | [版1] 追加 |
| `datasets[].name` | string | 新規生成では必須 | データセットの表示名。空白以外の文字を含む。manifest の `name` から取得する。識別には `id` を使う | [版1] 追加 |
| `datasets[].manifestUrl` | string | 必須 | 公開する manifest の HTTPS URL | [版1] 追加 |
| `datasets[].manifest` | object | 新規生成では必須 | 公開する manifest のコピー。項目・型・必須条件は [manifest フォーマット](../../formats/manifest.md) を参照する | [版1] 追加 |

トップレベルと `datasets` の各要素に未知の項目を許可する。読み手は未知の項目を解釈しない。同梱する `manifest` にはデータ種別ごとの追加項目を含められる。スキーマ上は `name` と `manifest` を省略できるが、新規生成のカタログには両方を必ず含める。`id` の重複はスキーマだけでは検出できないため、別途確認する。

### 同梱する manifest

`datasets[].manifest` には公開する manifest をそのまま同梱する。項目・型・必須条件・形式の識別と版・データ本体の取得と検証は [manifest フォーマット](../../formats/manifest.md) に従う。manifest の版はカタログの版と独立して扱う。

## 読み込みと検証

現行形式では、各 object に未知の項目を許可し、読み手はそれを解釈しない。既知の項目の型・必須条件・重複などの制約は維持する。

1. 利用する配布先のカタログを取得し、カタログ自身の `schemaVersion` に対応していることを確認する。
2. 各データの同梱 manifest を [manifest フォーマット](../../formats/manifest.md) に従って確認し、対応するデータを取得・検証する。manifest を別途取得したり、アプリ側で URL を組み立てたりする必要はない。

非対応のデータはダウンロードせず、対応しているほかのデータを利用できる。通信や検証に失敗しても保存済みデータは消さない。

版の判定と検証は読み込み側アプリで実装する契約であり、このリポジトリでは必要な情報を生成・配布する。

URL の形式を検証するときは、JSON Schema の `format` の検査を有効にする。カタログと公開 manifest の一致は、スキーマとは別に確認する。

### 配布と更新

正式版の通常生成は Release の作成までで、Pages の manifest・カタログ・履歴を更新しない。正式版カタログは掲載済み情報であり、最新 Release をすべて収録する一覧ではない。正式版は利用する Release を選び、添付 manifest を固定して取得する。手動公開・復旧・削除では正式版側の Pages 情報も更新される。開発版（Pre-release）は Pages の公開・削除に合わせて更新する。

正式版の `points/catalog.json` には正式版だけ、開発版の `points/catalog-dev.json` には開発版だけを収録する。未公開のデータや地点以外のデータは含めない。利用する配布先はカタログの URL で区別し、アプリは他方へ自動で切り替えない。

Pages を更新する公開・削除時に、サイト全体の manifest 一覧から自動生成し、他の地点データと配布先を引き継ぐ。manifest と同じ Pages 配置で更新し、配置後に一致を確認する。生成物は git に含めない。公開 URL と削除の操作手順は [地点データの README](../README.md#公開データのカタログ) を参照する。

## JSON の例

取得先は説明用の URL。同梱する manifest の版はカタログの版と独立しており、この例では manifest 版6を使い、任意項目を含めて記載する。

### 版2

```json
{
  "schemaVersion": 2,
  "datasets": [
    {
      "id": "sample-points",
      "name": "見本の地点データ",
      "manifestUrl": "https://example.com/releases/example-v6/manifest.json",
      "manifest": {
        "schemaFormat": "yamamuki-manifest",
        "schemaVersion": 6,
        "dataSchemaFormat": "yamamuki-points",
        "dataSchemaVersion": 6,
        "name": "見本の地点データ",
        "version": "example-v6",
        "fileName": "sample-points.json.gz",
        "downloadUrl": "https://example.com/releases/example-v6/sample-points.json.gz",
        "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
        "sizeBytes": 1000,
        "uncompressedSizeBytes": 5000,
        "dataTimestamp": "2026-10-15T00:00:00Z",
        "metadata": {
          "sources": [
            {
              "source": "見本の地名資料 https://example.com/source",
              "license": "CC BY 4.0 https://creativecommons.org/licenses/by/4.0/",
              "attribution": "見本資料の著作者",
              "changes": "地点名・座標・標高を抽出",
              "latestTimestamp": "2026-10-14T00:00:00Z",
              "name": "元資料"
            },
            {
              "source": "見本の山名一覧 https://example.com/supplement",
              "license": "CC0-1.0",
              "attribution": "見本の補足資料の作成者",
              "changes": "読み・別名・タグを追加",
              "latestTimestamp": "2026-10-14T01:00:00Z",
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
        }
      }
    }
  ]
}
```

## 変更履歴

### 版2（2026-10-15）

- 版6の manifest を同梱できるようにし、出典情報を metadata で保持する。
- トップレベルと datasets の各要素に未知の項目を許可する。版1のスキーマは変更せず保持する。

### 版1（2026-10-07、#36）

- 初版。

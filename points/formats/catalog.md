# 地点カタログフォーマット

[地点データの README に戻る](../README.md#公開データのカタログ)

## 概要

地点カタログの形式と各版の変更を説明する。Pages に掲載された地点データについて、取得先とダウンロード後の検証に必要な manifest を一覧する。

これはアプリなどが地点データを取得するための一覧。公開処理が各データの manifest と公開履歴を引き継ぐために使う管理ファイル `/catalog.json` とは別のファイル。

## 版について

地点カタログの版は `schemaVersion` で確認する。現行版は `1`。ほかの形式の版とは独立して管理する。

版内の変更は `版1.1` のように小数点以下で区別する。小数点以下は文書上の番号で、JSON の版の値には含めない。

項目表の「版の履歴」は、フィールドの追加や変更があった版を示す。スキーマは版ごとに分けて管理する。

| 形式の版 | 機械検証用スキーマ | 規格 |
|---|---|---|
| 1 | [catalog-v1.schema.json](../schemas/catalog-v1.schema.json) | JSON Schema Draft 2020-12 |

## 項目

| 項目 | JSON の型 | 必須 | 内容・制約 | 版の履歴 |
|---|---|---|---|---|
| `schemaVersion` | integer | 必須 | カタログの形式の版。`1` | [版1] 追加 |
| `datasets` | array | 必須 | 公開済み地点データの一覧。該当するデータがなければ `[]` | [版1] 追加 |
| `datasets[].id` | string | 必須 | データセットの識別子（例: `osm-peaks`）。小文字の英字で始まり、小文字の英数字・下線・ハイフンで構成する。同じカタログ内で重複させない | [版1] 追加 |
| `datasets[].name` | string | 新規生成では必須 | データセットの表示名。空白以外の文字を含む。manifest の `name` から取得する。識別には `id` を使う | [版1] 追加 |
| `datasets[].manifestUrl` | string | 必須 | 対応する最新版 manifest の HTTPS URL。ホスト名の後に `/` を持つ | [版1] 追加 |
| `datasets[].manifest` | object | 新規生成では必須 | 公開する manifest のコピー。ダウンロードと検証に必要な情報を同梱する | [版1] 追加 |

トップレベルと `datasets` の各要素には、上表にない項目を追加できない。同梱する `manifest` にはデータ種別ごとの追加項目を含められる。スキーマ上は `name` と `manifest` を省略できるが、新規生成のカタログには両方を必ず含める。`id` の重複はスキーマだけでは検出できないため、別途確認する。

### 同梱する manifest

カタログのスキーマでは、`manifest` がある場合に次の項目を検証する。各データセットの manifest 自体の仕様にも従う。

| 項目 | JSON の型 | 必須 | カタログのスキーマでの制約 | 版の履歴 |
|---|---|---|---|---|
| `schemaVersion` | integer | 必須 | 正の整数 | [版1] 追加 |
| `version` | string | 必須 | 空でない文字列 | [版1] 追加 |
| `downloadUrl` | string | 必須 | ホスト名を持つ HTTPS URL | [版1] 追加 |
| `fileName` | string | 必須 | 空でない文字列 | [版1] 追加 |
| `sha256` | string | 必須 | 小文字の 16 進数 64 文字 | [版1] 追加 |
| `sizeBytes` | integer | 必須 | 正の整数。取得する gzip のバイト数 | [版1] 追加 |
| `uncompressedSizeBytes` | integer | 必須 | 正の整数。展開後の JSON のバイト数 | [版1] 追加 |
| `license` | string | 必須 | 空でない利用条件の文字列 | [版1] 追加 |
| `attribution` | string | 必須 | 空でない出典表示の文字列 | [版1] 追加 |
| `dataSchemaVersion` | integer | 新規生成では必須 | データ本体の形式の版。正の整数 | [版1] 追加 |
| `name` | string | 新規生成では必須 | 空白以外の文字を含む表示名 | [版1] 追加 |

地点データの件数 `pointCount` なども、公開する manifest の内容をそのまま含める。共通項目の意味は [manifest フォーマット](../../formats/manifest.md)、地点固有の項目は [manifest の地点データ用の項目](../../formats/manifest.md#地点データ用の項目) を参照する。

地点はデータセットの `id` と地点自身の `id` を組み合わせて識別する。山頂の表示名は正式版・開発版とも「山頂」とする。

## 読み込みと検証

1. 利用する配布先のカタログを取得し、カタログ自身の `schemaVersion` に対応していることを確認する。
2. 各データの `manifest.schemaVersion` と `manifest.dataSchemaVersion` が、アプリの対応する版に含まれることを確認する。「現在の版以下」という比較では判定しない。データ本体の版が不明なら利用不可として扱い、manifest の版から推測しない。
3. 同梱された `manifest.downloadUrl` から本体を取得する。manifest を別途取得したり、アプリ側で URL を組み立てたりする必要はない。
4. 展開前に `manifest.sizeBytes` と `manifest.sha256`、展開後に `manifest.uncompressedSizeBytes` と件数 `manifest.pointCount` を照合する。

非対応のデータはダウンロードせず、対応しているほかのデータを利用できる。通信や検証に失敗しても保存済みデータは消さない。例えばカタログ版1・manifest 版5・地点データ版5に対応するアプリは、同梱 manifest の `schemaVersion` と `dataSchemaVersion` がともに `5` のデータを利用できる。

版の判定と検証は読み込み側アプリで実装する契約であり、このリポジトリでは必要な情報を生成・配布する。

URL の形式を検証するときは、JSON Schema の `format` の検査を有効にする。カタログと公開 manifest の一致、取得した本体のサイズ・ハッシュ・件数は、スキーマとは別に確認する。

### 配布と更新

正式版の通常生成は Release の作成までで、Pages の manifest・カタログ・履歴を更新しない。正式版カタログは掲載済み情報であり、最新 Release をすべて収録する一覧ではない。正式版は利用する Release を選び、添付 manifest を固定して取得する。手動公開・復旧・削除では正式版側の Pages 情報も更新される。開発版（Pre-release）は Pages の公開・削除に合わせて更新する。

正式版の `points/catalog.json` には正式版だけ、開発版の `points/catalog-dev.json` には開発版だけを収録する。未公開のデータや地点以外のデータは含めない。利用する配布先はカタログの URL で区別し、アプリは他方へ自動で切り替えない。

Pages を更新する公開・削除時に、サイト全体の manifest 一覧から自動生成し、他の地点データと配布先を引き継ぐ。manifest と同じ Pages 配置で更新し、配置後に一致を確認する。生成物は git に含めない。公開 URL と削除の操作手順は [地点データの README](../README.md#公開データのカタログ) を参照する。

## JSON の例

取得先は説明用の URL。同梱する manifest の版はカタログの版と独立しており、この例では manifest 版5を使う。

### 版1

```json
{
  "schemaVersion": 1,
  "datasets": [
    {
      "id": "osm-peaks",
      "name": "山頂",
      "manifestUrl": "https://example.com/points/osm-peaks/manifest.json",
      "manifest": {
        "schemaVersion": 5,
        "dataSchemaVersion": 5,
        "name": "山頂",
        "version": "20261007T071401Z-37585754546-1",
        "fileName": "osm-peaks.json.gz",
        "downloadUrl": "https://example.com/osm-peaks.json.gz",
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
    }
  ]
}
```

## 変更履歴

### 版1（2026-10-07、#36）

- 初版。

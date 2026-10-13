# 補足 manifest フォーマット

[地点データの README に戻る](../README.md)

## 概要

補足 manifest の形式と各版の変更を説明する。補足を適用する前の地点データと補足 JSON の取得先・サイズ・ハッシュ・出典を記載し、補足を適用した配布データと結び付ける。通常の `manifest.json` とは別の、確認・編集用のファイル。

補足 JSON 本体の形式は [補足データフォーマット](supplements-data.md)、配布データの manifest は [manifest フォーマット](../../formats/manifest.md) を参照する。

## 版について

補足 manifestの版は `schemaVersion` で確認する。現行版は `1`。ほかの形式の版とは独立して管理する。

版内の変更は `版1.1` のように小数点以下で区別する。小数点以下は文書上の番号で、JSON の版の値には含めない。

項目表の「版の履歴」は、フィールドの追加や変更があった版を示す。スキーマは版ごとに分けて管理する。

| 形式の版 | 機械検証用スキーマ | 規格 |
|---|---|---|
| 1 | [supplements-manifest-v1.schema.json](../schemas/supplements-manifest-v1.schema.json) | JSON Schema Draft 2020-12 |

## 項目

トップレベルの項目はすべて必須。スキーマにない項目は追加できない。

| 項目 | JSON の型 | 内容・制約 | 版の履歴 |
|---|---|---|---|
| `schemaVersion` | integer | 補足 manifest 自体の形式の版。`1` | [版1] 追加 |
| `version` | string | 対応する配布データの版。通常の manifest の `version` と同じ。英数字で始まり、英数字・ピリオド・ハイフン・下線で構成する | [版1] 追加 |
| `dataSchemaVersion` | integer | 補足を適用する前の地点データの形式の版。`5`。補足 JSON 自体の版ではない | [版1] 追加 |
| `distributionSha256` | string | 補足を適用した配布 gzip の SHA-256。通常の manifest の `sha256` と同じ。小文字の 16 進数 64 文字 | [版1] 追加 |
| `base` | object | 補足を適用する前の地点データのファイル情報。下表を参照 | [版1] 追加 |
| `supplements` | object | 補足 JSON のファイル情報。下表を参照 | [版1] 追加 |
| `sources` | object | 補足 JSON の出典一覧。出典 ID ごとに `source`・`license`・`attribution`・`changes` を任意の文字列で記載する。出典がない場合は `{}` | [版1] 追加 |
| `license` | string | 元の地点データの利用条件。通常の manifest と同じ。OSM 由来は `ODbL-1.0` | [版1] 追加 |
| `attribution` | string | 元の地点データの出典表示。通常の manifest と同じ。OSM 由来は `© OpenStreetMap contributors` | [版1] 追加 |

### ファイル情報

`base` と `supplements` はそれぞれ次の 4 項目をすべて持つ。ほかの項目は追加できない。

| 項目 | JSON の型 | 内容・制約 | 版の履歴 |
|---|---|---|---|
| `fileName` | string | `base` はパス区切りを含まない `.json.gz` のファイル名。`supplements` は `supplements.json` | [版1] 追加 |
| `downloadUrl` | string | ファイルを取得する HTTPS URL | [版1] 追加 |
| `sizeBytes` | integer | 取得するファイルのバイト数。正の整数。`base` は gzip のままの大きさ | [版1] 追加 |
| `sha256` | string | 取得するファイルそのものの SHA-256。小文字の 16 進数 64 文字。`base` は展開前の gzip のハッシュ | [版1] 追加 |

`sources` の構造は補足データのスキーマにある出典一覧の定義を共用する。出典の記載方法は [補足データフォーマット](supplements-data.md) を参照する。元の地点データの出典と、補足の出典を両方保持する。

## 読み込みと検証

1. 補足 manifest の `schemaVersion` が `1`、`dataSchemaVersion` が `5` であることを確認する。
2. 通常の manifest と `version`・`dataSchemaVersion`・`license`・`attribution` が一致し、`distributionSha256` が通常の manifest の `sha256` と一致することを確認する。
3. `base` と `supplements` の `downloadUrl` からファイルを取得し、それぞれの `sizeBytes` と `sha256` を照合する。
4. 元の地点データを展開し、補足 JSON を適用する。補足の出典と、補足を適用した地点データが配布内容と一致することを確認する。

URL の形式を検証するときは、JSON Schema の `format` の検査を有効にする。取得したファイルのサイズ・ハッシュや、通常の manifest・補足 JSON・配布データとの一致はスキーマだけでは検証できないため、別途確認する。

### OSM 山頂データでの配布

補足 manifest は `supplement-manifest.json` として、元データの `osm-peaks-source.json.gz`、補足の `supplements.json` と同じ Release に添付する。公開前に添付ファイルを再取得して検証し、補足関連の添付が一部しかない Release は公開しない。補足のない過去の Release も検証対象にできる。

アプリは補足を適用した `osm-peaks.json.gz` を通常の manifest から取得する。確認・編集用の添付と補足 manifest はアプリの読み込み対象にしない。具体的な配布・編集手順は [OSM 山頂の README](../osm-peaks/README.md) を参照する。

## JSON の例

URL・サイズ・ハッシュ・配布版は説明用の仮の値。実際のファイルの検証には使えない。

### 版1

```json
{
  "schemaVersion": 1,
  "version": "example-1",
  "dataSchemaVersion": 5,
  "distributionSha256": "0000000000000000000000000000000000000000000000000000000000000000",
  "base": {
    "fileName": "osm-peaks-source.json.gz",
    "downloadUrl": "https://example.com/osm-peaks-source.json.gz",
    "sizeBytes": 100,
    "sha256": "1111111111111111111111111111111111111111111111111111111111111111"
  },
  "supplements": {
    "fileName": "supplements.json",
    "downloadUrl": "https://example.com/supplements.json",
    "sizeBytes": 200,
    "sha256": "2222222222222222222222222222222222222222222222222222222222222222"
  },
  "sources": {},
  "license": "ODbL-1.0",
  "attribution": "© OpenStreetMap contributors"
}
```

## 変更履歴

### 版1（2026-10-10、#52）

- 初版。

# metadata フォーマット

[リポジトリの README に戻る](../README.md)

## 概要

データ JSON と配布 manifest で共有する共通情報。各文書のトップレベルに `metadata` object として置く。配布 manifest には、指し示すデータ JSON の `metadata` をそのままコピーする。

配布版・取得先・サイズ・ハッシュ・配布データの生成日時 `dataTimestamp` は manifest の直下に置く。これらは `metadata` に含めない。manifest の項目は [manifest フォーマット](manifest.md) を参照する。

## 版について

metadata の現行版は `1`。`metadata` 自体に `schemaFormat`・`schemaVersion` は置かず、文書全体のスキーマが参照する共通スキーマで版を指定する。地点 JSON・補足 JSON・manifest の版とは独立して管理する。

版内の変更は `版1.1` のように小数点以下で区別する。小数点以下は文書上の番号で、参照するスキーマの版には含めない。

項目表の「版の履歴」は、フィールドの追加や変更があった版を示す。スキーマは版ごとに分けて管理する。

| 形式の版 | 機械検証用スキーマ | 規格 |
|---|---|---|
| 1 | [metadata-v1.schema.json](../schemas/metadata-v1.schema.json) | JSON Schema Draft 2020-12 |

### スキーマからの参照

文書全体のスキーマから `$ref` で共通定義を参照する。

```json
{
  "$ref": "../schemas/metadata-v1.schema.json"
}
```

参照パスは、参照元のスキーマファイルからの相対パスにする。出典一覧だけを共用する場合は `#/$defs/sources`、タグ情報だけなら `#/$defs/tags` を参照する。形式ごとの必須条件は、参照元のスキーマで追加する。

## 共通項目

共通スキーマでは、各項目は任意。`metadata` には空 object を指定できる。未知の項目は許可し、読み手は解釈しない。既知の項目の制約は維持する。

| 項目 | 型 | 必須 | 内容・制約 | 版の履歴 |
|---|---|---|---|---|
| `sources` | object の配列 | 任意 | 出典名を `name` に持つ出典情報の配列。元データと補足の出典を同じ一覧に保持する。各出典の構造は下表を参照 | [版1] 追加 |
| `tags` | object の配列 | 任意 | タグ名を `name` に持つタグ情報の配列。各地点の `tags` 配列とは別の、文書全体で共有する情報 | [版1] 追加 |
| `pointCount` | integer | 任意 | 収録地点の件数。1 以上の整数。地点 JSON の `points` の要素数と一致させる | [版1] 追加 |

### 出典ごとの情報

`sources` の各要素は、必須の `name` と任意の出典情報を持つ object。`name` は空文字・前後の空白・配列内の重複を認めない。タグ名やファイル名とは独立して扱う。以下の項目は `metadata.sources[]` に置く。

| 項目 | 型 | 必須 | 内容・制約 | 版の履歴 |
|---|---|---|---|---|
| `name` | string | 必須 | 出典名。配列内で重複しない名前 | [版1] 追加 |
| `source` | string | 任意 | 資料名・参照 URL | [版1] 追加 |
| `license` | string | 任意 | 資料に適用される利用条件の名称・版・説明 URL | [版1] 追加 |
| `attribution` | string | 任意 | 利用条件に従って表示する著作者名・出典表示 | [版1] 追加 |
| `changes` | string | 任意 | 資料に加えた加工・変更 | [版1] 追加 |
| `lastestTimeStamp` | string | 任意 | この出典に由来する収録データの最終更新日時。タイムゾーン付き ISO 8601。取得日時や配布データの生成日時ではない。不明なら省略する | [版1] 追加 |

`name` 以外の出典属性は任意。未知の属性は許可し、読み手は解釈しない。既知の属性には指定された型を使う。日時を推測で補わない。検証時は JSON Schema の `format` 検査を有効にする。

### タグごとの情報

`tags` の各要素は、必須の `name` と任意の読みを持つ object。`name` は空文字・前後の空白・配列内の重複を認めない。以下の項目は `metadata.tags[]` に置く。

| 項目 | 型 | 必須 | 内容・制約 | 版の履歴 |
|---|---|---|---|---|
| `name` | string | 必須 | タグ名。各地点の `tags` の値と一致させる | [版1] 追加 |
| `nameReading` | string | 任意 | タグ名の読み。空文字や前後に空白がある文字列は指定できない | [版1] 追加 |

読みが不明なら `nameReading` を省略する。読みがなくても `name` は必須。未知の属性は許可し、読み手は解釈しない。既知の属性には指定された型を使う。

読みを登録していないタグは読みなしとして扱う。読みのあるタグを読みの文字コード順、同じ読みならタグ名の文字コード順で並べ、読みのないタグはその後にタグ名の文字コード順で並べる。配列の記載順には依存しない。読みをひらがなで記載する運用では、検証処理は空でない文字列かを確かめ、ひらがなだけかどうかは検査しない。

## 形式ごとの条件

| 形式 | metadata の扱い |
|---|---|
| [地点 JSON](../points/formats/data.md) | `metadata` と各項目は任意。生成時は実際の地点数を `pointCount` に設定する |
| [補足 JSON](../points/formats/supplements-data.md) | `metadata` は任意。`sources` と `tags` を使い、`pointCount` は指定しない |
| [配布 manifest](manifest.md) | `metadata` と、1 件以上の `sources` は必須。地点データの manifest では `pointCount` も必須。内容はデータ JSON と一致させる |

必須条件は各形式のスキーマで検証する。本体と manifest の一致や、`pointCount` と実際の件数の一致はスキーマだけでは確認できないため、生成・公開処理で照合する。

## JSON の例

地点 JSON の `metadata` の例。manifest にも同じ object をコピーする。URL・件数・日時は説明用の値。

```json
{
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
  }
}
```

## 変更履歴

### 版1（2026-10-14）

- 地点 JSON・補足 JSON・配布 manifest の共通定義をまとめる。
- 出典一覧 `sources`、タグ情報 `tags`、地点件数 `pointCount` を定義する。
- `sources`・`tags` は object の配列とし、出典・タグはそれぞれ必須の `name` で識別する。識別子の重複は認めない。未公開の版1に統合する。
- 出典ごとの最終更新日時は `sources[].lastestTimeStamp` に記載する。

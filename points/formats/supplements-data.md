# 補足データフォーマット

[地点データの README に戻る](../README.md)

## 概要

補足 JSON の形式と各版の変更を説明する。

補足ファイルを単体で配布するときの manifest は、[共通の manifest フォーマット](../../formats/manifest.md#補足ファイル単体の配布) を使う。

## 版について

補足 JSON の版は `schemaVersion` で確認する。現行版は `2`。ほかの形式の版とは独立して管理する。

版内の変更は `版1.1` のように小数点以下で区別する。小数点以下は文書上の番号で、JSON の版の値には含めない。

項目表の「版の履歴」は、フィールドの追加や変更があった版を示す。スキーマは版ごとに分けて管理する。

| 形式の版 | 機械検証用スキーマ | 規格 |
|---|---|---|
| 2 | [supplements-data-v2.schema.json](../schemas/supplements-data-v2.schema.json) | JSON Schema Draft 2020-12 |
| 1 / 1.1 | [supplements-data-v1.schema.json](../schemas/supplements-data-v1.schema.json) | JSON Schema Draft 2020-12 |

## 項目

地点の補足情報は一つの JSON ファイルにまとめる。`schemaVersion`・`points` と、任意の共通情報 `metadata` を持ち、`metadata.sources` に出典一覧、`metadata.tags` にタグ情報を格納し、`points` の `id` で元の地点に対応付ける。地点ごとにタグ・表示名・よみがな・別名・除外を指定する。

例の出典情報は、実際に確認した資料とその利用条件に置き換える。Wikipedia では参照した版の固定リンクを記載する。タグを追加しない地点は `tags` を省略する。

| 項目 | 型 | 必須 | 内容 | 版の履歴 |
|---|---|---|---|---|
| `schemaFormat` | string | 必須 | JSON の形式の識別子。`yamamuki-point-supplements` 固定。他の形式と区別する | [版2] 追加 |
| `schemaVersion` | integer | 必須 | 補足 JSON の形式の版。`2` | [版1] 追加 |
| `metadata` | object | 任意 | 共通の出典一覧とタグ情報。省略・空 object は共通情報なし | [版2] 追加 |
| `points` | array | 必須 | 地点ごとの補足。`id` で元データと対応付ける | [版1] 追加 |

### 共通情報（metadata）

共通項目の型・構造・制約は [metadata フォーマット](../../formats/metadata.md) を参照する。補足 JSON では `metadata` は任意。`sources` と `tags` を使い、`pointCount` は指定しない。

`metadata.sources` には補足に使った資料を登録する。地点や補足項目ごとの出典指定は行わない。生成時は補足の出典情報をマージ済み地点 JSON の `metadata.sources` に保持する。

`metadata.tags` への登録だけでは地点にタグを追加しない。タグは地点ごとの `tags` で追加する。元データのタグに対して読みだけを登録でき、使っていないタグの情報も登録できる。生成時は補足のタグ情報を地点 JSON の `metadata.tags` に引き継ぐ。

### 地点ごとの項目

`id` と `relatedIds` の値の制約は [地点の共通項目スキーマ](../schemas/point-common-v1.schema.json) を参照する。必須・省略・`null` の扱いは各形式で定める。

| 項目 | 型 | 必須 | 内容 | 版の履歴 |
|---|---|---|---|---|
| `id` | string | 必須 | 対応する地点の `id` と完全一致する文字列。空文字・前後の空白は使わない | [版2] 追加 |
| `osmId` | integer | 廃止 | 版1の OSM ノード番号による対象指定。版2では `id: "osm:<ノード番号>"` を使う | [版1] 追加<br>[版2] 廃止 |
| `relatedIds` | string の配列 | 任意（補正項目のうち1つ以上が必須） | 関連先 ID の配列全体を置き換える。空配列で解除する。重複・自分自身の ID は指定しない | [版2] 追加 |
| `tags` | string の配列 | 任意（補正項目のうち1つ以上が必須） | 追加するタグの配列。元のタグを保って重複なく追記する | [版1] 追加 |
| `tagsRemove` | string の配列 | 任意（補正項目のうち1つ以上が必須） | 元のタグから削除する値の配列。画面上のタグ削除も保存する | [版1] 追加 |
| `name` | string | 任意（補正項目のうち1つ以上が必須） | 表示名を置き換える。空文字は使えない | [版1] 追加 |
| `nameReading` | string・string の配列・null | 任意（補正項目のうち1つ以上が必須） | よみがなを置き換える。複数ある場合は重複のない1件以上の文字列配列。`null` は元のよみがなを削除する。空配列は使わない | [版1] 追加<br>[版2] 複数の読みに対応 |
| `graphic` | object または null | 任意（補正項目のうち1つ以上が必須） | `svg` に SVG 本文、任意の `scale` に表示倍率を指定する。元の画像を置き換え、`null` は画像を削除する。省略時は元の画像を保つ | [版1.1] 追加（#56） |
| `aliases` | object の配列（旧文字列も読める） | 任意（補正項目のうち1つ以上が必須） | 追加・補正する別名の配列。各要素は必須の `name` と任意の `nameReading` を持つ | [版1] 追加 |
| `aliasesRemove` | string の配列 | 任意（補正項目のうち1つ以上が必須） | 元の別名から削除する値の配列 | [版1] 追加 |
| `exclude` | boolean | 任意（補正項目のうち1つ以上が必須） | `true` で表示・配布対象から除外する | [版1] 追加 |
| `reason` | string | 任意 | 補正理由（任意）。表示名の置き換え・除外・削除でも省略できる | [版1] 追加<br>[版2] 任意化 |
| `note` | string | 任意 | 管理用の自由なメモ。地点名の置き換えや一致検査には使わない | [版1] 追加 |
| `expected` | object | 任意 | 補正前に想定する `name`・`nameReading`・`aliases`・`relatedIds`。元データと異なる場合は適用を中止する | [版1] 追加 |

`id` で対象地点を指定する。同じ ID の補足行の重複はエラーにする。`relatedIds` は対象を探すキーではなく、その地点の関連付けを補正する値。関連先を含むデータセットは同時に読み込まれていなくてもよい。

`id` はデータセットをまたいで一意とする。補足の適用時は、対象の地点 JSON にその ID が存在することを確認する。OSM のノード番号を持たない GSI や手動作成の地点には `id` を使う。ID の永続性は保証されないため、元データを更新した場合は対応する地点を確認する。

補足する各項目を省略すると元の値を使う。タグ・別名の空配列は追加・削除しない指定とする。`relatedIds` の空配列は関連付けを解除する。同じタグや別名を追加と削除の両方に指定できない。タグ・別名・よみがなには空文字や前後の空白を使えず、配列内の重複も拒否する。各地点には `tags`・`tagsRemove`・`name`・`nameReading`・`graphic`・`aliases`・`aliasesRemove`・`exclude`・`relatedIds` のいずれかの指定が必要。

## 読み込みと検証

現行形式では、各 object に未知の項目を許可し、読み手はそれを解釈しない。既知の項目の型・必須条件・重複などの制約は維持する。

`schemaVersion` が対応する版であることを確認し、`points` の `id` で元データの地点に対応付ける。不正な値・未知の ID・重複 ID・追加と削除の競合がないかを検証する。`expected` がある場合は元データと照合し、不一致なら適用を中止する。

`metadata` は [共通スキーマ](../../schemas/metadata-v1.schema.json) と補足 JSON 固有の条件で検証する。版6の地点 JSON と補足 JSON を併せて検証する場合は、両者の`metadata.tags` が一致することも確認する（省略は空配列として比較）。

補足 JSON の編集と保存は [地点データビューアの操作手順](../README.md#補足-json-を開く) を参照する。

### タグと出典の引き継ぎ

`tags` は「日本百名山」などの分類ラベルを持つ配列。OSM の元タグをそのまま収録する項目ではなく、データ作成者が分類を確認して付ける。山名から分類を推測しない。アプリは未知のタグを無視できるようにする。

生成時は補足 JSON の出典をマージ済み地点 JSON の `metadata.sources` に保持する。元データの出典と利用条件も残す。補足単体の配布 manifest は、補足 JSON の `metadata` をそのままコピーし、`dataSchemaVersion: 2` とする。

地点名と別名の `nameReading` は、1つの読みを文字列、複数の読みを文字列配列で記録できる。例えば白馬岳は `"nameReading": ["しろうまだけ", "はくばだけ"]` とする。読みの空配列・重複・空文字・前後の空白は認めない。`metadata.tags` の `nameReading` も同じく文字列または文字列配列を使う。

## JSON の例

### 版2

任意項目と子 object の項目を含む例。対象地点は `id` で指定する。`exclude: false` は除外しない指定で、2 行目は除外する指定。資料名・URL・地点は説明用の値。

```json
{
  "schemaFormat": "yamamuki-point-supplements",
  "schemaVersion": 2,
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
    ]
  },
  "points": [
    {
      "id": "osm:123456",
      "name": "見本山",
      "nameReading": ["みほんやま", "みほんさん"],
      "aliases": [
        {
          "name": "見本岳",
          "nameReading": "みほんだけ"
        }
      ],
      "aliasesRemove": [
        "旧別名"
      ],
      "tags": [
        "日本百名山"
      ],
      "tagsRemove": [
        "旧分類"
      ],
      "exclude": false,
      "graphic": {
        "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\"><circle cx=\"5\" cy=\"5\" r=\"4\"/></svg>",
        "scale": 1.5
      },
      "reason": "見本の資料で名前と読みを確認",
      "note": "照合結果を記録する管理用メモ",
      "expected": {
        "name": "旧見本山",
        "nameReading": "きゅうみほんやま",
        "aliases": [
          {
            "name": "旧別名",
            "nameReading": "きゅうべつめい"
          }
        ],
        "relatedIds": [
          "custom:old-sample"
        ]
      },
      "relatedIds": [
        "gsi-vector:sample",
        "custom:sample"
      ]
    },
    {
      "exclude": true,
      "reason": "見本山と重複する地点",
      "id": "osm:234567"
    }
  ]
}
```


出典情報は説明用。実際に確認した資料と利用条件に置き換える。

### 版1.1

```json
{
  "schemaVersion": 1,
  "sources": {
    "山の資料": {
      "source": "参照した資料の名称・URL",
      "license": "資料の利用条件の名称・URL",
      "attribution": "資料の著作者表示",
      "changes": "山名一覧を抽出し、osmId・よみがな・別名を追加"
    }
  },
  "points": [
    {
      "osmId": 204683948,
      "note": "羊蹄山。確認用メモ",
      "tags": [
        "日本百名山",
        "北海道百名山"
      ],
      "nameReading": "みほんやま",
      "aliases": [
        "見本岳"
      ],
      "graphic": {
        "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><path d=\"M10 90 L50 10 L90 90 Z\"/></svg>",
        "scale": 1.5
      },
      "name": "見本山",
      "aliasesRemove": [
        "旧別名"
      ],
      "tagsRemove": [
        "旧分類"
      ],
      "exclude": false,
      "reason": "見本の資料で名前と読みを確認",
      "expected": {
        "name": "旧見本山",
        "nameReading": "きゅうみほんやま",
        "aliases": [
          "旧別名"
        ]
      }
    }
  ]
}
```

### 版1

```json
{
  "schemaVersion": 1,
  "sources": {
    "山の資料": {
      "source": "参照した資料の名称・URL",
      "license": "資料の利用条件の名称・URL",
      "attribution": "資料の著作者表示",
      "changes": "山名一覧を抽出し、osmId・よみがな・別名を追加"
    }
  },
  "points": [
    {
      "osmId": 204683948,
      "note": "羊蹄山。確認用メモ",
      "tags": [
        "日本百名山",
        "北海道百名山"
      ],
      "nameReading": "みほんやま",
      "aliases": [
        "見本岳"
      ],
      "name": "見本山",
      "aliasesRemove": [
        "旧別名"
      ],
      "tagsRemove": [
        "旧分類"
      ],
      "exclude": false,
      "reason": "見本の資料で名前と読みを確認",
      "expected": {
        "name": "旧見本山",
        "nameReading": "きゅうみほんやま",
        "aliases": [
          "旧別名"
        ]
      }
    }
  ]
}
```

## 変更履歴

### 版2（2026-10-14）

- 必須の `schemaFormat: "yamamuki-point-supplements"` を追加し、形式を識別する。未公開の版2に統合する。

- 出典一覧とタグ情報を `metadata.sources`・`metadata.tags` にまとめる。直下の `sources`・`tags` は使わない。
- `aliases` を、`name` と任意の `nameReading` を持つ object 配列に変更する。
- 対象地点をグローバルな `id` で指定する。旧 `osmId` 指定は廃止する。`reason` は任意とする。
- `relatedIds` で関連先 ID の配列を置き換え、空配列で解除する。
- タグ共通の読みと出典ごとの最終更新日時は共通 metadata スキーマに従う。
- 版1のスキーマでは版2を読めない。旧版の読み込みは維持し、新規保存・配布は版2にそろえる。補足を指す manifest の `dataSchemaVersion` は `2` とする。

### スキーマの改名（2026-10-13、#66）

- `points/schemas/supplements-v1.schema.json` を `points/schemas/supplements-data-v1.schema.json` に改名した。検証ルールと JSON の形式の版は変えていない。

### 版1.1（2026-10-12、#56）

- `graphic` による SVG 画像と表示倍率の補足を追加した。
- 追加前のスキーマは `additionalProperties: false` のため、`graphic` を含む JSON を拒否する。画像付きの補足を検証する場合は、版1.1の追加を反映したスキーマが必要。

### 版1（2026-10-10、#52）

- 初版。

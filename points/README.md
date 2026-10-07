# 地点データ

[リポジトリの README に戻る](../README.md)

山頂やランドマークなど、名前と位置を持つ地点データの共通形式をまとめる。生成処理・収録条件・配布手順は、取得元と対象ごとのフォルダに分ける。

| 対象・取得元 | 状態 | 詳細 |
|---|---|---|
| OpenStreetMap の山頂 | 実装済み | [osm_peaks](osm_peaks/README.md) |
| ランドマークなど | 将来追加予定・未実装 | 追加時に専用フォルダを作る |

## 公開データのカタログ

公開済みの地点データすべてのダウンロードと検証に必要な情報を、次のカタログから取得できる。

| 配布先 | カタログ | 使用する項目 |
|---|---|---|
| 正式版 | [正式版のカタログ](https://shohei0205.github.io/yamamuki-data/points/catalog.json) | `channel` が `stable` |
| 開発版 | [開発版のカタログ](https://shohei0205.github.io/yamamuki-data/points/catalog-dev.json) | `channel` が `dev` |

正式版の `points/catalog.json` には `stable` のデータだけ、開発版の `points/catalog-dev.json` には `dev` のデータだけを収録する。アプリは利用する配布先のカタログを取得し、他方に自動で切り替えない。まだ公開していないデータや地形など地点以外のデータは含めない。

機械検証用のスキーマは [地点カタログの版1](../schemas/points/catalog-v1.schema.json) を参照する。同じ `id` と `channel` の組は重複させない。

| 項目 | 内容 |
|---|---|
| `schemaVersion` | カタログの形式の版。現在は整数の `1`。manifest・地点データの版とは独立 |
| `datasets` | データセットの配列。公開済み地点データがない場合は `[]` |
| `datasets[].id` | データセット名（例: `osm_peaks`）。地点の `id` と組み合わせて識別する |
| `datasets[].channel` | 正式版は `stable`、開発版は `dev` |
| `datasets[].manifestUrl` | 対応する最新版 manifest の HTTPS URL（互換用・個別取得用） |
| `datasets[].manifest` | manifest の内容。形式の版・データの版・`downloadUrl`・ファイル名・SHA-256・圧縮前後のサイズ・件数・出典などを含む |

両カタログは公開・削除時にサイト全体の manifest 一覧から自動生成し、他の地点データと配布先を引き継ぐ。アプリは利用する配布先の項目を選び、同梱された `manifest.downloadUrl` から本体を取得する。manifest を別途取得する必要はない。`manifest.sizeBytes`・`manifest.sha256` で取得した gzip を検証し、展開後にサイズと件数を照合する。形式の版とデータの版も同梱情報から確認する。manifest とカタログは同じ Pages 配置で更新し、配置後に一致を確認する。生成物は git に含めない。サイト全体の更新用 `/catalog.json` とは別の、地点データの取得先を一覧するファイル。

`manifest` は新規生成するカタログに必ず含める。追加前のカタログを読む場合に限り、省略されていたら `manifestUrl` から取得する。既存項目を維持した追加のため、カタログの版は1のままとする。

同梱情報は公開する manifest のコピーで、アプリ側で URL を組み立てない。`downloadUrl` がない旧版（版1〜3）の OSM 山頂についてのみ、公開処理で対応する Release の URL を補う。元の manifest は変更しない。旧版の件数は `mountainCount`、版5以降は `pointCount` で確認する。

### カタログから削除する

Actions の「地点データをカタログから削除」→「Run workflow」で、正式版なら `main`、開発版なら `dev` を選び、`dataset` に `osm_peaks` などのデータセット名を入力する。配布先は実行元ブランチで決まり、対象の配布先だけを削除する。

地点カタログとサイト全体の一覧から対象を除き、その manifest・公開履歴の Pages ファイルも削除する。OSM 山頂の旧 URL がある場合はそれも削除し、次回の公開で旧 URL から復活することを防ぐ。他のデータ・配布先・公開履歴、Release とデータ本体は保持する。

既存一覧の取得・検証に失敗した場合や対象がない場合は配置を中止する。公開と同じ `publish-data-pages` グループで直列化し、配置後にカタログと履歴の一致を確認する。削除済みデータを再度公開すれば一覧に戻る。定期生成があるデータは次回の公開時に再掲載されるため、継続して配布を止める場合は生成・公開の運用も変更する。

### アプリで利用できるデータを判定する

アプリは、カタログ自身の `schemaVersion` に対応していることを確認し、各データの `manifest.schemaVersion` と `manifest.dataSchemaVersion` をそれぞれ確認する。単純に「現在の版以下」と比較せず、アプリが対応すると明示した版の一覧に含まれるかで判定する。

例えばカタログ版1・manifest 版5・地点データ版5に対応するアプリなら、各データの同梱 manifest が `"schemaVersion": 5`・`"dataSchemaVersion": 5` のときに利用できる。非対応の項目はダウンロードせず、対応しているほかのデータは利用できる。読み込みに失敗しても保存済みデータは消さない。

`dataSchemaVersion` は新規生成の manifest に必ず含め、カタログにもそのまま同梱する。未記載の旧山頂データは、公開処理が従来の manifest 版1〜5に対応する地点データの版を補う。それ以外で版が不明な場合は、アプリは利用不可として扱い、manifest の版から本体の版を推測しない。

この判定は読み込み側アプリで実装する契約であり、このリポジトリでは判定に必要な版の情報を生成・配布する。

## 地点の形式

データ本体は gzip で圧縮した UTF-8（BOM なし）の JSON 配列とし、配列の各要素を1地点のオブジェクトとする。以下は山頂・ランドマーク・手動作成の地点に共通するスキーマで、現在の地点データのスキーマは版5で、山頂の新規生成もこの版を使う。地点データの版は manifest の版と独立して管理する。manifest の形式と版は [リポジトリ共通仕様](../README.md#manifestjson) に従う。

機械検証用の [地点データの版5](../schemas/points/pointdata-v5.schema.json)（JSON Schema Draft 2020-12）は、gzip 展開後の JSON 配列に適用する。配列は1地点以上とし、定義していない地点の項目は許可しない。データセット全体での `id` の重複、別名と表示名の一致、数値が有限であることは別途確認する。URL の `format` 検査も有効にする。

### 項目

「必須」は項目自体を省略できないことを表す。必須項目は `id`・`name`・`latitude`・`longitude` の4つ。その他は省略できる。`elevationM`・`nameReading`・`wikipediaUrl`・`wikidataUrl` の省略は `null`、`aliases` の省略は `[]` と同じ意味とする。

| 項目 | JSON の型 | 必須 | 内容・制約 |
|---|---|---|---|
| `id` | string | 必須 | データセット内で一意な、空でない固定 ID。取得元の接頭辞は不要 |
| `osmId` | integer | 任意 | 関連する OSM ノードの正の整数 ID。関連付けがない場合は省略し、`null` は使わない |
| `name` | string | 必須 | 空でない表示名 |
| `latitude` | number | 必須 | WGS 84 の緯度（度）。−90 以上90 以下 |
| `longitude` | number | 必須 | WGS 84 の経度（度）。−180 以上180 以下 |
| `elevationM` | number または null | 任意 | 地点自体の標高（m）。不明なら `null`。負の標高も使用できる |
| `nameReading` | string または null | 任意 | 表示名の読み仮名。不明なら `null` |
| `aliases` | string の配列 | 任意 | 別名。ない場合は `[]`。空の名前・表示名と同じ名前・重複は含めない |
| `wikipediaUrl` | string または null | 任意 | Wikipedia の解説記事の HTTPS URL。ない場合は `null` |
| `wikidataUrl` | string または null | 任意 | Wikidata の項目の HTTPS URL。ない場合は `null` |

数値に `NaN` や無限大は使用しない。文字列の名前・読み仮名・別名は前後の空白を除く。標高に建物の高さなどを入れない。リンク先の記事本文は同梱しない。

### JSON の例

次の配列は、OSM 由来の山頂と、OSM との関連付けがない手動作成の地点の形式を示す。手動作成の地点は説明用の架空のデータ。

```json
[
  {
    "id": "3403990450",
    "osmId": 3403990450,
    "name": "万三郎岳",
    "latitude": 34.8627963,
    "longitude": 139.0018525,
    "elevationM": 1405.6,
    "nameReading": "ばんざぶろうだけ",
    "aliases": ["天城山"],
    "wikipediaUrl": "https://ja.wikipedia.org/wiki/%E5%A4%A9%E5%9F%8E%E5%B1%B1",
    "wikidataUrl": null
  },
  {
    "id": "example-tower",
    "name": "展望塔",
    "latitude": 35.0,
    "longitude": 139.0
  }
]
```

### ID とデータセット

`id` は名前・座標を変更しても維持し、削除した ID を別の地点に再利用しない。OSM 山頂ではノード番号を先頭ゼロなしの文字列にし、手動作成では `"example-tower"` のような短い値を付ける。

アプリ側では「データセット名＋id」の組で地点を識別する。同じデータセットを複数ファイルに分ける場合も、そのデータセット全体で ID を一意にする。異なるデータセット間では同じ ID を使用できる。同一地点が複数のデータセットに含まれる場合の統合は別に判断する。

項目の取得方法・配列の並び順・収録対象の重複の扱いは各データの仕様に記載する。山頂では OSM ノード番号の数値順に並べる。

manifest の共通仕様は [リポジトリ共通の manifest.json](../README.md#manifestjson) を参照する。

## OSM との関連付け

手動作成の地点でも任意の `osmId` を指定できる。`id` は地点自体の永続的な識別子、`osmId` は関連する OSM ノードへの参照として扱う。

```json
{"id":"example-tower","osmId":123456,"name":"展望塔","latitude":35.0,"longitude":139.0}
```

上記は形式説明用の架空の地点・番号であり、実データでは確認した番号を指定する。`osmId` は正の整数とし、関連付けがない場合は項目そのものを省略する。`null` や文字列は使わない。

OSM 由来の山頂は `id` と `osmId` の両方を出力する（例: `id` が `"123456"`、`osmId` が `123456`）。手動作成の地点は OSM の関連先を変更・削除しても元の ID を維持する。関連付けだけでデータの自動統合・上書きは行わない。

`osmId` は従来どおりノードの番号を表す。way・relation は同じ番号のノードと区別できないため、この項目には指定しない。それらとの関連付けが必要になった時点で種別の表し方を追加する。

## データを追加するとき

`points/<取得元と対象>/` に `README.md`・`scripts/`・`tests/` をまとめる。例えば OSM のランドマークを追加する場合は `points/osm_landmarks/` とし、実装時にこの一覧とルートの一覧を更新する。生成物は各フォルダの `build/`・`dist/` に置き、git に含めない。

manifest は [リポジトリ共通仕様](../README.md#manifestjson) に従い、公開 URL・ファイル名・元データの取得方法・公開時の件数基準はデータセットごとに定める。生成・公開処理の共通化は追加データの要件に応じて行う。

## ライセンス

現在の OSM 由来データの出典は **© OpenStreetMap contributors**。利用条件は [リポジトリのライセンス](../README.md#ライセンス)を参照する。別の取得元を追加する場合は、その利用条件と出典を各データの資料に明記する。

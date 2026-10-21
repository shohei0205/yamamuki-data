# OpenStreetMap の山頂データ (osm-peaks)

[地点データの共通仕様に戻る](../README.md)

全国の山頂データの形式、取得先、生成・検査・公開の手順をまとめる。

この文書の構成：[配るもの](#配るもの)・[置き場所](#置き場所)・[作り方](#作り方)・[地点の補足対応表](#地点の補足対応表)・[出典とライセンス](#出典とライセンス)・[テスト](#テスト)

## 配るもの

Geofabrik の日本全国の OSM データから、`natural=peak` または `natural=volcano` の名前付きノードを抽出する。ロシア極東の PBF から国後島・択捉島・色丹島・歯舞群島の範囲にある同じ条件のノードも補い、`osm-peaks.json.gz` 1 ファイルにまとめる。way・relation と名前のないノードは含めない。収録する地点の `type` は山頂を表す `peak` とし、火山ノードも同じ種別で出力する。名前は前後の空白を除き、日本 PBF では `name:ja`、`name` の順に使う（`name:ja` だけのノードも含む）。日本以外の PBF では、空欄でない `name:ja` があるノードだけを収録し、その値を名前に使う。

追加範囲は [regions.json](regions.json) の島ごとの矩形（西端の経度、南端の緯度、東端の経度、北端の緯度の順）で指定する。国名や行政境界のタグでは判定しない。国後島・択捉島の日本語名付き山頂が 0 件の場合は欠落として生成を止める。尖閣諸島は既存の日本 PBF の収録分を使い、日本語名のない地点の名前は推測して補わない。

2026-10-03 配布分を確認すると、日本以外の PBF から追加する日本語名付き山頂は国後島 13 件、択捉島 37 件、色丹島 5 件、歯舞群島 0 件だった。爺爺岳（OSM ID `281896663`）には `name:ja` があり、統合後は 14,088 件になる。これらは確認時点の件数で、以後の生成では OSM の登録内容に従う。

複数の取得元に同じ OSM ID があれば、ノードの版、編集日時の順で新しい方を使う。同じ版・編集日時で内容が異なる場合は生成を止める。

地点 JSON の項目と制約は [地点データフォーマット](../formats/data.md) を参照する。

### 地点ごとの出典情報

各地点の `provenance` に取得元・元資料の分類・標高の出典を記録する。形式と制約は [地点データフォーマット](../formats/data.md#地点ごとの出典情報)を参照する。

| 項目 | 値 |
|---|---|
| `dataSource` | `osm`（OpenStreetMap） |
| `classification` | 元ノードのタグに応じて `natural=peak` または `natural=volcano` |
| `elevationSource` | `ele` から標高を取得できた場合は `osm`、未登録・解釈できない場合は項目を省略 |

`classification` は OSM の元の分類を保持する。共通の地点種別 `type` は、どちらも `peak` とする。名前・読み・別名・タグの補正では元の出典情報を保持する。標高を補正した場合は `elevationSource` を `supplement`（補足による補正）にし、削除した場合は省略する。補足の出典一覧は manifest の `metadata.sources` に記録する。

### ID の作り方と維持方針

地点 JSON 版6の `id` は `osm:<ノード番号>` とする（例: `osm:123456`）。`osmId` は版6で廃止する。他の地点との関連付けには `relatedIds` を使う。山頂版と名前付きノードの全部入り版では、同じノードに同じ ID を使う。版5以前の `id` は接頭辞のない数値文字列。

同じ OSM ノードなら、名前や位置が更新されても ID を維持する。OSM 上で別のノードに置き換わると、同じ山でも ID が変わる。[地点データの共通仕様](../formats/data.md#id-とデータセット)に従い、ID の維持は努力義務とし、永続性を保証しない。

## 置き場所

| 配布先 | manifest の取得方法 |
|---|---|
| 正式版 | 利用する `osm-peaks-<version>` の Release を選び、`https://github.com/shohei0205/yamamuki-data/releases/download/<タグ>/manifest.json` を固定して取得する。本文の先頭に「## 要確認」がある Release は選ばない。 |
| 開発版（Pre-release） | [開発版の最新版 manifest](https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/manifest.json) を取得する |

データ本体（`osm-peaks.json.gz`）は manifest の `downloadUrl` から取得する。正式版を切り替えるときは、利用する側で固定した Release のタグを更新する。リポジトリ全体の `releases/latest` は使わない。

正式版の通常生成は Pages を更新しない。正式版の [Pages manifest](https://shohei0205.github.io/yamamuki-data/points/osm-peaks/manifest.json) と公開履歴には、以前掲載した情報が残る。手動公開・復旧・カタログ削除では更新されることがあるが、正式版の最新 Release の取得先としては使わない。

[Releases](https://github.com/shohei0205/yamamuki-data/releases) に `manifest.json` と `osm-peaks.json.gz` を添付する。開発版の最新版 manifest は冒頭の表、公開履歴は「公開履歴」の節から取得できる。正式版は Release の manifest を使う。

manifest の項目・型は [リポジトリ共通の manifest.json](../../formats/manifest.md) を、形式の版ごとの説明と変更履歴は [地点データフォーマット](../formats/data.md) を参照する。版4 のときの配り方（`peaks/`・`peaks-<version>`・`japan-mountains.json.gz`）は [版4の旧形式データ](#版4の旧形式データ) にある。

山頂データでは次の値・取得方法を使う。

| 項目 | 山頂データでの値・取得方法 |
|---|---|
| `version` | 生成時の UTC 日時・Actions 実行 ID・再実行番号を連結 |
| `fileName` | `osm-peaks.json.gz` |
| `downloadUrl` | 正式版は `osm-peaks-<version>`、開発版は `osm-peaks-dev-<version>` の Release の gzip ファイル |
| `pointCount` | 収録した名前付き山頂ノードの件数 |
| `dataTimestamp` | 配布データを生成した UTC 日時。PBF の基準日時とは区別する |
| `metadata.sources` の `OpenStreetMap（日本・Geofabrik 配布）` の `latestTimestamp` | 収録する山頂ノードの OSM 最終編集日時の最大値。収録対象外のノードは集計しない |
| `metadata.sources` の `OpenStreetMap（日本・Geofabrik 配布）` の `source` | 実際に取得・検証した日付付き全国 PBF の URL。latest 指定でも確定した日付付き URL を記録 |
| `metadata.sources` の `OpenStreetMap（日本・Geofabrik 配布）` の `license` | `ODbL-1.0` |
| `metadata.sources` の `OpenStreetMap（日本・Geofabrik 配布）` の `attribution` | `© OpenStreetMap contributors` |

元データと収録ノードの日時は必須とし、欠落・形式不正・収録ノードの日時が元データより新しい場合は生成・公開を止める。元データの日付はダウンロード日時ではない。同じ元データで再実行しても配布の `version` は変わる。

Release のタイトルとタグは同じ値とし、正式版は `osm-peaks-<version>`、開発版は `osm-peaks-dev-<version>` とする。取得・検証の契約は [共通の読み込みと公開](../../formats/manifest.md#読み込みと検証) に従う。

山頂データは正式版と開発版の参照先を分ける。Pages に掲載された地点データの一覧は [地点カタログ](../README.md#公開データのカタログ) を参照する。リポジトリ全体の `releases/latest` は使わない。

正式版と開発版で manifest の形式は共通とし、開発版の履歴 Release は Pre-release とする。アプリで配布先を選ぶときの扱いは [地点カタログの説明](../README.md#公開データのカタログ) を参照する。

正式版・開発版の検査はそれぞれの配布先で行う。Pages の自動更新と下書きの自動公開は開発版だけで行い、確認済みデータの手動公開・参照先の復旧は対象の配布先に行う。開発版を正式版の比較基準にせず、開発版の公開で正式版の参照先を更新しない。タグの衝突を防ぐため、`latest` と `dev-` で始まる版名は予約する。

Pages の初期設定、サイト全体の一覧・履歴の引き継ぎ、配置確認、参照先の復旧は [共通の Pages 配置](../../README.md#pages-への配置と参照先の復旧) を参照する。

### アプリ用データと確認・編集用の添付

アプリは `manifest.json` の参照先から、補足適用済みの `osm-peaks.json.gz` を取得する。アプリ側で補足をマージする必要はない。確認・編集や再現のため、補足適用前のオリジナルと補足 JSON も同じ Release に配布する。

| データ JSON | manifest | 内容 |
|---|---|---|
| `osm-peaks.json.gz` | `manifest.json` | 補足適用済みの地点データ。原本と補足の出典を保持する |
| `osm-peaks-source.json.gz` | `osm-peaks-source-manifest.json` | 補足適用前のオリジナル地点データ |
| `supplements.json.gz` | `supplements-manifest.json` | タグ・名称・読み・別名の補正、配布除外を指定する補足データ |

各 manifest は [共通の manifest フォーマット](../../formats/manifest.md) に従い、それぞれが指す JSON の `metadata` をそのままコピーする。地点 JSON の形式は [地点データフォーマット](../formats/data.md)、補足 JSON の形式は [補足データフォーマット](../formats/supplements-data.md) を参照する。

公開前に添付ファイルを再取得し、各 JSON と manifest の整合性、およびオリジナルに補足を適用して配布地点を再現できることを検証する。補足関連の添付がない過去の Release も検証できるが、一部だけそろっている場合は公開しない。

### 公開履歴

Pages に掲載した履歴は、正式版の [history.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks/history.json) と開発版の [history.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/history.json) に保存する。正式版の通常生成はこの履歴を更新しないため、正式版 Release の一覧ではない。項目と記録のタイミングは [共通の公開履歴](../../README.md#公開履歴) を参照する。

### 版4の旧形式データ

新しいアプリではこの旧形式を使わず、現行の地点データを利用する。

| 項目 | 内容 |
|---|---|
| 最新版の manifest | `https://shohei0205.github.io/yamamuki-data/peaks/manifest.json`（正式版。2026-10-21 からは `{"schemaVersion": 999}`）。開発版の `peaks-dev/` は新しく作らない |
| Release のタグ | `peaks-<version>`（正式版）、`peaks-dev-<version>`（開発版） |
| データ本体のファイル名 | `japan-mountains.json.gz` |
| 最後に作った正式版の Release | `peaks-20261007T084736Z-37595744056-1` |
| 件数 | 14,035 |
| 元データ | Geofabrik の日本全国 PBF（2026-10-06 版） |
| ライセンス | ODbL-1.0、© OpenStreetMap contributors |

版4 は新しく生成しない。公開済みの Release は残す。最新版の manifest（`peaks/manifest.json`）は、2026-10-21 に `{"schemaVersion": 999}` に置き換えて公開を終えた（Issue #48）。

版4 だけを読むアプリは、形式の版を見て読めないデータと判断できる。ファイルを消すと通信の失敗と区別できないため、消さずに置き換えた。このあとの公開でもカタログからサイトを作り直すので、置き換えた `peaks/manifest.json` は残る。置き換えに使ったワークフローは、1 回だけ使うものなので消した。

収録するのは OSM の `natural=peak` または `natural=volcano` の名前付きノード。way・relation と名前のないノードは含めない。名前・標高・読み・別名・解説リンクの取り方は「配るもの」の OSM タグの説明を参照する。JSON 本体の制約は [地点データの変更履歴の版4](../formats/data.md#版42026-10-0326) にある。

Pages の `peaks/` の manifest・履歴は、版4 だけを読むアプリに更新を促すために残す。main / dev からの公開、地点カタログからの削除、テストデータの公開でも、既存の互換データを引き継いで同じ内容を配置する。osm-peaks の更新で互換データを上書きしない。互換データは地点カタログには含めない。存在しない互換データを新たに作成することはない。

## 作り方

### フォルダ構成

```text
osm-peaks/
  README.md         収録条件・生成・公開手順
  scripts/          元データの取得・生成・検査・公開
  tests/            生成・検査・公開処理のテスト
  supplements.json  タグ・名称・読み・別名・配布除外の補足
  notes/            タグごとの採用方針と確認記録
```

利用開始時に、リポジトリの Settings → Pages → Build and deployment の Source を **GitHub Actions** にし、`github-pages` 環境の配置元として `main` と `dev` を許可する。

[地点 / OSM山頂：Release 生成・検査](https://github.com/shohei0205/yamamuki-data/actions/workflows/publish-data.yml) は、毎月 1 日の UTC 03:23（日本時間 12:23）に `main` で正式版を生成する。手動でも実行できる。

正式版は Release を作るところまでで、Pages には公開しない。利用する側で取得する正式版の Release を選ぶ運用としている（[#44](https://github.com/shohei0205/yamamuki-data/issues/44)）。開発版（`dev`）は今までどおり、検査に通ったら Pages の `points/osm-peaks-dev/` に公開する。

手動で動かすときは Actions の「地点 / OSM山頂：Release 生成・検査」→「Run workflow」で正式版なら `main`、開発版なら `dev` を選ぶ。この2つ以外のブランチでは公開しない。公開ジョブの `GITHUB_TOKEN` に `contents: write`・`pages: write`・`id-token: write` を付与し、追加のトークンは使わない。

1. 単体テストと、小さな PBF による生成テストを行う。
2. Geofabrik の `japan-latest.osm.pbf` から日付付き URL を確定する。取得対象日を指定した場合は、その日付の URL を直接使い、全国データを取得する。途中で切れたら同じ版の続きから再開し、配布元の MD5 と照合する。Overpass API は使わない。
3. `osmium tags-filter` で対象ノードだけを抽出し、配布ファイルと manifest を作る。
4. 前回の版を取得・検証し、全国と地域別の件数、元データの日時、形式の版を比較する。前回の版は、正式版では「要確認」でない最新の正式版 Release（下書きと Pre-release を除き、タグの版名が最も新しいもの）、開発版では Pages の `points/osm-peaks-dev/manifest.json` が指す Release とする。件数・前回との差・検査結果・圧縮サイズ・元データの日時・SHA-256 を、Release の説明と Actions の実行概要に記録する。
5. 正式版は、両ファイルを添付した Release を作る。ただし、前回公開版の取得・検証・比較でエラーが起きた場合は、理由を記載した下書きに残し、手動確認を待つ。「最新」の印（Latest）は付けない。要確認の版も Release にし、本文の先頭の「要確認」の見出しの下に理由を書く。Pages は更新しない。
6. 開発版は、両ファイルを下書きの Pre-release に添付する。検査に合格した場合だけ、別の公開ジョブが下書きのファイルをダウンロードし、再検証して公開する。履歴版を公開後、開発版の最新版 manifest を Pages で更新する。

Actions の各ステップでは、時刻付きで処理の開始・完了をログに出す。Python の出力はためずに随時表示する。

- ダウンロード中：受信が進んでいる間は約15秒ごとに取得量・割合・平均速度・経過秒数を表示する。接続待ち、再開位置、再試行、取得済みファイルの再利用、MD5 の照合も記録する。通信が止まっている間は次の受信またはタイムアウトまで進捗行は増えない。
- 通信の診断：HTTP メソッド、要求 URL、各転送先と HTTP ステータス、最終応答 URL、Range、経過時間を記録する。応答ヘッダーは Location・Content-Length・Content-Range・Content-Type・Retry-After・Date・Age・Cache-Control・Cache-Status・Via・Server に限定し、本文は記録しない。
- 取得失敗時：処理段階、試行回数、対象 URL、保存先、例外の種類・内容、残り時間と次の待ち時間を表示する。最後の試行でも失敗内容を残し、試行回数の上限・時間制限・再試行対象外のどれで終了したかを記録する。
- 生成中：元データの日時、osmium の抽出進捗、読み取り5,000件ごとの件数、圧縮前後のサイズ、SHA-256 と manifest の作成、保存完了を表示する。
- 公開時：Release（開発版は下書き）の作成・アップロード開始、アップロード完了、公開完了を表示する。

失敗した実行では、それ以前の公開済み Release を書き換えない。開発版でアップロード途中に失敗した下書きは公開されず、残った下書きは手動で削除できる。正式版でアップロード途中に失敗した Release は、検証済みの配布版として選ばない。再実行は新しいタグを作る。配布ファイルの再現性のため、gzip ヘッダーに生成日時やファイル名を含めない。

ダウンロードは通信待ち60秒、最大6回（15秒間隔）の試行、全体60分の制限を設ける。Actions は取得ステップ65分、生成ジョブ90分、公開ジョブ15分で打ち切る。途中ファイルは元データの MD5 ごとに保存し、別の版のデータをつなげない。Range に対応しない応答では先頭から取り直す。通信待ちや再試行で失敗した場合は、同じコマンドを再実行すれば途中ファイルを再利用できる（配布元が同じ版の場合）。Actions の別実行には途中ファイルを引き継がない。

### 抽出・変換の規則

Geofabrik の日本全国の OSM データから、`natural=peak` または `natural=volcano` の名前付きノードを抽出する。way・relation と名前のないノードは含めない。収録する地点の `type` は山頂を表す `peak` とし、火山ノードも同じ種別で出力する。名前は前後の空白を除き、`name:ja`、`name` の順に使う（`name:ja` だけのノードも含む）。

ふりがな・別名・解説リンク・標高は、以下の OSM タグから変換する。

- `nameReading`: `name:ja-Hira` のふりがな。前後の空白を除く。未登録・空欄は `null`。推測による補完はしない。補足 JSON に指定があれば、そのよみがなを適用する。
- `aliases`: `alt_name:ja`、`alt_name` の順に集めた別名の配列。セミコロンで分割し、前後の空白・空欄・表示名と同じ名前・重複を除く。未登録は `[]`。
- `wikipediaUrl`: `wikipedia` の「言語:記事名」を HTTPS URL に変換した解説へのリンク。日本語・空白・記号を URL 用に変換し、記事内の節にも対応する。未登録・形式不正は `null`。
- `wikidataUrl`: `wikidata` の項目 ID（例: `Q39231`）から作った HTTPS リンク。未登録・形式不正は `null`。

- `elevationM`: `ele` から取得する。値がない、または解釈できないときは `null`。カンマ、m・ft などの単位、セミコロン区切りの先頭値に対応する。

生成ごとの件数・圧縮前後のサイズ・元データの日時・SHA-256・検査結果は、Actions の実行概要と Release の説明で確認する。各版の manifest にも件数・サイズ・日時・ハッシュを記録する。

圧縮後のサイズが 5,000,000 バイトを超える場合は公開を止め、分割を検討する。データが空、ID が重複、座標が不正、元データの日付が取得できない場合も公開しない。

### 自動公開と確認待ち

次のいずれかに当たる版は「要確認」とする。正式版は Release にしたうえで本文の先頭に「## 要確認」と理由を書き、次の生成の比較基準にしない。開発版は下書きのまま残し、Pages の最新版参照を更新しない。確認待ちは通常の処理結果として扱い、通知用の処理は追加しない。

| 確認条件 | 初期の基準 |
|---|---|
| 初回 | 通常の公開済み Release がなく、比較対象がない |
| 全国の最低件数 | 10,000 件未満 |
| 全国の減少 | 前回公開版から20%以上減少 |
| 一部地域の減少 | 前回20件以上あった緯度経度1度の区画で、20%以上減少 |
| 山頂の最新編集日時が同じ | `latestPointTimestamp` が前回公開版と同じ日時。PBF の基準日時や生成した版が新しくても下書きに残す |
| 日時の逆戻り | 元データの日時が前回公開版より古い |
| 形式の変更 | `schemaVersion` が前回公開版と異なる |
| 比較不能 | 前回公開版の取得・検証ができない。通信失敗や権限不足を初回扱いしない |

地域の判定は都道府県ではなく、緯度・経度をそれぞれ切り下げた1度区画で行う。例えば北緯35度・東経139度の区画は、北緯35度以上36度未満・東経139度以上140度未満。細かな境界付近の変化で保留が多ければ、実績を見て基準を調整する。比較対象は同じ配布先の公開済みの版とし、確認待ちの版（開発版の下書き、正式版の要確認の Release）は基準にしない。

件数の減少などは警告として手動で承認できる。ただしファイル破損、SHA-256・サイズ・件数の不一致、空データ、不正な ID・座標・データ形式は公開不可。手動でも同じ検証を行う。生成直後に不正が見つかった場合は下書きも作らない。

### 正式版と開発版の実行方法

月次実行は `main` から正式版（`stable`）の Release を作る。手動実行は Actions の「地点 / OSM山頂：Release 生成・検査」→「Run workflow」でブランチを選ぶ。

| 実行元ブランチ | 配布先 | 生成・公開に使う処理 |
|---|---|---|
| `main` | 正式版（`stable`）。Release まで | `main` のコード |
| `dev` | 開発版（`dev`、Pre-release）。Pages にも公開 | `dev` のコード |

配布先の選択欄は設けず、ブランチから自動判定する。手動公開・参照先復旧も同じ規則で動く。その他のブランチやタグからは公開しない。ブランチへの push だけではデータを生成しない。`dev` の月次実行はなく、開発版が必要なときに手動実行する。

利用開始には、このワークフローを `main` と `dev` の両方に配置する。開発中の生成処理は `dev` で確認し、正式版に採用するときは変更を `main` へ取り込む。

生成は配布先ごとのグループで直列化する。公開・手動復旧・Pages の配置は正式版と開発版で共通のグループを使い、他の参照先の引き継ぎから配置まで順番に行う。開発版から正式版への自動昇格は行わず、正式版が必要なときは `main` を選んで生成する。開発版の初回は下書きに残るので、確認して手動公開する。

### 取得対象日を指定する

「地点 / OSM山頂：Release 生成・検査」の Run workflow で、`source_date` に `YYYY-MM-DD` を入力すると、`latest` の転送を使わず日付付き URL を直接取得する。例えば `2026-09-29` は `https://download.geofabrik.de/asia/japan-260929.osm.pbf` になる。空欄または月次実行では`latest` を使う。入力欄を表示するため、main 側の入口にも同じ入力項目が必要。

Actions の実行日はデータの配布日とは限らず、当日分はまだ存在しない場合がある。配布済みの日付を指定する。対象が404の場合は失敗とし、別の日付への自動切り替えはしない。サイズと日付付き URL の MD5 を照合し、別の版の途中ファイルを混ぜない。古い日付を指定しても、公開前の日時・件数・同一更新日時の検査はそのまま行う。

ローカルでは `points/osm-peaks/` 内で次のように指定する。

```bash
python -u scripts/download_source.py --source-date 2026-09-29
```

### 全国 PBF の再利用

Actions では取得前に配布元の日付付き URL・サイズ・MD5 を確認し、配布日と MD5 をキーに保存済みの全国 PBF を復元する。復元後も配布元のサイズと MD5 を照合し、一致した場合だけ使う。取得元の記録は検証後に作り直す。`latest` 指定でも実際の配布日に固定して取得するため、実行日だけで同じデータと判断しない。同じ日付でも MD5 が変われば別のキャッシュを使う。

キャッシュがない・復元に失敗した・ファイルが破損した場合は Geofabrik から取得する。検証済みの PBF は生成処理より前に保存するので、後続の生成や公開に失敗しても次回に再利用できる。キャッシュの保存に失敗してもデータの生成は続ける。復元は完全一致のキーだけを指定し、別の日付を代用しない。

これは Geofabrik からの大容量転送を減らす仕組みで、GitHub のキャッシュからの転送は発生する。キャッシュは容量や利用状況により削除されるため、永続保存ではない。ブランチ間の共有範囲は GitHub のキャッシュ規則に従う。詳細は [GitHub のキャッシュの説明](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching) を参照する。

### Summary で実行内容を確認する

ブランチ・実行者・入力項目・公開先などの共通表示は [実行概要の確認](../../README.md#実行概要の確認) を参照する。山頂の生成では取得対象日を表示し、空欄なら latest を使うことを明記する。

実際の件数などは「データの検査結果（生成後）」と「データの検査結果（公開前の再検査）」で区別する。生成が完了すると「生成したリリース」に、GitHub が返した実際の URL・タグ・下書き保留か自動公開へ進むかを表示する。下書きの `untagged-...` URL もそのまま使う。

### 確認済みの下書きを手動公開する

[共通の手動公開手順](../../README.md#確認済みデータの公開) に従い、確認した山頂 Release の URL またはタグを指定する。正式版のタグは `osm-peaks-<version>`、開発版は `osm-peaks-dev-<version>` とする。

山頂の手動公開では下書きの 2 ファイルを取得・再検証し、検査結果に記録された SHA-256（明示した場合は入力値）と一致した場合に、件数の減少などの警告を承認して公開する。検査結果の SHA-256 が欠けている場合は、確認した値を明示する。自動公開では生成ジョブが渡す SHA-256 を必須とする。

再ダウンロード・再生成・アセットの差し替えは行わない。前回公開版との比較もやり直すため、通信失敗や前回ファイルの破損で比較できない場合は手動公開も停止する。形式変更時はアプリ側が新形式に対応しているかも確認する。

#### Release を削除した後に公開を再開する

公開サイトの manifest が指す山頂 Release を削除した場合、手動公開で「Pages の配布サイトを初期化」を指定する。前回の Release が存在しない場合に限り、前回との比較を省いて確認済みデータの公開を再開する。新しいデータ本体のサイズ・ハッシュ・件数などの検査は行う。通信失敗やデータ破損は初期化を指定しても停止する。他のデータと配布先は引き継ぐ。

### 手元での生成

以下のコマンドは `points/osm-peaks/` を作業ディレクトリにして実行する。Python 3.12 と osmium-tool が必要（Ubuntu では `sudo apt-get install osmium-tool`）。

```bash
# リポジトリのルートから移動する。
cd points/osm-peaks
python -u scripts/download_source.py --source-date YYYY-MM-DD
python -u scripts/download_source.py --region far-eastern-fed-district \
  --source-date YYYY-MM-DD --output build/far-eastern-fed-district.osm.pbf
python scripts/build_data.py build/japan-latest.osm.pbf \
  --version local-20260930 --output-dir dist
```

開発版を手元で生成するときは、生成コマンドに `--channel dev` を指定する。省略時は `RELEASE_CHANNEL` の値、未設定なら正式版を使う。取得 URL のリポジトリは `GH_REPO`、未設定なら `GITHUB_REPOSITORY`、どちらも未設定なら `shohei0205/yamamuki-data` を使う。

取得時に PBF の隣へ `<PBF のファイル名>.source.json` を保存し、URL・サイズ・MD5 を記録する。取得済みファイルを再利用した場合も記録を作る。生成時に記録と PBF を照合し、日付付き URL を manifest の `metadata.sources` の `OpenStreetMap（日本・Geofabrik 配布）` の `source` に引き継ぐ。記録の欠落や不一致は生成を止める。既存の PBF に記録がない場合は、同じ対象日で取得コマンドを再実行すると、内容が一致すれば再ダウンロードせずに記録を作れる。

元データの取得には数 GB の通信量と空き容量が必要。元データは `points/osm-peaks/build/`、配布ファイルは `points/osm-peaks/dist/` に保存し、どちらも git に入れない。

### 全部入り JSON の生成

山以外も含む確認用 JSON の生成方法は、[全部入り JSON の生成手順](../README.md#山以外も含む全部入り-json-を作る) を参照する。
## 地点の補足対応表

[supplements.json](supplements.json) 一つでタグと補正を管理する。`id` で地点を指定し、補足適用前のオリジナルへタグの追加・削除、表示名・読みの補正、別名の追加・削除、配布対象からの除外を適用する。全国版の生成スクリプトでは `--supplements` で別の補足 JSON を指定できる。不正値・未知の ID・重複 ID・想定値の不一致はエラーとし、別の地点へ自動で付け替えない。

補足 JSON の配列は著書順を表さない。`note` は任意の管理用メモで、ビューアーで保存すると補足適用前の地点名を自動設定する。修正理由と管理用メモは補足 JSON に保持し、マージ済み地点 JSON には含めない。

共通形式は [補足データフォーマット](../formats/supplements-data.md)、編集操作は [ビューアの編集手順](../README.md#補足を編集する) を参照する。

### 出典の管理

補足の出典・ライセンス・帰属表示・加工内容は補足 JSON の `metadata.sources` に記録する。マージ済み地点 JSON には原本と補足の出典をまとめ、その `metadata` を配布 manifest にコピーする。出典は `note` に記載しない。項目と制約は [metadata フォーマット](../../formats/metadata.md) を参照する。

### タグごとの記録

タグごとの記録は `notes/<タグ名>.md` に置く。新しいタグを追加するときも同じ命名にする。

以下のタグを補足 JSON に収録する。

| タグ | 読み | 記録 |
|---|---|---|
| 日本百名山 | にほんひゃくめいざん | [日本百名山の作成手順と採用地点](notes/日本百名山.md) |

### 対象 ID と管理用メモの補完

```bash
PYTHONPATH=points/osm-peaks python -m scripts.supplements points/osm-peaks/supplements.json --points osm-peaks-source.json.gz --output points/osm-peaks/build/supplements.json
```

`id` を指定した行は、その指定を保って検証する。`id` も osmId もない行では `note` の山名・別名を完全一致で照合して osmId を補う。osmId があり `note` が空なら地点名をメモに補う。旧形式で入力した `osmId` は、保存時に `id: "osm:<ノード番号>"` へ変換する。候補が複数ある場合は未解決とし、終了コード 1 を返す。形式の不備は終了コード 2。入力は変更せず、補足・補正・出典情報を出力へ引き継ぐ。補完には補足適用前の元データを使う。


## 出典とライセンス

| 用途 | 出典 |
|---|---|
| 山頂の一覧・山名・読み・別名・座標・標高・分類 | [OpenStreetMap](https://www.openstreetmap.org/) の名前付きノード。Geofabrik の [日本全国の PBF](https://download.geofabrik.de/asia/japan.html) から抽出する |
| Wikipedia・Wikidata へのリンク | OpenStreetMap ノードの `wikipedia`・`wikidata` タグ |
| 日本百名山の山名一覧 | [Wikipedia「日本百名山」](https://ja.wikipedia.org/w/index.php?title=日本百名山#著書順の山の一覧)（CC BY-SA 4.0）。帰属表示・加工内容は補足 JSON の `metadata.sources` を参照 |
| 日本百名山の地点照合・名称・読み・別名の補正 | 国土地理院の山岳一覧・最適化ベクトルタイル・自然地名を加工して作成（[PDL-1.0](https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html)）。元資料・帰属表示・加工内容は補足 JSON の `metadata.sources` を参照 |

OpenStreetMap 由来のデータは **© OpenStreetMap contributors** を表示し、[Open Database License（ODbL）1.0](https://opendatacommons.org/licenses/odbl/1-0/) に従って利用・再配布する。[OpenStreetMap の著作権とライセンス](https://www.openstreetmap.org/copyright)を参照する。

出典・利用条件の一覧は、[ルート README のライセンス](../../README.md#ライセンス)を参照する。

## テスト

全部入り生成のテストも動かすため、先に `python -m pip install -r requirements-all.txt` を実行する。

Python 3.12 と osmium-tool を使い、単体テストと小さな PBF による生成テストを行う。SVG の対応表・JSON への内蔵・倍率・未対応要素の拒否・公開前の再検査も単体テストで確認する。CIの「共通：データ処理のテスト」（`.github/workflows/test.yml`）の「地点 / OSM山頂」ジョブで push・PR 時に実行する。

テストは `points/osm-peaks/` 内で、外部通信を行わず次のコマンドで実行できる。osmium がない場合は PBF を使うテストだけをスキップする。Actions では osmium を入れてすべて実行する。

```bash
python -m unittest discover -s tests -v
```

追加 PBF の取得はそれぞれ全体15分、Actions の取得ステップ20分で打ち切る。日本 PBF の配布日を追加 PBF に指定し、その日の追加 PBF が未配布なら失敗とする。日付を自動でずらして生成しない。

追加 PBF も取得元の地域・配布日・MD5 ごとにキャッシュし、復元後に照合する。2026-10-10 時点の配布ページの目安では、ロシア極東は約 371 MB。日本 PBF に加えて約 371 MB の取得・保存が必要になる。実際のサイズと取得時間はログで確認する。

生成コマンドは追加 PBF も必須とする。保存先を変える場合は `--far-eastern-pbf` で指定する。全取得元の URL・サイズ・MD5・基準日時は manifest の `sourcePbfs` に記録する。日本 PBF の URL は `metadata.sources` の `OpenStreetMap（日本・Geofabrik 配布）`、極東連邦管区 PBF の URL は `OpenStreetMap（ロシア極東連邦管区・Geofabrik 配布）` にも記録する。どちらにもライセンスと帰属表示を付け、地点 JSON と manifest に同じ出典を記録する。

### 作業ブランチでの生成確認

Actions の手動実行で作業ブランチを選ぶと、生成と検査まで実行し、配布ファイルを artifact に 7 日間保存する。Release の作成と Pages の更新は行わない。生成された manifest の URL は確認用で、配布先への公開を意味しない。

比較に使う前回公開版を取得できない場合や、比較処理で例外が起きた場合も、生成物の検証に通っていれば理由付きの下書き Release に保存する。開発版の自動公開は行わない。作業ブランチでの生成確認では前回公開版との比較を行わない。

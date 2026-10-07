# 山頂データ

[地点データの共通仕様に戻る](../README.md)

全国の山頂データの形式、取得先、生成・検査・公開の手順をまとめる。

| 配布先 | 最新版の manifest | 公開履歴 |
|---|---|---|
| 正式版 | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks/manifest.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks/manifest.json) | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks/history.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks/history.json) |
| 開発版 | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/manifest.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/manifest.json) | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/history.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/history.json) |

データ本体（`osm-peaks.json.gz`）は、各 manifest の `downloadUrl` から取得する。

## 配るもの

Geofabrik の日本全国の OSM データから、`natural=peak` または `natural=volcano` の名前付きノードを抽出する。日本全体を `osm-peaks.json.gz` 1 ファイルにまとめる。way・relation と名前のないノードは含めない。収録する地点の `type` は山頂を表す `peak` とし、火山ノードも同じ種別で出力する。名前は前後の空白を除き、`name:ja`、`name` の順に使う（`name:ja` だけのノードも含む）。

地点ごとの JSON の形式は [地点データの共通仕様](../README.md#地点の形式)を参照する。OSM ノード ID の昇順に並べる。ふりがな・別名・解説リンク・標高は、以下の OSM タグから変換する。

- `nameReading`: `name:ja-Hira` のふりがな。前後の空白を除く。未登録・空欄は `null`。推測による補完はしない。
- `aliases`: `alt_name:ja`、`alt_name` の順に集めた別名の配列。セミコロンで分割し、前後の空白・空欄・表示名と同じ名前・重複を除く。未登録は `[]`。
- `wikipediaUrl`: `wikipedia` の「言語:記事名」を HTTPS URL に変換した解説へのリンク。日本語・空白・記号を URL 用に変換し、記事内の節にも対応する。未登録・形式不正は `null`。
- `wikidataUrl`: `wikidata` の項目 ID（例: `Q39231`）から作った HTTPS リンク。未登録・形式不正は `null`。

- `elevationM`: `ele` から取得する。値がない、または解釈できないときは `null`。カンマ、m・ft などの単位、セミコロン区切りの先頭値に対応する。

生成ごとの件数・圧縮前後のサイズ・元データの日時・SHA-256・検査結果は、Actions の実行概要と Release の説明で確認する。各版の manifest にも件数・サイズ・日時・ハッシュを記録する。

圧縮後のサイズが 5,000,000 バイトを超える場合は公開を止め、分割を検討する。データが空、ID が重複、座標が不正、元データの日付が取得できない場合も公開しない。

## 置き場所

[Releases](https://github.com/shohei0205/yamamuki-data/releases) に、次の 2 ファイルを公開する。

- [https://shohei0205.github.io/yamamuki-data/points/osm-peaks/manifest.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks/manifest.json)
- データ本体は manifest の `downloadUrl` から取得する。

manifest の項目・型・版の履歴は [リポジトリ共通の manifest.json](../../README.md#manifestjson) を参照する。

山頂データでは次の値・取得方法を使う。

| 項目 | 山頂データでの値・取得方法 |
|---|---|
| `version` | 生成時の UTC 日時・Actions 実行 ID・再実行番号を連結 |
| `fileName` | `osm-peaks.json.gz` |
| `downloadUrl` | 正式版は `osm-peaks-<version>`、開発版は `osm-peaks-dev-<version>` の Release の gzip ファイル |
| `pointCount` | 収録した名前付き山頂ノードの件数 |
| `sourceTimestamp` | 全国 PBF ヘッダーの `osmosis_replication_timestamp` |
| `latestPointTimestamp` | 収録する山頂ノードの OSM 最終編集日時の最大値。収録対象外のノードは集計しない |
| `sourceUrl` | 実際に取得・検証した日付付き全国 PBF の URL。latest 指定でも確定した日付付き URL を記録 |
| `license` | `ODbL-1.0` |
| `attribution` | `© OpenStreetMap contributors` |

元データと収録ノードの日時は必須とし、欠落・形式不正・収録ノードの日時が元データより新しい場合は生成・公開を止める。元データの日付はダウンロード日時ではない。同じ元データで再実行しても配布の `version` は変わる。

Release のタイトルとタグは同じ値とし、正式版は `osm-peaks-<version>`、開発版は `osm-peaks-dev-<version>` とする。アプリは manifest の `downloadUrl` から対応するファイルを取得する。最新版の参照用 manifest は GitHub Pages に置き、データ本体は各版に保存する。アプリでは展開前にサイズと SHA-256 を検証する。既存アプリへの読み込み機能の組み込みは、アプリ側の別作業となる。

山頂データは正式版と開発版の参照先を分ける。公開済みの地点データ一覧は [地点カタログ](../README.md#公開データのカタログ) を参照する。リポジトリ全体の `releases/latest` は使わない。

| 配布先 | Pages のパス | データ本体を置くタグ |
|---|---|---|
| 正式版（`stable`） | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks/manifest.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks/manifest.json) | `osm-peaks-<version>` |
| 開発版（`dev`） | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/manifest.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/manifest.json) | `osm-peaks-dev-<version>` |

開発版の manifest は [https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/manifest.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/manifest.json)、本体は manifest の `downloadUrl` から取得する。正式版と開発版で manifest の形式は共通とする。配布先はアプリ側で選ぶ。開発版が無い・取得できない場合に正式版へ自動で切り替えない。開発版の履歴 Release には GitHub の Pre-release を付ける。

件数・更新日時の比較、初回の手動確認、異常時の下書き保留、手動公開、参照先の復旧は配布先ごとに独立して行う。開発版を正式版の比較基準にせず、開発版の公開で正式版の参照先を更新しない。タグの衝突を防ぐため、`latest` と `dev-` で始まる版名は予約する。

履歴版の公開後、Actions で manifest のサイトを生成し、`upload-pages-artifact` と `deploy-pages` で GitHub Pages に直接配置する。ソースは `main`・`dev` に置き、配布専用ブランチや生成物のコミットは作らない。

サイトには更新処理用の `catalog.json`（schemaVersion 1、`manifests` にパスと manifest の対応を保存）も置く。既存の一覧からサイト全体を作り直し、選択した配布先だけを置き換えるため、他方の配布先・将来の別種別を維持できる。アプリはこれまでどおり各パスの `manifest.json` を使い、一覧を読む必要はない。

自動公開と手動公開は、正式版・開発版共通の `publish-data-pages` グループで直列化する。公開先の一覧と配置した内容が一致するまで最大55秒待ち、反映を確認してからジョブを完了する。一覧の取得や検証に失敗した場合は配置しない。404 の場合も通常の公開では停止し、初期化を明示した初回の手動公開だけが一覧を新しく作る。

ファイルを削除してから上げ直す時間は生じない。キャッシュにより更新前の manifest が返る場合はあるが、履歴版を残すため、その manifest でも対応するデータ本体を取得できる。アプリは通信や検証に失敗したら保存済みデータを維持して再試行する。

参照先の更新に失敗した場合は、公開済みの同じ Release の URL またはタグを手動公開ワークフローに指定して復旧する。履歴版のデータ本体は再生成・再アップロードしない。公開済みの履歴 Release には不変化を適用できる。

山頂の生成時は地点の `id` と `osmId` の両方を出力し、OSM ノード ID の数値順に並べる。取得できない任意情報は`null`・`[]` として出力する。検査では任意項目の省略を許可し、`osmId` がある場合は `id` との一致を確認する。

## 公開履歴

Pages の [https://shohei0205.github.io/yamamuki-data/points/osm-peaks/history.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks/history.json)（正式版）と [https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/history.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/history.json)（開発版）に、最新版として公開した記録を古い順で残す。ファイルは `schemaVersion: 1` と `entries` の配列を持つ。

| 項目 | 内容 |
|---|---|
| `kind` | 通常の公開は `publication`。履歴記録を始める時点の既存の最新版は `snapshot` |
| `publishedAt` | 公開処理で履歴を生成した UTC 日時。配置の完了日時ではない。既存の最新版は日時を推測せず `null` |
| `version` | データの版 |
| `releaseUrl` | データ本体を保存した Release のページ |
| `downloadUrl` | データ本体の取得 URL |
| `actionsRunUrl` | 公開した Actions の実行 URL（再実行番号付き）。既存の最新版や手元での生成は `null` |

再公開や古い版への差し戻しも、その都度追加する。下書きで保留された場合や、Pages の配置に進む前に失敗した場合は公開サイトに履歴を追加しない。Pages の配置後に反映確認だけが失敗した場合は、配置された履歴が残る。

履歴も `catalog.json` に含め、次回の配置で他の配布先の履歴と一緒に引き継ぐ。配置後は manifest と履歴の両方が生成内容と一致することを確認する。各版のアセット本体は Release に残し、Pages に複製しない。記録を始める前の公開日時や再公開の経緯は復元しない。

## 作り方

利用開始時に、リポジトリの Settings → Pages → Build and deployment の Source を **GitHub Actions** にし、`github-pages` 環境の配置元として `main` と `dev` を許可する。

[地点 / OSM山頂：生成・検査](https://github.com/shohei0205/yamamuki-data/actions/workflows/publish-data.yml) は、毎月 1 日の UTC 03:23（日本時間 12:23）に `main` で動く。GitHub の混雑で開始が遅れる場合がある。

手動で動かすときは Actions の「地点 / OSM山頂：生成・検査」→「Run workflow」で正式版なら `main`、開発版なら `dev` を選ぶ。この2つ以外のブランチでは公開しない。公開ジョブの `GITHUB_TOKEN` に `contents: write`・`pages: write`・`id-token: write` を付与し、追加のトークンは使わない。

1. 単体テストと、小さな PBF による生成テストを行う。
2. Geofabrik の `japan-latest.osm.pbf` から日付付き URL を確定する。取得対象日を指定した場合は、その日付の URL を直接使い、全国データを取得する。途中で切れたら同じ版の続きから再開し、配布元の MD5 と照合する。Overpass API は使わない。
3. `osmium tags-filter` で対象ノードだけを抽出し、配布ファイルと manifest を作る。
4. Pages の選択した配布先の前回 manifest（正式版は `points/osm-peaks/manifest.json`、開発版は `points/osm-peaks-dev/manifest.json`）を取得・検証し、全国と地域別の件数、元データの日時、形式の版を比較する。件数・前回との差・検査結果・圧縮サイズ・元データの日時・SHA-256 を、下書きの説明と Actions の実行概要に記録する。
5. 両ファイルを下書き Release に添付する。検査に合格した場合だけ、別の公開ジョブが下書きのファイルをダウンロードし、再検証して公開する。履歴版を公開後、その配布先の最新版 manifest を更新する。

Actions の各ステップでは、時刻付きで処理の開始・完了をログに出す。Python の出力はためずに随時表示する。

- ダウンロード中：受信が進んでいる間は約15秒ごとに取得量・割合・平均速度・経過秒数を表示する。接続待ち、再開位置、再試行、取得済みファイルの再利用、MD5 の照合も記録する。通信が止まっている間は次の受信またはタイムアウトまで進捗行は増えない。
- 通信の診断：HTTP メソッド、要求 URL、各転送先と HTTP ステータス、最終応答 URL、Range、経過時間を記録する。応答ヘッダーは Location・Content-Length・Content-Range・Content-Type・Retry-After・Date・Age・Cache-Control・Cache-Status・Via・Server に限定し、本文は記録しない。
- 取得失敗時：処理段階、試行回数、対象 URL、保存先、例外の種類・内容、残り時間と次の待ち時間を表示する。最後の試行でも失敗内容を残し、試行回数の上限・時間制限・再試行対象外のどれで終了したかを記録する。
- 生成中：元データの日時、osmium の抽出進捗、読み取り5,000件ごとの件数、圧縮前後のサイズ、SHA-256 と manifest の作成、保存完了を表示する。
- 公開時：下書き Release の作成・アップロード開始、アップロード完了、公開完了を表示する。

失敗した実行では、それ以前の公開済み Release を書き換えない。アップロード途中に失敗した下書きは公開されず、残った下書きは手動で削除できる。再実行は新しいタグを作る。配布ファイルの再現性のため、gzip ヘッダーに生成日時やファイル名を含めない。

ダウンロードは通信待ち60秒、最大6回（15秒間隔）の試行、全体60分の制限を設ける。Actions は取得ステップ65分、生成ジョブ90分、公開ジョブ15分で打ち切る。途中ファイルは元データの MD5 ごとに保存し、別の版のデータをつなげない。Range に対応しない応答では先頭から取り直す。通信待ちや再試行で失敗した場合は、同じコマンドを再実行すれば途中ファイルを再利用できる（配布元が同じ版の場合）。Actions の別実行には途中ファイルを引き継がない。

### 自動公開と確認待ち

通常は生成・検査・公開まで自動で進む。次のいずれかに当たる場合は下書きのまま残し、選択した配布先の最新版参照を更新しない。確認待ちは通常の処理結果として扱い、通知用の処理は追加しない。

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

地域の判定は都道府県ではなく、緯度・経度をそれぞれ切り下げた1度区画で行う。例えば北緯35度・東経139度の区画は、北緯35度以上36度未満・東経139度以上140度未満。細かな境界付近の変化で保留が多ければ、実績を見て基準を調整する。比較対象は常に同じ配布先の最新版参照が指す公開済みの版とし、確認待ちの下書きは基準にしない。

件数の減少などは警告として手動で承認できる。ただしファイル破損、SHA-256・サイズ・件数の不一致、空データ、不正な ID・座標・データ形式は公開不可。手動でも同じ検証を行う。生成直後に不正が見つかった場合は下書きも作らない。

### 正式版と開発版の実行方法

月次実行は `main` から正式版（`stable`）を生成する。手動実行は Actions の「地点 / OSM山頂：生成・検査」→「Run workflow」でブランチを選ぶ。

| 実行元ブランチ | 配布先 | 生成・公開に使う処理 |
|---|---|---|
| `main` | 正式版（`stable`） | `main` のコード |
| `dev` | 開発版（`dev`、Pre-release） | `dev` のコード |

配布先の選択欄は設けず、ブランチから自動判定する。手動公開・参照先復旧も同じ規則で動く。その他のブランチやタグからは公開しない。ブランチへの push だけではデータを生成しない。`dev` の月次実行はなく、開発版が必要なときに手動実行する。

利用開始には、このワークフローを `main` と `dev` の両方に配置する。開発中の生成処理は `dev` で確認し、正式版に採用するときは変更を `main` へ取り込む。

生成は配布先ごとのグループで直列化する。公開・手動復旧・Pages の配置は正式版と開発版で共通のグループを使い、他の参照先の引き継ぎから配置まで順番に行う。開発版から正式版への自動昇格は行わず、正式版が必要なときは `main` を選んで生成する。両方の初回は下書きに残るので、確認して手動公開する。

### 取得対象日を指定する

「地点 / OSM山頂：生成・検査」の Run workflow で、`source_date` に `YYYY-MM-DD` を入力すると、`latest` の転送を使わず日付付き URL を直接取得する。例えば `2026-09-29` は `https://download.geofabrik.de/asia/japan-260929.osm.pbf` になる。空欄または月次実行では`latest` を使う。入力欄を表示するため、main 側の入口にも同じ入力項目が必要。

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

生成・検査では取得対象日、手動公開では入力した URL またはタグ・SHA-256・確認内容と公開理由・Pages 初期化の指定を、処理の冒頭に Summary へ表示する。共通してブランチ・イベント・実行者・実行回数・配布先も表示する。空欄の場合は latest の取得・SHA-256 の自動取得・理由の未記入を明記する。

実際の件数などは「データの検査結果（生成後）」と「データの検査結果（公開前の再検査）」で区別する。生成が完了すると「生成したリリース」に、GitHub が返した実際の URL・タグ・下書き保留か自動公開へ進むかを表示する。下書きの `untagged-...` URL もそのまま使う。公開が完了した場合は公開ジョブの Summary に公開後の URL も表示する。単体テストのダミーデータは Summary に出さない。過去の実行に保存された Summary はこの変更では書き換わらない。

### 確認済みの下書きを手動公開する

1. Releases の下書きにある検査結果と必要なデータの差分を確認する。
2. データ種別共通の Actions「共通：確認済みデータを公開」→「Run workflow」で正式版なら `main`、開発版なら `dev` を選ぶ。
3. `tag` 欄に確認した Release ページの URL を貼り付け、確認内容・公開理由は必要に応じて入力する（空欄でも実行可能）。`untagged-...` を含む下書きの URL も使える。対象タグ（正式版は `osm-peaks-<version>`、開発版は `osm-peaks-dev-<version>`）も指定できる。`sha256` 欄は空欄でよく、Release の検査結果から自動取得する。配布先とタグ、Pre-release の有無が一致しない場合は公開を止める。
4. 下書きの2ファイルを取得・再検証し、検査結果に記録された SHA-256（明示した場合は入力値）と一致した場合に、警告を承認して公開する。公開者と理由を Release の説明に残し、空欄の場合は「理由の記入なし」と記録する。

Pages に初めて配置するときだけ、`initialize_pages` を選ぶ。以後は選ばず、公開済みの一覧を引き継いで他の配布先を維持する。

検査結果の SHA-256 が欠けている場合は自動取得できないため、確認した値を明示する。自動公開では引き続き生成ジョブが渡す SHA-256 を必須とする。

この処理では再ダウンロード・再生成・アセットの差し替えを行わない。前回公開版との比較もやり直すため、通信失敗や前回ファイルの破損で比較できない場合は手動公開も停止する。自動公開と手動公開のジョブは共通の実行グループで直列に動かす。

検証を通すため、下書きは GitHub の公開ボタンから直接公開せず、このワークフローを使う。アプリ側が新形式に対応しているかの確認も、形式変更時の手動公開で行う。

### 手元での生成

以下のコマンドは `points/osm-peaks/` を作業ディレクトリにして実行する。Python 3.12 と osmium-tool が必要（Ubuntu では `sudo apt-get install osmium-tool`）。

```bash
# リポジトリのルートから移動する。
cd points/osm-peaks
python -u scripts/download_source.py
python scripts/build_data.py build/japan-latest.osm.pbf \
  --version local-20260930 --output-dir dist
```

開発版を手元で生成するときは、生成コマンドに `--channel dev` を指定する。省略時は `RELEASE_CHANNEL` の値、未設定なら正式版を使う。取得 URL のリポジトリは `GH_REPO`、未設定なら `GITHUB_REPOSITORY`、どちらも未設定なら `shohei0205/yamamuki-data` を使う。

取得時に PBF の隣へ `<PBF のファイル名>.source.json` を保存し、URL・サイズ・MD5 を記録する。取得済みファイルを再利用した場合も記録を作る。生成時に記録と PBF を照合し、日付付き URL を manifest の `sourceUrl` に引き継ぐ。記録の欠落や不一致は生成を止める。既存の PBF に記録がない場合は、同じ対象日で取得コマンドを再実行すると、内容が一致すれば再ダウンロードせずに記録を作れる。

元データの取得には数 GB の通信量と空き容量が必要。元データは `points/osm-peaks/build/`、配布ファイルは `points/osm-peaks/dist/` に保存し、どちらも git に入れない。

### 山頂に SVG を設定する

`points/osm-peaks/graphics/` に `<assetId>.svg` と、地点 ID から assetId への対応表 `points.json` を置く。対応表の例は `{"3403990450":"fuji"}`。生成する地点の中から対応する ID に `graphic` を追加する。SVG を用意する場合は、この資料に画像の出典・作成者・利用条件も追記する。

月次・手動の生成 Action は `graphics/points.json` がある場合に画像を取り込み、対応する地点の `graphic.svg` に SVG 本文を内蔵する。手元で生成する場合は次のように指定する（作業場所は `points/osm-peaks/`）。

```bash
python scripts/build_data.py build/japan-latest.osm.pbf --version local \
  --graphics-directory graphics --graphics-map graphics/points.json
```

入力の SVG が欠けている場合は生成を止める。SVG 本文はデータ本体と一緒に圧縮し、公開前は取得した JSON 内の SVG を再検査する。配布するのは gzip と manifest の2ファイル。入力の SVG は git に保存し、生成した `dist/` はコミットしない。

## テスト

Python 3.12 と osmium-tool を使い、単体テストと小さな PBF による生成テストを行う。SVG の対応表・JSON への内蔵・倍率・未対応要素の拒否・公開前の再検査も単体テストで確認する。CIの「共通：データ処理のテスト」（`.github/workflows/test.yml`）の「地点 / OSM山頂」ジョブで push・PR 時に実行する。

テストは `points/osm-peaks/` 内で、外部通信を行わず次のコマンドで実行できる。osmium がない場合は PBF を使うテストだけをスキップする。Actions では osmium を入れてすべて実行する。

```bash
python -m unittest discover -s tests -v
```

## ライセンス

出典・利用条件は [README のライセンス](../../README.md#ライセンス)を参照。

### Releaseを削除した後に公開を再開する

公開サイトのmanifestが指すReleaseを削除した場合、手動公開で「Pagesの配布サイトを初期化」を指定する。前回のReleaseが存在しない場合に限り、前回との比較を省いて確認済みデータの公開を再開する。新しいデータ本体のサイズ・ハッシュ・件数などの検査は行う。通信失敗やデータ破損は初期化を指定しても停止する。他のデータと配布先は引き継ぐ。

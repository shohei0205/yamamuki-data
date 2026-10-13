# OpenStreetMap の山頂データ

[地点データの共通仕様に戻る](../README.md)

全国の山頂データの形式、取得先、生成・検査・公開の手順をまとめる。

| 配布先 | 最新版の manifest | 公開履歴 |
|---|---|---|
| 正式版 | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks/manifest.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks/manifest.json) | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks/history.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks/history.json) |
| 開発版 | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/manifest.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/manifest.json) | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/history.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/history.json) |

データ本体（`osm-peaks.json.gz`）は、各 manifest の `downloadUrl` から取得する。

正式版は生成しても Pages を更新しない。上の正式版の manifest と公開履歴は、その運用に変えた時点のまま残る。アプリ（yamamuki）がどの正式版を読むかは、yamamuki の PR で Release の manifest を選んで決める。

## 配るもの

Geofabrik の日本全国の OSM データから、`natural=peak` または `natural=volcano` の名前付きノードを抽出する。日本全体を `osm-peaks.json.gz` 1 ファイルにまとめる。way・relation と名前のないノードは含めない。収録する地点の `type` は山頂を表す `peak` とし、火山ノードも同じ種別で出力する。名前は前後の空白を除き、`name:ja`、`name` の順に使う（`name:ja` だけのノードも含む）。

地点ごとの JSON の形式は [地点データの共通仕様](../README.md#地点の形式)を参照する。OSM ノード ID の昇順に並べる。ふりがな・別名・解説リンク・標高は、以下の OSM タグから変換する。

- `nameReading`: `name:ja-Hira` のふりがな。前後の空白を除く。未登録・空欄は `null`。推測による補完はしない。補足 JSON に指定があれば、そのよみがなを適用する。
- `aliases`: `alt_name:ja`、`alt_name` の順に集めた別名の配列。セミコロンで分割し、前後の空白・空欄・表示名と同じ名前・重複を除く。未登録は `[]`。
- `wikipediaUrl`: `wikipedia` の「言語:記事名」を HTTPS URL に変換した解説へのリンク。日本語・空白・記号を URL 用に変換し、記事内の節にも対応する。未登録・形式不正は `null`。
- `wikidataUrl`: `wikidata` の項目 ID（例: `Q39231`）から作った HTTPS リンク。未登録・形式不正は `null`。

- `elevationM`: `ele` から取得する。値がない、または解釈できないときは `null`。カンマ、m・ft などの単位、セミコロン区切りの先頭値に対応する。

生成ごとの件数・圧縮前後のサイズ・元データの日時・SHA-256・検査結果は、Actions の実行概要と Release の説明で確認する。各版の manifest にも件数・サイズ・日時・ハッシュを記録する。

圧縮後のサイズが 5,000,000 バイトを超える場合は公開を止め、分割を検討する。データが空、ID が重複、座標が不正、元データの日付が取得できない場合も公開しない。

## 置き場所

[Releases](https://github.com/shohei0205/yamamuki-data/releases) に `manifest.json` と `osm-peaks.json.gz` を添付する。最新版を示す manifest と公開履歴は、冒頭の表にある Pages の URL から取得できる。

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

Release のタイトルとタグは同じ値とし、正式版は `osm-peaks-<version>`、開発版は `osm-peaks-dev-<version>` とする。取得・検証の契約は [共通の読み込みと公開](../../README.md#読み込みと公開) に従う。

山頂データは正式版と開発版の参照先を分ける。公開済みの地点データ一覧は [地点カタログ](../README.md#公開データのカタログ) を参照する。リポジトリ全体の `releases/latest` は使わない。

| 配布先 | Pages のパス | データ本体を置くタグ |
|---|---|---|
| 正式版（`stable`） | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks/manifest.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks/manifest.json) | `osm-peaks-<version>` |
| 開発版（`dev`） | [https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/manifest.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/manifest.json) | `osm-peaks-dev-<version>` |

正式版と開発版で manifest の形式は共通とし、開発版の履歴 Release は Pre-release とする。アプリで配布先を選ぶときの扱いは [地点カタログの説明](../README.md#公開データのカタログ) を参照する。

件数・更新日時の比較、初回の手動確認、異常時の下書き保留、手動公開、参照先の復旧は配布先ごとに独立して行う。開発版を正式版の比較基準にせず、開発版の公開で正式版の参照先を更新しない。タグの衝突を防ぐため、`latest` と `dev-` で始まる版名は予約する。

Pages の初期設定、サイト全体の一覧・履歴の引き継ぎ、配置確認、参照先の復旧は [共通の Pages 配置](../../README.md#pages-への配置と参照先の復旧) を参照する。

山頂の生成時は地点の `id` と `osmId` の両方を出力し、OSM ノード ID の数値順に並べる。取得できない任意情報は`null`・`[]` として出力する。検査では任意項目の省略を許可し、`osmId` がある場合は `id` との一致を確認する。

## 公開履歴

山頂の公開履歴は、正式版の [history.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks/history.json) と開発版の [history.json](https://shohei0205.github.io/yamamuki-data/points/osm-peaks-dev/history.json) に保存する。項目と記録のタイミングは [共通の公開履歴](../../README.md#公開履歴) を参照する。

## 作り方

利用開始時に、リポジトリの Settings → Pages → Build and deployment の Source を **GitHub Actions** にし、`github-pages` 環境の配置元として `main` と `dev` を許可する。

[地点 / OSM山頂：生成・検査](https://github.com/shohei0205/yamamuki-data/actions/workflows/publish-data.yml) は、毎月 1 日の UTC 03:23（日本時間 12:23）に `main` で正式版を生成する。手動でも実行できる。

正式版は Release を作るところまでで、Pages には公開しない。配信事故（[#44](https://github.com/shohei0205/yamamuki-data/issues/44)）のあと、アプリが読むデータは yamamuki の PR で選ぶ形に変えたため。開発版（`dev`）は今までどおり、検査に通ったら Pages の `points/osm-peaks-dev/` に公開する。

手動で動かすときは Actions の「地点 / OSM山頂：生成・検査」→「Run workflow」で正式版なら `main`、開発版なら `dev` を選ぶ。この2つ以外のブランチでは公開しない。公開ジョブの `GITHUB_TOKEN` に `contents: write`・`pages: write`・`id-token: write` を付与し、追加のトークンは使わない。

1. 単体テストと、小さな PBF による生成テストを行う。
2. Geofabrik の `japan-latest.osm.pbf` から日付付き URL を確定する。取得対象日を指定した場合は、その日付の URL を直接使い、全国データを取得する。途中で切れたら同じ版の続きから再開し、配布元の MD5 と照合する。Overpass API は使わない。
3. `osmium tags-filter` で対象ノードだけを抽出し、配布ファイルと manifest を作る。
4. 前回の版を取得・検証し、全国と地域別の件数、元データの日時、形式の版を比較する。前回の版は、正式版では「要確認」でない最新の正式版 Release（下書きと Pre-release を除き、タグの版名が最も新しいもの）、開発版では Pages の `points/osm-peaks-dev/manifest.json` が指す Release とする。件数・前回との差・検査結果・圧縮サイズ・元データの日時・SHA-256 を、Release の説明と Actions の実行概要に記録する。
5. 正式版は、両ファイルを添付した Release を下書きにせずそのまま作る。「最新」の印（Latest）は付けない。要確認の版も Release にし、本文の先頭の「要確認」の見出しの下に理由を書く。Pages は更新しない。
6. 開発版は、両ファイルを下書きの Pre-release に添付する。検査に合格した場合だけ、別の公開ジョブが下書きのファイルをダウンロードし、再検証して公開する。履歴版を公開後、開発版の最新版 manifest を Pages で更新する。

Actions の各ステップでは、時刻付きで処理の開始・完了をログに出す。Python の出力はためずに随時表示する。

- ダウンロード中：受信が進んでいる間は約15秒ごとに取得量・割合・平均速度・経過秒数を表示する。接続待ち、再開位置、再試行、取得済みファイルの再利用、MD5 の照合も記録する。通信が止まっている間は次の受信またはタイムアウトまで進捗行は増えない。
- 通信の診断：HTTP メソッド、要求 URL、各転送先と HTTP ステータス、最終応答 URL、Range、経過時間を記録する。応答ヘッダーは Location・Content-Length・Content-Range・Content-Type・Retry-After・Date・Age・Cache-Control・Cache-Status・Via・Server に限定し、本文は記録しない。
- 取得失敗時：処理段階、試行回数、対象 URL、保存先、例外の種類・内容、残り時間と次の待ち時間を表示する。最後の試行でも失敗内容を残し、試行回数の上限・時間制限・再試行対象外のどれで終了したかを記録する。
- 生成中：元データの日時、osmium の抽出進捗、読み取り5,000件ごとの件数、圧縮前後のサイズ、SHA-256 と manifest の作成、保存完了を表示する。
- 公開時：Release（開発版は下書き）の作成・アップロード開始、アップロード完了、公開完了を表示する。

失敗した実行では、それ以前の公開済み Release を書き換えない。開発版でアップロード途中に失敗した下書きは公開されず、残った下書きは手動で削除できる。正式版でアップロード途中に失敗した Release は、yamamuki から選ばれないので残してよい。再実行は新しいタグを作る。配布ファイルの再現性のため、gzip ヘッダーに生成日時やファイル名を含めない。

ダウンロードは通信待ち60秒、最大6回（15秒間隔）の試行、全体60分の制限を設ける。Actions は取得ステップ65分、生成ジョブ90分、公開ジョブ15分で打ち切る。途中ファイルは元データの MD5 ごとに保存し、別の版のデータをつなげない。Range に対応しない応答では先頭から取り直す。通信待ちや再試行で失敗した場合は、同じコマンドを再実行すれば途中ファイルを再利用できる（配布元が同じ版の場合）。Actions の別実行には途中ファイルを引き継がない。

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

月次実行は `main` から正式版（`stable`）の Release を作る。手動実行は Actions の「地点 / OSM山頂：生成・検査」→「Run workflow」でブランチを選ぶ。

| 実行元ブランチ | 配布先 | 生成・公開に使う処理 |
|---|---|---|
| `main` | 正式版（`stable`）。Release まで | `main` のコード |
| `dev` | 開発版（`dev`、Pre-release）。Pages にも公開 | `dev` のコード |

配布先の選択欄は設けず、ブランチから自動判定する。手動公開・参照先復旧も同じ規則で動く。その他のブランチやタグからは公開しない。ブランチへの push だけではデータを生成しない。`dev` の月次実行はなく、開発版が必要なときに手動実行する。

利用開始には、このワークフローを `main` と `dev` の両方に配置する。開発中の生成処理は `dev` で確認し、正式版に採用するときは変更を `main` へ取り込む。

生成は配布先ごとのグループで直列化する。公開・手動復旧・Pages の配置は正式版と開発版で共通のグループを使い、他の参照先の引き継ぎから配置まで順番に行う。開発版から正式版への自動昇格は行わず、正式版が必要なときは `main` を選んで生成する。開発版の初回は下書きに残るので、確認して手動公開する。

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

OpenStreetMap 由来のデータは **© OpenStreetMap contributors** を表示し、[Open Database License（ODbL）1.0](https://opendatacommons.org/licenses/odbl/1-0/) に従って利用・再配布する。[OpenStreetMap の著作権とライセンス](https://www.openstreetmap.org/copyright)を参照。

## 地点の補足対応表

[supplements.json](supplements.json) 一つで全タグと補正を管理する。生成時は osmId で対応付け、タグの追記・表示名とよみがなの補正・別名の追加と削除・地点の除外を適用する。`--supplements` で別のファイルを指定できる。不正値・未知の ID・重複 ID・想定値の不一致は生成エラーにする。修正理由と管理用メモは補足ファイルに保持し、統合した地点データには注入しない。

日本百名山の 100 地点も同じ補足 JSON に収録する。共通形式は [補足 JSON の形式](../README.md#補足-json-の形式)、編集操作は [ビューアの編集手順](../README.md#補足を編集する) を参照する。

### 山名と osmId の補完

```bash
PYTHONPATH=points/osm-peaks python -m scripts.supplements points/osm-peaks/supplements.json --points osm-peaks-source.json.gz --output points/osm-peaks/build/supplements.json
```

osmId がない行では `note` の山名・別名を完全一致で照合する。osmId があり `note` が空なら地点名をメモに補う。候補が複数ある場合は未解決とし、終了コード 1 を返す。形式の不備は終了コード 2。入力は変更せず、補足・補正・出典情報を出力へ引き継ぐ。補完には補足適用前の元データを使う。

### 出典の管理

出典・ライセンス・著作者表示・加工内容は、補足 JSON の `sources` に記録する。生成時は manifest.json の `supplementSources` に引き継がれる。詳しい属性は [補足 JSON の形式](../README.md#補足-json-の形式) を参照する。

### タグごとの記録

タグごとの記録は `notes/<タグ名>.md` に置く。新しいタグを追加するときも同じ命名にする。

日本百名山の採用方針・地点の見直し・確認待ち事項・外部資料との照合結果は、[日本百名山タグの採用方針と検証記録](notes/日本百名山.md) にまとめる。現在の採用 osmId は [supplements.json](supplements.json) で管理する。

### アプリ用データと確認・編集用の添付

アプリは、現状の manifest の参照先から補足マージ済みの `osm-peaks.json.gz` 一つを取得する。ファイル名・取得 URL・manifest の版5を維持し、Android・iOS 側で補足をマージする処理は追加しない。マージ前のオリジナル JSON.gz と補足は、確認・編集用として同じ Release に添付する。

| ファイル | 内容 |
|---|---|
| `osm-peaks.json.gz` | 補足マージ済みの地点データ。アプリが取得する唯一の地点 JSON |
| `manifest.json` | アプリ向けの取得先・サイズ・ハッシュ。`supplementSources` に補足の全出典を記録 |
| `osm-peaks-source.json.gz` | 補足マージ前のオリジナル地点データ。OSM からの変換処理は適用済み。確認・編集用 |
| `supplements.json` | 全タグ・補正・除外と、ファイル全体の出典一覧 |
| `supplement-manifest.json` | 上記の元データと補足の取得先・サイズ・SHA-256、Release の版、出典・ライセンス |

補足用 manifest の形式は [版1](../../schemas/manifest/supplement-v1.schema.json)。`base` と `supplements` にそれぞれ `fileName`・`downloadUrl`・`sizeBytes`・`sha256` を記載し、`distributionSha256` で既存の配布 gzip と結び付ける。`dataSchemaVersion` は元データの版、`sources` は補足の出典一覧。トップレベルの OSM の `license`・`attribution` も保持する。補足 manifest は Release に添付し、Pages の既存 manifest の参照先は変更しない。

公開前にすべての添付を再取得し、サイズ・ハッシュ・URL・版・出典が一致することと、元データへ補足を適用して配布データを再現できることを検査する。補足のない過去の Release も検証できる。補足関連の添付が一部しかない Release は公開しない。

由来を確認・編集するときは、ビューアで `osm-peaks-source.json.gz` を開き、続けて `supplements.json` を開く。統合済みの `osm-peaks.json.gz` だけでは補正前の値や項目ごとの由来は復元できない。確認・編集用の添付と補足 manifest はアプリの読み込み対象にしない。

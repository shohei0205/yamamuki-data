# 山頂データ

[README に戻る](../README.md)

全国の山頂データの形式、取得先、生成・検査・公開の手順をまとめる。

## 配るもの

Geofabrik の日本全国の OSM データから、`natural=peak` または `natural=volcano` の名前付きノードを抽出する。日本全体を `japan-mountains.json.gz` 1 ファイルにまとめる。way・relation と名前のないノードは含めない。名前は前後の空白を除き、`name:ja`、`name` の順に使う（`name:ja` だけのノードも含む）。

gzip を展開すると、UTF-8 の JSON 配列になる。OSM ノード ID の昇順で、各項目は次の形式。

```json
{"osmId":3403990450,"name":"万三郎岳","latitude":34.8627963,"longitude":139.0018525,"elevationM":1405.6,"nameReading":"ばんざぶろうだけ","aliases":["天城山"],"wikipediaUrl":"https://ja.wikipedia.org/wiki/%E5%A4%A9%E5%9F%8E%E5%B1%B1","wikidataUrl":null}
```

- `osmId`: OSM ノード ID（整数）。
- `name`: 表示名。
- `nameReading`: `name:ja-Hira` のふりがな。前後の空白を除く。未登録・空欄は `null`。推測による補完はしない。
- `aliases`: `alt_name:ja`、`alt_name` の順に集めた別名の配列。セミコロンで分割し、前後の空白・空欄・表示名と同じ名前・重複を除く。未登録は `[]`。
- `wikipediaUrl`: `wikipedia` の「言語:記事名」を HTTPS URL に変換した解説へのリンク。日本語・空白・記号を URL 用に変換し、記事内の節にも対応する。未登録・形式不正は `null`。
- `wikidataUrl`: `wikidata` の項目 ID（例: `Q39231`）から作った HTTPS リンク。未登録・形式不正は `null`。

- `latitude` / `longitude`: WGS 84 の緯度・経度（度）。
- `elevationM`: 標高（m）。値がない、または解釈できないときは `null`。カンマ、m・ft などの単位、セミコロン区切りの先頭値に対応する。

2026年9月29日20:22:51 UTC 時点の全国 PBF（2,541,313,014 バイト）で、14,023 件を生成できた。追加情報を含む gzip は 450,887 バイト、展開後は 2,749,206 バイト。配布元の MD5、生成物の SHA-256・件数・サイズをローカルで照合済み。以後の生成でも Actions の実行概要で実測値を確認する。5,000,000 バイトを超える場合は公開を止め、分割を検討する。データが空、ID が重複、座標が不正、元データの日付が取得できない場合も公開しない。

## 置き場所

[Releases](https://github.com/shohei0205/yamamuki-data/releases) に、次の 2 ファイルを公開する。

- [manifest.json](https://github.com/shohei0205/yamamuki-data/releases/download/peaks-latest/manifest.json)
- データ本体は manifest の `version` と `fileName` から取得先を決める（下記参照）。

`manifest.json` の形式（schemaVersion 3）:

| 項目 | 内容 |
|---|---|
| `schemaVersion` | 形式の版。現在は整数の `3`。JSON 配列の形式も対象とする |
| `version` | 生成時の UTC 日時・Actions の実行 ID・再実行番号をつないだ文字列 |
| `fileName` | `japan-mountains.json.gz` |
| `sha256` | gzip ファイルそのものの SHA-256（小文字の16進数） |
| `sizeBytes` | gzip ファイルのバイト数 |
| `uncompressedSizeBytes` | 展開後の JSON のバイト数 |
| `mountainCount` | 山の件数 |
| `sourceTimestamp` | PBF ヘッダーの `osmosis_replication_timestamp`。UTC の日時（例: `2026-09-30T20:21:22Z`） |
| `latestMountainTimestamp` | アセットに収録する山頂ノードの `timestamp`（OSM 上の最終編集日時）の最大値。UTC の日時。現在の全国データでは `2026-09-29T08:06:38Z` |
| `sourceUrl` | `https://download.geofabrik.de/asia/japan-latest.osm.pbf` |
| `license` | `ODbL-1.0` |
| `attribution` | `© OpenStreetMap contributors` |

schemaVersion 2 では、山データに `nameReading`・`aliases`・`wikipediaUrl`・`wikidataUrl` を追加した。既存の名前・位置・標高は引き続き同じ形式。利用するアプリは対応する形式の版を確認してから読み込む。リンク先の記事本文は同梱せず、閲覧には通信が必要。

schemaVersion 3 では、manifest に `latestMountainTimestamp` を追加した。山ごとの JSON の項目は版2から変更していない。名前のない山頂など収録対象外のノードは集計しない。収録対象のノードに日時がない・形式が不正・元データの基準日時より新しい場合は生成を止める。公開前の検査でも日時の形式と基準日時との前後関係を確認する。

`latestMountainTimestamp` は山頂ノードの最終編集日時であり、現地調査日・標高の測定日・アセット生成日時ではない。`sourceTimestamp` は全国 PBF 全体の基準日時を表す。

元データの日付はダウンロード日時とは異なる。同じ元データで再実行すると、配布の `version` は変わる。

Release のタグは `peaks-<version>`。manifest を取得後、`https://github.com/shohei0205/yamamuki-data/releases/download/peaks-<version>/<fileName>` から対応するファイルを取得する。`peaks-latest` は manifest だけを持ち、データ本体は各版に保存する。アプリでは展開前にサイズと SHA-256 を検証する。既存アプリへの読み込み機能の組み込みは、アプリ側の別作業となる。

山頂データは正式版と開発版の参照先を分ける。リポジトリ全体の `releases/latest` は使わない。

| 配布先 | 最新 manifest を置くタグ | データ本体を置くタグ |
|---|---|---|
| 正式版（`stable`） | `peaks-latest` | `peaks-<version>` |
| 開発版（`dev`） | `peaks-dev-latest` | `peaks-dev-<version>` |

開発版の manifest は `https://github.com/shohei0205/yamamuki-data/releases/download/peaks-dev-latest/manifest.json`、本体は `releases/download/peaks-dev-<version>/<fileName>` から取得する。manifest の形式は共通で、配布先はアプリ側で選び、その配布先のタグを組み立てる。開発版が無い・取得できない場合に正式版へ自動で切り替えない。開発版には参照用 Release も含めて GitHub の Pre-release を付ける。

件数・更新日時の比較、初回の手動確認、異常時の下書き保留、手動公開による確認、参照先の復旧は配布先ごとに独立して行う。開発版を正式版の比較基準にせず、開発版の公開で正式版の参照先を更新しない。`latest` と `dev-` で始まる版名はタグの衝突を防ぐため予約する。

履歴版を公開してから、選択した配布先の最新版 manifest を差し替える。差し替え中は一時的に取得できない場合があるため、アプリは取得・検証に失敗したら保存済みのデータを維持して再試行する。参照先の更新に失敗した場合は、公開済みの同じタグ・SHA-256・理由を手動公開ワークフローに指定して復旧する（古い版を指定すると意図的な差し戻しになる）。履歴版のデータ本体は再生成・再アップロードしない。`peaks-latest` と `peaks-dev-latest` はアセットを差し替えるため、リリースの不変化を適用しない運用が必要。

## 作り方

[全国の山頂データを生成・検査](https://github.com/shohei0205/yamamuki-data/actions/workflows/publish-data.yml) は、毎月 1 日の UTC 03:23（日本時間 12:23）に `main` で動く。GitHub の混雑で開始が遅れる場合がある。

手動で動かすときは Actions の「全国の山頂データを生成・検査」→「Run workflow」で正式版なら `main`、開発版なら `dev` を選ぶ。この2つ以外のブランチでは公開しない。リポジトリの `GITHUB_TOKEN` に `contents: write` を付与し、追加のトークンは使わない。

1. 単体テストと、小さな PBF による生成テストを行う。
2. Geofabrik の `japan-latest.osm.pbf` から日付付き URL を確定し、全国データを取得する。途中で切れたら同じ版の続きから再開し、配布元の MD5 と照合する。Overpass API は使わない。
3. `osmium tags-filter` で対象ノードだけを抽出し、配布ファイルと manifest を作る。
4. 選択した配布先の前回 manifest（正式版は `peaks-latest`、開発版は `peaks-dev-latest`）を取得・検証し、全国と地域別の件数、元データの日時、形式の版を比較する。件数・前回との差・検査結果・圧縮サイズ・元データの日時・SHA-256 を、下書きの説明と Actions の実行概要に記録する。
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
| 山頂の最新編集日時が同じ | `latestMountainTimestamp` が前回公開版と同じ日時。PBF の基準日時や生成した版が新しくても下書きに残す |
| 日時の逆戻り | 元データの日時が前回公開版より古い |
| 形式の変更 | `schemaVersion` が前回公開版と異なる |
| 比較不能 | 前回公開版の取得・検証ができない。通信失敗や権限不足を初回扱いしない |

地域の判定は都道府県ではなく、緯度・経度をそれぞれ切り下げた1度区画で行う。例えば北緯35度・東経139度の区画は、北緯35度以上36度未満・東経139度以上140度未満。細かな境界付近の変化で保留が多ければ、実績を見て基準を調整する。比較対象は常に同じ配布先の最新版参照が指す公開済みの版とし、確認待ちの下書きは基準にしない。

件数の減少などは警告として手動で承認できる。ただしファイル破損、SHA-256・サイズ・件数の不一致、空データ、不正な ID・座標・データ形式は公開不可。手動でも同じ検証を行う。生成直後に不正が見つかった場合は下書きも作らない。

### 正式版と開発版の実行方法

月次実行は `main` から正式版（`stable`）を生成する。手動実行は Actions の「全国の山頂データを生成・検査」→「Run workflow」でブランチを選ぶ。

| 実行元ブランチ | 配布先 | 生成・公開に使う処理 |
|---|---|---|
| `main` | 正式版（`stable`） | `main` のコード |
| `dev` | 開発版（`dev`、Pre-release） | `dev` のコード |

配布先の選択欄は設けず、ブランチから自動判定する。手動公開・参照先復旧も同じ規則で動く。その他のブランチやタグからは公開しない。ブランチへの push だけではデータを生成しない。`dev` の月次実行はなく、開発版が必要なときに手動実行する。

利用開始には、このワークフローを `main` と `dev` の両方に配置する。開発中の生成処理は `dev` で確認し、正式版に採用するときは変更を `main` へ取り込む。

同じ配布先の生成・公開・手動復旧は共通の実行グループで直列化する。正式版と開発版は別のグループなので、互いの公開待ちにはならない。開発版から正式版への自動昇格は行わず、正式版が必要なときは `main` を選んで生成する。両方の初回は下書きに残るので、確認して手動公開する。

### 確認済みの下書きを手動公開する

1. Releases の下書きにある検査結果と必要なデータの差分を確認する。
2. Actions の「確認済みの山頂データを公開」→「Run workflow」で正式版なら `main`、開発版なら `dev` を選ぶ。
3. 対象タグ（正式版は `peaks-<version>`、開発版は `peaks-dev-<version>`）、確認した gzip の SHA-256、確認内容・公開理由を入力する。配布先とタグ、Pre-release の有無が一致しない場合は公開を止める。
4. 下書きの2ファイルを取得・再検証し、入力した SHA-256 と一致した場合に、警告を承認して公開する。公開者と理由を Release の説明に残す。

この処理では再ダウンロード・再生成・アセットの差し替えを行わない。前回公開版との比較もやり直すため、通信失敗や前回ファイルの破損で比較できない場合は手動公開も停止する。定期実行と手動公開は共通の実行グループで直列に動かす。

検証を通すため、下書きは GitHub の公開ボタンから直接公開せず、このワークフローを使う。アプリ側が新形式に対応しているかの確認も、形式変更時の手動公開で行う。

### 手元での生成

以下のコマンドは `peaks/` を作業ディレクトリにして実行する。Python 3.12 と osmium-tool が必要（Ubuntu では `sudo apt-get install osmium-tool`）。

```bash
# リポジトリのルートから移動する。
cd peaks
python -u scripts/download_source.py
python scripts/build_data.py build/japan-latest.osm.pbf \
  --version local-20260930 --output-dir dist
```

元データの取得には数 GB の通信量と空き容量が必要。元データは `peaks/build/`、配布ファイルは `peaks/dist/` に保存し、どちらも git に入れない。

テストは `peaks/` 内で、外部通信を行わず次のコマンドで実行できる。osmium がない場合は PBF を使うテストだけをスキップする。Actions では osmium を入れてすべて実行する。

```bash
python -m unittest discover -s tests -v
```

## ライセンス

出典・利用条件は [README のライセンス](../README.md#ライセンス)を参照。

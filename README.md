# yamamuki-data

山むきアプリ（[shohei0205/yamamuki](https://github.com/shohei0205/yamamuki)）が使うデータを作って配るためのリポジトリ。

## データの種類

| 種類 | 詳細 |
|---|---|
| 地点データ | [データ一覧・共通形式・公開手順](points/README.md) |
| 地形データ | 将来追加予定・未実装 |

収録対象、データ本体の形式、ダウンロード先、生成・公開手順、検査基準は、種類ごとの文書にまとめる。

## データを利用する

1. [地点データの一覧](points/README.md)から、利用するデータを選ぶ。
2. [地点データの取得・利用手順](points/README.md#データを取得して利用する)に従い、manifest とデータ本体を取得して検証する。
3. JSON の項目は [地点 JSON の形式](points/README.md#地点-json-の形式)、出典・利用条件は [ライセンス](#ライセンス)を確認する。

## フォルダ構成

```text
points/            地点データ（詳細は points/README.md）
formats/           データ種別に共通する manifest の版別形式説明
schemas/           データ種別に共通する manifest の版別スキーマ
release_tools/     データ種別共通の公開入口
.github/workflows/ GitHub Actions の定義
```

データの種類ごとにフォルダを分け、内部の構成は各 README に記載する。Actions の定義は `.github/workflows/` に置く。

## データの配布

### 配布の方針

生成したデータは [GitHub Releases](https://github.com/shohei0205/yamamuki-data/releases) に置く。元データとデータ本体は原則として git に含めない。国土地理院版は生成済みの地点 JSON と出典要約 JSON だけをコミットし、Action で Release を作る。開発版の最新版を示す小さな `manifest.json` は、Actions の生成物として GitHub Pages に公開する。

正式版は Release を作るところまでで、Pages には公開しない。アプリ（yamamuki）がどの版を読むかは、yamamuki の PR で Release の manifest を選んで決める。アプリが読んでいる版のデータが消えないよう、公開した Release とその添付ファイルは削除・差し替えしない。

データの種類ごとに公開時期と最新版の参照先を分ける。正式版と開発版も独立して扱い、リポジトリ全体の `releases/latest` はアプリの参照先に使わない。具体的な URL とタグは各データの文書を参照する。

| ブランチ | 配布先 |
|---|---|
| `main` | 正式版 |
| `dev` | 開発版（Pre-release） |

開発中の処理は `dev` で確認し、正式版に採用するときは変更を `main` に取り込む。実行タイミングと確認方法はデータごとに定める。

正式版も開発版も、このリポジトリの同じ [Releases 一覧](https://github.com/shohei0205/yamamuki-data/releases) に公開する。`main`・`dev` は生成処理の実行元で、配布ファイルは各 Release の **Assets** に添付する。データの種類と正式版・開発版の違いは、Release のタグで区別する。

最新版を示す manifest は GitHub Pages に置く。`main`・`dev` のコードでサイトのファイルを生成して直接配置するため、配布専用ブランチは作らない。生成物も git に入れない。

データごとの manifest と公開履歴の URL、データ本体の取得方法は、各データの文書にまとめる。

### 配布データの取得情報（manifest.json）

データ本体の取得先・大きさ・ハッシュ・出典を記載する共通の JSON。項目・制約・読み込み方法は [manifest フォーマット](formats/manifest.md) を参照する。機械検証用のスキーマは [manifest の版5](schemas/manifest-v5.schema.json)。

## Actions の構成

共通の確認・公開操作は「共通：」、データ固有の操作は「種類 / データ名：」でそろえる。種類全体に対する操作は「種類：」とする。テストの入口は1つにまとめ、内部のジョブを共通処理・データごとに分ける。生成は取得元や実行周期に応じてデータ別のActionにする。

| Action | 対象 |
|---|---|
| 共通：データ処理のテスト | 共通処理と各データのテスト |
| 共通：文字形式の確認 | 改行コードとBOM |
| 共通：確認済みデータを公開 | データ種別共通の手動公開 |

データ固有のActionと操作手順は各READMEを参照する。

## 確認済みデータの公開

Actions の「共通：確認済みデータを公開」を入口とする。正式版は `main`、開発版は `dev` を選ぶ。指定した Release のタグからデータ種別を判定し、その種別の検証・公開処理を実行する。対応する種別と固有の検査条件は各 README を参照する。

1. 下書き Release の検査結果と、必要なデータの差分を確認する。
2. Actions の「共通：確認済みデータを公開」→「Run workflow」で実行元ブランチを選ぶ。
3. `tag` 欄に確認した Release ページの URL またはタグを指定する。`untagged-...` を含む下書きの URL も使える。確認内容・公開理由は任意で、`sha256` は空欄なら検査結果から取得する。
4. 公開処理が添付ファイルを取得・再検証し、配布先・タグ・Pre-release の区分と SHA-256 を確認する。公開者と理由を Release の説明に残し、空欄なら「理由の記入なし」と記録する。

検証を通すため、下書きは GitHub の公開ボタンから直接公開せず、このワークフローを使う。Pages に初めて配置するときだけ `initialize_pages` を選ぶ。既存の参照先を復旧する場合の条件は、対象データの手順を確認する。

共通の振り分け処理は `release_tools/`、データ固有の検証・公開処理は各データのフォルダに置く。データ種別を追加するときは `release_tools/publish_reviewed.py` の登録表に追加する。未対応の種別や最新版参照タグを指定すると停止する。

### Pages への配置と参照先の復旧

利用開始時に、リポジトリの Settings → Pages → Build and deployment の Source を **GitHub Actions** にし、`github-pages` 環境の配置元として `main` と `dev` を許可する。

履歴版の公開後、Actions で manifest のサイトを生成し、`upload-pages-artifact` と `deploy-pages` で GitHub Pages に直接配置する。ソースは `main`・`dev` に置き、配布専用ブランチや生成物のコミットは作らない。

サイト全体の更新には `/catalog.json` を使う。`schemaVersion: 1` とし、`manifests` にパスと manifest の対応、公開履歴も保存する。既存一覧からサイト全体を作り直し、対象のデータ・配布先だけを置き換える。他のデータ・配布先・履歴は引き継ぐ。これは [アプリが使う地点カタログ](points/README.md#公開データのカタログ) とは別の一覧である。

Pages を更新するジョブは共通の `publish-data-pages` グループで直列化する。配置後は manifest 一覧・公開履歴・地点カタログが生成内容と一致するまで最大 55 秒待ち、反映を確認してからジョブを完了する。既存一覧の取得や検証に失敗した場合は配置しない。404 の場合も通常の公開では停止し、初期化を明示した初回の手動公開だけが一覧を新しく作る。

配置時にファイルを削除してから上げ直す時間は生じない。キャッシュにより更新前の manifest が返る場合はあるが、履歴版を残すため、その manifest でも対応するデータ本体を取得できる。アプリは通信や検証に失敗したら保存済みデータを維持して再試行する。

参照先の更新に失敗した場合は、公開済みの同じ Release の URL またはタグを手動公開ワークフローに指定して復旧する。履歴版のデータ本体は再生成・再アップロードしない。公開済みの履歴 Release には不変化を適用できる。

### 実行概要の確認

生成・検査と確認済みデータの公開は、処理の冒頭に Summary へ実行パラメーターを表示する。ブランチ・イベント・実行者・実行回数・配布先に加え、手動公開では入力 URL またはタグ・SHA-256・確認内容と公開理由・Pages 初期化の指定を表示する。空欄の場合の扱いも明記する。データ固有の入力項目と検査結果は各 README を参照する。

公開ジョブの Summary には公開後の Release の URL、最新版 manifest の固定 URL と Pages の配置結果を表示する。下書きで保留された場合は参照先を更新しない。単体テストのダミーデータは Summary に出さない。過去の実行に保存された Summary は書き換わらない。

## 公開履歴

各データセット・配布先の `history.json` に、最新版として公開した記録を古い順で残す。URL は各データの README を参照する。形式は `schemaVersion: 1` と `entries` 配列で、各要素は次の項目を持つ。

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

## 開発ルール

作業の進め方は [AGENTS.md](AGENTS.md) を参照する。データを追加するときは種類ごとのフォルダにスクリプト・テスト・README をまとめ、種類ごとの README のデータ一覧からリンクする。

### ブランチのマージ方針

開発版の `dev` と正式版の `main` は継続して使い、変更は PR を通して取り込む。取り込み先に応じて、次のマージ方法を選ぶ。

| 取り込み元 → 取り込み先 | マージ方法 |
|---|---|
| 作業ブランチ → `dev` | スカッシュマージ（Squash and merge） |
| `dev` → `main` | 通常のマージコミット（Create a merge commit） |
| `main` → `dev`（正式版だけに入れた修正の反映） | 通常のマージコミット（Create a merge commit） |

作業ブランチの変更は、1つの目的を1つのコミットにまとめて `dev` に取り込む。`dev` で確認した変更を正式版に採用するときは、取り込み先を `main`、取り込み元を `dev` とした PR を作る。

`dev` → `main` は履歴を共有したまま取り込む。スカッシュすると、同じ変更が `main` では別のコミットになり、次の正式版への PR に取り込み済みのコミットが並んだり、競合が起きやすくなったりするため、この方向ではスカッシュやリベースでのマージを使わない。

正式版だけに修正を入れた場合は、`main` → `dev` の PR を作り、マージコミットで取り込む。これにより、次の開発・正式版公開にも修正と履歴を引き継ぐ。`dev` → `main` のマージ後も `dev` ブランチは削除しない。

GitHub のリポジトリ設定では、マージコミットとスカッシュマージの両方を許可する。マージはユーザーが行い、エージェントは行わない。

### 変更の確かめ方

以下はリポジトリのルートから実行する。手動公開入口のテストには Python 3.12 を使い、外部通信は行わない。

```bash
# 改行コードと BOM の確認
.github/scripts/check-text-format.sh

# データ種別共通の手動公開入口のテスト
python -m unittest discover -s release_tools/tests -v
```

CI の「共通：文字形式の確認」はすべての PR で文字コード・改行を確認する。「共通：データ処理のテスト」（`.github/workflows/test.yml`）は push・PR 時に、共通の公開処理・地点の共通ビューア・OSM 山頂・国土地理院山頂・テスト用地点を別のジョブで確認する。データ種別を追加するときもこのActionにジョブを追加する。必須チェックの `test` ジョブは各テストの結果を集約し、すべて成功した場合だけ成功する。ジョブを追加するときは `test` の依存先と結果確認にも追加する。

データ固有のテストは各 README を参照する。

- [地点データのテスト](points/README.md#変更の確かめ方)

## ライセンス

特定の Release を使うときは、その Release の manifest に記載された出典・利用条件を確認する。

### 配布データ

| 対象 | 出典・表示 | 利用条件 | 補足・詳細 |
|---|---|---|---|
| [OSM 山頂](points/osm-peaks/README.md)（`osm-peaks`） | © OpenStreetMap contributors | [ODbL-1.0](https://opendatacommons.org/licenses/odbl/1-0/) | [OSM の著作権とライセンス](https://www.openstreetmap.org/copyright)を参照 |
| OSM 山頂の補足適用前の元データ・リリース済みアプリ向けの互換データ | © OpenStreetMap contributors | ODbL-1.0 | [互換データの扱い](points/osm-peaks/README.md#リリース済みアプリ向けの互換データ)を参照 |
| [国土地理院の山頂](points/gsi-peaks/README.md)（`gsi-peaks`） | 出典：国土地理院（日本の主な山岳標高一覧・自然地名・標高タイル）を加工して作成 | [公共データ利用規約 第 1.0 版](https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html)（`PDL-1.0`） | 元資料 URL・取得日時・出典表示は [snapshot.json](points/gsi-peaks/snapshot.json) に記録 |
| [テスト用地点](points/testdata/README.md)（`testdata`、開発版のみ） | 山むきの動作確認用に作成した架空データ | CC0-1.0 | 内蔵 SVG と分類タグを含む |

### 補足データ・画像

| 対象 | 出典・表示 | 利用条件 | 補足・詳細 |
|---|---|---|---|
| 日本百名山の山名一覧 | Wikipedia「日本百名山」の投稿者 | CC BY-SA 4.0 | 参照した版・投稿者へのリンク・加工内容は [supplements.json の `sources`](points/osm-peaks/supplements.json) に記録 |
| 日本百名山の照合に用いた地点データ | © OpenStreetMap contributors | ODbL-1.0 | 補足の出典は `sources` に記録し、配布データの manifest の `supplementSources` に引き継ぐ。元の地点データと補足の利用条件をそれぞれ確認する |
| 地点に追加する画像 | 画像ごとの取得元・作成者 | 画像ごとの利用条件 | 追加時にデータセットの README に記載する |

### 地点データビューアが利用する外部データ

[地点データビューア（points/viewer.html）](points/viewer.html) が地図表示と所在地判定に利用するデータ。

| 対象 | 出典 | 利用条件 | 補足・詳細 |
|---|---|---|---|
| 所在地判定用の行政区域 | 共同通信社の [japan-choropleth](https://github.com/kyodo-official/japan-choropleth) が加工した国土交通省「国土数値情報 行政区域データ 2025 年版」 | CC BY 4.0 | ビューアが取得し、配布する地点 JSON・補足 JSON には含めない |
| 背景地図 | 地理院タイル | [地理院タイルの案内](https://maps.gsi.go.jp/development/ichiran.html)を参照 | ビューアが取得し、配布する地点 JSON・補足 JSON には含めない |

### コード・文書

| 対象 | ライセンスの指定状況 |
|---|---|
| リポジトリのコード・文書全体 | 全体に適用するライセンスファイルは置いていない。配布データの利用条件をコードや文書全体のライセンスとして扱わない |

# yamamuki-data

山の地点データなどを作り、誰でも使える形で配るリポジトリ。[山むきアプリ](https://github.com/shohei0205/yamamuki)もこのデータを使っている。

地点データを地理院地図に重ねて表示する [データビューワ](viewer/README.md) を用意しています。

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
LICENSES/          Release に添付するライセンスの本文
.github/workflows/ GitHub Actions の定義
```

データの種類ごとにフォルダを分け、内部の構成は各 README に記載する。Actions の定義は `.github/workflows/` に置く。

## データの配布

- [正式版の Release 一覧](https://github.com/shohei0205/yamamuki-data/releases?q=draft%3Afalse+prerelease%3Afalse)
- [下書き Release の一覧](https://github.com/shohei0205/yamamuki-data/releases?q=draft%3Atrue)（リポジトリへの書き込み権限が必要）

### 配布の方針

生成したデータは [GitHub Releases](https://github.com/shohei0205/yamamuki-data/releases) に置く。元データとデータ本体は原則として git に含めない。国土地理院版は補足適用前の地点 JSON と出典要約 JSON をコミットし、Action で補足を適用して Release を作る。正式公開後の `manifest.json` は GitHub Pages に掲載する。

OSM 山頂・国土地理院山頂の生成 Action は、最終タグの下書きに配布ファイルを保存する。同じ Release を Pre-release として試用公開し、アプリで確認したあとに正式公開する。利用する側で Release の版を選び、その Release に添付された manifest の URL を固定して取得する。アプリが読んでいる版のデータが消えないよう、一般公開した Pre-release・正式版とその添付ファイルは保持する。生成 Action の Draft には整理用マーカーを付ける。マーカー付き Draft は、作成から 7 日以上経つと週 1 回の整理対象になる。

データの種類ごとに公開時期と最新版の参照先を分ける。リポジトリ全体の `releases/latest` はアプリの参照先に使わない。具体的な URL とタグは各データの文書を参照する。

山頂データの下書き作成・試用公開・正式公開は `main` で手動実行する。作業ブランチでは生成・検査を行い、配布ファイルを Actions の Artifacts に保存する。試用公開から正式公開まで、タグと配布ファイルの URL は同じ。

Actions とローカルのどちらで生成したデータも、配布ファイルは [Releases](https://github.com/shohei0205/yamamuki-data/releases) の **Assets** に添付する。データの種類と版は Release のタグで、試用公開と正式公開は Pre-release の設定で区別する。ローカル生成のデータは、添付するファイルと検証方法をデータごとの README に記載する。

正式公開・復旧で、正式版の manifest・カタログ・履歴を更新する。既存の開発版カタログとその参照先は公開当時の内容で残す。サイトは Actions から直接配置する。

データごとの manifest と公開履歴の URL、データ本体の取得方法は、各データの文書にまとめる。

### 配布データの取得情報（manifest.json）

データ本体の取得先・大きさ・ハッシュ・出典を記載する共通の JSON。項目・制約・読み込み方法は [manifest フォーマット](formats/manifest.md) を、各 JSON の共通情報は [metadata フォーマット](formats/metadata.md) を参照する。

## Actions の構成

共通の確認・公開操作は「共通：」、データ固有の操作は「種類 / データ名：」でそろえる。種類全体に対する操作は「種類：」とする。テストの入口は1つにまとめ、内部のジョブを共通処理・データごとに分ける。生成は取得元や実行周期に応じてデータ別のActionにする。

| Action | 対象 |
|---|---|
| 共通：データ処理のテスト | 共通処理と各データのテスト |
| 共通：文字形式の確認 | 改行コードとBOM |
| 共通：Draft-release を試用公開 | 下書きを Pre-release として公開 |
| 共通：Pre-release を正式公開 | 試用版を正式公開し、配布カタログを更新 |
| データビューワ：Release | [Windows・macOS のビューワを手動公開](viewer/README.md#配布版の公開) |

データ固有のActionと操作手順は各READMEを参照する。

## 確認済みデータの公開

下書きの生成、試用公開、正式公開は別の Action として `main` で手動実行する。

1. 生成 Action の Summary から下書き Release を開き、検査結果とデータの差分を確認する。
2. 「共通：Draft-release を試用公開」で、`tag` に下書きの URL またはタグを指定する。Action が添付ファイルを再検証し、同じタグの Pre-release として一般公開する。
3. 試用公開した Release を、利用するアプリの開発版で確認する。
4. 確認が済んだら、「共通：Pre-release を正式公開」で同じタグを指定する。Action が再検証して Pre-release の指定を解除し、正式版カタログを更新する。

`sha256` は空欄なら Release の検査結果から取得する。確認内容と公開理由は任意で、正式公開時に Release の説明へ記録する。Pages に初めて配置する場合は `initialize_pages` を選ぶ。

手動公開の検証は `release_tools/point_release.py`、Release の取得・Pages・公開履歴の共通処理は `release_tools/release_data.py` にまとめる。OSM の生成時の比較と下書き作成は `points/osm-peaks/scripts/release_data.py`、GSI の検証・snapshot・配布ファイルの生成は `points/gsi-peaks/scripts/release_snapshot.py` に置き、共通の公開処理を呼び出す。OSM・国土地理院の山頂は同じ処理を使い、manifest の形式・版・取得 URL、データ本体のサイズ・SHA-256、地点の ID・座標・出典・件数、添付されたオリジナルと補足から配布地点を再現できるかを確認する。生成時の出典固有の検査は、各データの生成処理で行う。旧形式の Release は従来の検証を使う。

共通形式（manifest・地点 JSON とも版 6）の地点データは、`<データ ID>-<版>` と添付 manifest の `version` から対象を判定する。OSM 山頂は `osm-peaks-<version>`、国土地理院山頂は `gsi-peaks-<version>` を使う。試用公開から正式公開まで添付ファイルは同じものを使う。正式公開が成功したあとに `points/<データ ID>/manifest.json` と正式版カタログを更新する。地形など異なる形式のデータは、データごとの手順で検証して公開する。

生成 Action は Draft の本文に `<!-- yamamuki-data:temporary-draft:v1 -->` を付ける。作成から 7 日以上経ったマーカー付き Draft は、参照先を再確認する週次の整理 Action の対象になる。試用公開時にはマーカーを外し、以後は一般公開した版として保持する。

### 公開前の Draft を整理する

`main` で「地点：古い Draft を一覧にする」を実行し、Summary の候補を確認する。対象は作成から 7 日以上経ち、公開日時が記録されていない整理用マーカー付きの山頂 Draft。

削除する場合は「地点：古い Draft を削除する」にタグを 1 件指定する。Action はアプリ側とデータ側の参照先、Release の状態を再確認し、参照されている版や Pre-release・正式版は保持する。取得・照合に失敗した場合は停止する。Git タグは削除しない。

毎週月曜の日本時間 12:00 に「地点：古い Draft を定期整理」も同じ条件で実行する。整理用マーカーは `<!-- yamamuki-data:temporary-draft:v1 -->`。試用公開時にマーカーを外す。Draft を長く保持する場合は、本文からこのマーカーを取り除く。

Pre-release として一般公開した版は、正式公開しない場合も添付ファイルと取得 URL を保持する。

### Pages への配置と参照先の復旧

利用開始時に、リポジトリの Settings → Pages → Build and deployment の Source を **GitHub Actions** にし、`github-pages` 環境の配置元として `main` を許可する。

確認済みデータの正式公開・復旧では、Actions で manifest のサイトを生成し、`upload-pages-artifact` と `deploy-pages` で GitHub Pages に直接配置する。ソースは `main` に置き、配布専用ブランチや生成物のコミットは作らない。

サイト全体の更新には `/catalog.json` を使う。`schemaVersion: 1` とし、`manifests` にパスと manifest の対応、公開履歴も保存する。既存一覧からサイト全体を作り直し、対象のデータ・配布先だけを置き換える。他のデータ・配布先・履歴は引き継ぐ。これは [データ配布用の地点カタログ](points/README.md#公開データのカタログ) とは別の一覧である。

Pages を更新するジョブは共通の `publish-data-pages` グループで直列化する。配置後は manifest 一覧・公開履歴・地点カタログが生成内容と一致するまで最大 55 秒待ち、反映を確認してからジョブを完了する。既存一覧の取得や検証に失敗した場合は配置しない。404 の場合も通常の公開では停止し、初期化を明示した初回の手動公開だけが一覧を新しく作る。

配置時にファイルを削除してから上げ直す時間は生じない。キャッシュにより更新前の manifest が返る場合はあるが、履歴版を残すため、その manifest でも対応するデータ本体を取得できる。アプリは通信や検証に失敗したら保存済みデータを維持して再試行する。

参照先の更新に失敗した場合は、「共通：Pre-release を正式公開」で、公開済みの正式版の URL またはタグを指定して復旧する。履歴版のデータ本体は再生成・再アップロードしない。公開済みの履歴 Release には不変化を適用できる。

### 実行概要の確認

OSM 山頂の生成と共通の公開 Action は、処理の冒頭に Summary へ実行パラメーターを表示する。ブランチ・イベント・実行者・実行回数・配布先に加え、手動公開では操作と入力 URL またはタグ・SHA-256・確認内容と公開理由・Pages 初期化の指定を表示する。空欄の場合の扱いも明記する。データ固有の入力項目と検査結果は各 README を参照する。

Pages を更新する公開ジョブの Summary には、公開後の Release の URL、配置する manifest の固定 URL と Pages の配置結果を表示する。`main` で山頂データを生成したときは、Summary に下書き Release の URL とタグを表示する。試用公開では Release の URL とタグを表示し、Pages の参照先は正式公開・復旧時に更新する。単体テストのダミーデータは Summary に出さない。過去の実行に保存された Summary は書き換わらない。

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

作業ブランチから `main` に PR を作り、スカッシュマージで取り込む。GitHub のリポジトリ設定でスカッシュマージを許可し、`main` のブランチ保護で PR と必要な CI を要求する。

`dev` ブランチを廃止するときは、先に移行の変更を `main` に取り込む。`dev` 宛ての PR があれば取り込み先を変更し、`github-pages` 環境の配置元を `main` にそろえる。`dev` の変更がすべて取り込まれ、実行中の公開ジョブが完了したことを確認する。`dev` を対象にする削除制限を解除してからブランチを削除する。公開済みの開発版には従来のタグと URL を使う。

### 変更の確かめ方

以下はリポジトリのルートから実行する。手動公開入口のテストには Python 3.12 を使い、外部通信は行わない。

```bash
# 改行コードと BOM の確認
.github/scripts/check-text-format.sh

# データ種別共通の手動公開入口のテスト
python -m unittest discover -s release_tools/tests -v
```

CI の「共通：文字形式の確認」はすべての PR で文字コード・改行を確認する。「共通：データ処理のテスト」（`.github/workflows/test.yml`）は push・PR 時に、共通の公開処理・地点の共通ビューア・データビューワ（Windows・macOS）・OSM 山頂・国土地理院山頂を別のジョブで確認する。データ種別を追加するときもこのActionにジョブを追加する。必須チェックの `test` ジョブは各テストの結果を集約し、すべて成功した場合だけ成功する。ジョブを追加するときは `test` の依存先と結果確認にも追加する。

データ固有のテストは各 README を参照する。

- [地点データのテスト](points/README.md#変更の確かめ方)

## ライセンス

配布データの利用条件は出典ごとに異なる。利用する Release の manifest と、各データセットの「出典とライセンス」を確認する。補足適用後のデータを使う場合は、元データと補足の両方の出典・利用条件を確認する。

### 配布データと補足

| 対象 | 出典・帰属表示 | 利用条件 | 詳細 |
|---|---|---|---|
| OpenStreetMap 由来の地点データ | © OpenStreetMap contributors | [ODbL 1.0](https://opendatacommons.org/licenses/odbl/1-0/) | [OSM 山頂の出典とライセンス](points/osm-peaks/README.md#出典とライセンス)。補足適用前の原本と旧形式のデータも含む |
| 国土地理院由来の地点データ・OSM版の地点照合と補正に用いた情報 | 出典：国土地理院。資料名と加工内容を記録する | [公共データ利用規約（第1.0版）](https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html)（PDL-1.0） | [GSI 山頂](points/gsi-peaks/README.md#出典とライセンス)・[OSM 山頂](points/osm-peaks/README.md#出典とライセンス) の出典とライセンス |
| 日本百名山の山名一覧 | Wikipedia「日本百名山」の投稿者 | [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) | [GSI 山頂](points/gsi-peaks/README.md#出典とライセンス)・[OSM 山頂](points/osm-peaks/README.md#出典とライセンス) |
| 公開済みの動作確認用地点データ | このリポジトリで作成 | [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/) | [開発版カタログ](https://shohei0205.github.io/yamamuki-data/points/catalog-dev.json) の `testdata` |

地点に追加する画像は、画像ごとの出典・利用条件を各データセットの文書で確認する。

### 配布データの作成者と利用条件のファイル

配布データは、上の表の元データを [The yamamuki Authors](AUTHORS.md) が選び、照合・補正して作っている。作成者は manifest の `metadata.authors` に記録し、元データの作者とは分けて扱う。元データの出典・利用条件・帰属表示は、これまでどおり `metadata.sources` に記録する。

各 Release には、そのデータの利用条件をまとめた `LICENSE.txt` を添付し、manifest の `licenseUrl` から参照できるようにする。`LICENSE.txt` には、データ全体のライセンス、出典ごとの利用条件と帰属表示、作成者、表示のお願いを記載する。OpenStreetMap 由来のデータは ODbL 1.0 の派生データベースとして配るため、本文 `ODbL-1.0.txt`（[LICENSES/ODbL-1.0.txt](LICENSES/ODbL-1.0.txt) の写し）も添付する。国土地理院由来のデータは、データ全体に一つのライセンスを定めず、出典ごとの利用条件に従う。

元データのライセンスが求める表示に加えて、できれば作成者も表示してほしい。表示の例は次のとおり。この表示はお願いで、義務ではない。

```text
山データ：yamamuki-data（The yamamuki Authors）
```

### ビューアーが利用する外部データ

[地点データビューアー](points/viewer.html) は、背景地図と所在地判定用の境界データを取得する。これらは配布する地点データや補足には含めない。

| 用途 | 出典 | 利用条件・詳細 |
|---|---|---|
| 背景地図 | 地理院タイル | [地理院タイルの案内](https://maps.gsi.go.jp/development/ichiran.html) |
| 所在地判定 | 共同通信社の [japan-choropleth](https://github.com/kyodo-official/japan-choropleth) が加工した国土交通省「国土数値情報 行政区域データ 2025 年版」 | CC BY 4.0。[ビューアーの説明](points/README.md#地点データをブラウザで確認する) |

### コード・文書

リポジトリのコード・文書全体に適用するライセンスファイルは置いていない。配布データの利用条件を、コードや文書全体のライセンスとして扱わない。

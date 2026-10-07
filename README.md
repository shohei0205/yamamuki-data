# yamamuki-data

山むきアプリ（[shohei0205/yamamuki](https://github.com/shohei0205/yamamuki)）が使うデータを作って配るためのリポジトリ。

## データの種類

| 種類 | 詳細 |
|---|---|
| 地点データ | [データ一覧・共通形式・公開手順](points/README.md) |
| 地形データ | 将来追加予定・未実装 |

収録対象、データ本体の形式、ダウンロード先、生成・公開手順、検査基準は、種類ごとの文書にまとめる。

## フォルダ構成

```text
points/            地点データ（詳細は points/README.md）
schemas/           データ形式ごとの版別スキーマ
release_tools/     データ種別共通の公開入口
.github/workflows/ GitHub Actions の定義
```

データの種類ごとにフォルダを分け、内部の構成は各 README に記載する。Actions の定義は `.github/workflows/` に置く。

## 配布の方針

生成したデータは [GitHub Releases](https://github.com/shohei0205/yamamuki-data/releases) に置く。元データとデータ本体は git に含めない。最新版を示す小さな `manifest.json` は、Actions の生成物として GitHub Pages に公開する。

データの種類ごとに公開時期と最新版の参照先を分ける。正式版と開発版も独立して扱い、リポジトリ全体の `releases/latest` はアプリの参照先に使わない。具体的な URL とタグは各データの文書を参照する。

| ブランチ | 配布先 |
|---|---|
| `main` | 正式版 |
| `dev` | 開発版（Pre-release） |

開発中の処理は `dev` で確認し、正式版に採用するときは変更を `main` に取り込む。実行タイミングと確認方法はデータごとに定める。

正式版も開発版も、このリポジトリの同じ [Releases 一覧](https://github.com/shohei0205/yamamuki-data/releases) に公開する。`main`・`dev` は生成処理の実行元で、配布ファイルは各 Release の **Assets** に添付する。データの種類と正式版・開発版の違いは、Release のタグで区別する。

最新版を示す manifest は GitHub Pages に置く。`main`・`dev` のコードでサイトのファイルを生成して直接配置するため、配布専用ブランチは作らない。生成物も git に入れない。

データごとの manifest と公開履歴の URL、データ本体の取得方法は、各データの文書にまとめる。

## manifest.json

このリポジトリで配布する各データセットに共通する、UTF-8（BOM なし）の JSON オブジェクト。データ本体の取得先・大きさ・ハッシュ・出典を記載する。`schemaVersion` は manifest 自体の形式の版を表す。現在は **版5**。データ本体のスキーマの版は独立して管理し、manifest の版と同じ番号であるとは限らない。使用するデータ本体の版は `dataSchemaVersion` に記載する。データ本体の構造は種類ごとの仕様に従い、地点データは [points/README.md](points/README.md#地点の形式) に記載する。

機械検証用の [manifest の版5](schemas/manifest/manifest-v5.schema.json)（JSON Schema Draft 2020-12）を用意している。共通スキーマはデータ種別ごとの追加項目を許可する。

`format` の検査を有効にした検証ツールを使う。日時の前後関係・本体のサイズ・ハッシュ・件数・取得 URL と Release の一致は JSON Schema では照合できないため、公開処理で別途検証する。

### 項目

| 項目 | JSON の型 | 必須 | 内容・制約 |
|---|---|---|---|
| `schemaVersion` | integer | 必須 | 形式の版。新規生成は `5` |
| `dataSchemaVersion` | integer | 新規生成では必須 | データ本体のスキーマの版。正の整数。manifest の `schemaVersion` と独立して管理する |
| `name` | string | 任意 | データセットの表示名 |
| `version` | string | 必須 | データセットの配布版。英数字で始まり、英数字・ピリオド・ハイフン・下線で構成する |
| `downloadUrl` | string | 必須 | この版の gzip データ本体を取得する HTTPS URL |
| `fileName` | string | 必須 | gzip データ本体のファイル名。データセットごとに定める |
| `sha256` | string | 必須 | gzip ファイルそのものの SHA-256。小文字の16進数64文字 |
| `sizeBytes` | integer | 必須 | gzip ファイルのバイト数。正の整数 |
| `uncompressedSizeBytes` | integer | 必須 | 展開後の JSON のバイト数。正の整数 |
| `sourceTimestamp` | string | 任意 | 元データ全体の基準日時。取得日時とは区別する。不明なら省略 |
| `sourceUrl` | string | 任意 | 元データの取得・参照 URL。特定できない場合は省略 |
| `license` | string | 必須 | データセットの利用条件。OSM 由来では `ODbL-1.0` |
| `attribution` | string | 必須 | 表示すべき出典。OSM 由来では `© OpenStreetMap contributors` |

データ種別ごとの件数・更新日時などの追加項目と制約は、そのデータの仕様に記載する。日時は UTC の ISO 8601 形式（例: `2026-09-30T20:21:22Z`）を使い、記録していない日時を推測して埋めない。

### 読み込みと公開

利用するアプリは対応する `schemaVersion` を確認し、`downloadUrl` から本体を取得する。展開前に `sizeBytes` と `sha256`、展開後に `uncompressedSizeBytes` を照合し。データ種別ごとの件数なども各仕様に従って照合する。通信や検証に失敗した場合は保存済みデータを維持する。

データセットと正式版・開発版ごとに最新版の manifest の URL を分ける。Release に添付する manifest と最新版として配置する manifest は同じ内容を使い、公開済みの版の本体は差し替えない。公開先の URL・Release タグ・ファイル名は各データセットの資料に記載する。

スキーマファイルは種類ごとに保存する。manifest は `schemas/manifest/manifest-v<版>.schema.json` に置き、データ本体などのスキーマの置き場所は種類ごとの資料に記載する。それぞれ必要なときに独立して版を上げ、公開済みの版は原則変更しない。

## Actions の構成

共通の確認・公開操作は「共通：」、データ固有の操作は「種類 / データ名：」でそろえる。種類全体に対する操作は「種類：」とする。テストの入口は1つにまとめ、内部のジョブを共通処理・データごとに分ける。生成は取得元や実行周期に応じてデータ別のActionにする。

| Action | 対象 |
|---|---|
| 共通：データ処理のテスト | 共通処理と各データのテスト |
| 共通：文字形式の確認 | 改行コードとBOM |
| 共通：確認済みデータを公開 | データ種別共通の手動公開 |

データ固有のActionと操作手順は各READMEを参照する。

## 確認済みデータの公開

Actions の「共通：確認済みデータを公開」を共通の入口とする。確認した Release の URL（またはタグ）を入力すると、タグからデータ種別を判定し、その種別の検証・公開処理を実行する。正式版は `main`、開発版は `dev` を選ぶ。確認理由は任意。SHA-256 は検査結果から自動取得できる。

共通の振り分け処理は `release_tools/`、データ固有の検証・公開処理は各データのフォルダに置く。対応する種別と固有の公開手順は各 README を参照する。データ種別を追加するときは `release_tools/publish_reviewed.py` の登録表に追加する。未対応の種別や最新版参照タグを公開対象に指定すると停止する。

Pages を更新するジョブは共通の `publish-data-pages` グループで直列化し、既存のサイトを引き継いでから配置する。正式版・開発版は別のパスに保ち、片方の公開で他方の参照先を消さない。

最新版 manifest の固定 URL と Pages の配置結果は公開ジョブの Summary に表示する。下書きで保留された場合は参照先を更新しない。

両方の Actions（生成・検査と確認済みデータの公開）は、処理の冒頭に Summary へ実行パラメーターを表示する。未入力の場合の扱いも明記する。生成した Release へのリンクも Summary に表示し、下書きの確認へ移動できる。

## ブランチのマージ方針

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

## 変更の確かめ方

以下はリポジトリのルートから実行する。手動公開入口のテストには Python 3.12 を使い、外部通信は行わない。

```bash
# 改行コードと BOM の確認
.github/scripts/check-text-format.sh

# データ種別共通の手動公開入口のテスト
python -m unittest discover -s release_tools/tests -v
```

CI の「共通：文字形式の確認」はすべての PR で文字コード・改行を確認する。「共通：データ処理のテスト」（`.github/workflows/test.yml`）は push・PR 時に、共通の公開処理・OSM山頂・テスト用地点を別のジョブで確認する。データ種別を追加するときもこのActionにジョブを追加する。

データ固有のテストは各 README を参照する。

- [地点データのテスト](points/README.md#変更の確かめ方)

## 開発ルール

作業の進め方は [AGENTS.md](AGENTS.md) を参照する。データを追加するときは種類ごとのフォルダにスクリプト・テスト・README をまとめ、種類ごとの README のデータ一覧からリンクする。

## ライセンス

データセットごとに出典と利用条件を確認し、各 README と manifest に記載する。アプリなどで利用する場合も、対応する出典とライセンスを表示する。

OpenStreetMap 由来のデータは **© OpenStreetMap contributors** を表示し、[Open Database License（ODbL）1.0](https://opendatacommons.org/licenses/odbl/1-0/) に従って利用・再配布する。[OpenStreetMap の著作権とライセンス](https://www.openstreetmap.org/copyright)を参照。

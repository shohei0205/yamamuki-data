# yamamuki-data

山むきアプリ（[shohei0205/yamamuki](https://github.com/shohei0205/yamamuki)）が使うデータを作って配るためのリポジトリ。

## データ一覧

| データ | 状態 | 詳細 |
|---|---|---|
| 山頂 | 生成・検査・公開の処理を実装 | [山頂データの仕様と運用](points/osm_peaks/README.md) |
| ランドマークなどの地点 | 将来追加予定・未実装 | [地点データの共通仕様](points/README.md) |
| 地形 | 将来追加予定・未実装 | 実装時に専用の文書を追加する |

データごとの収録対象、JSON の形式、ダウンロード先、生成・公開手順、検査基準は、それぞれの文書にまとめる。

## フォルダ構成

```text
points/
  README.md         地点データの共通形式・追加方針
  osm_peaks/
    README.md       OSM 山頂データの生成・公開手順
    scripts/        OSM 山頂データの取得・生成・検査・公開
    tests/          OSM 山頂データのテスト
schemas/
  manifest/         manifest の版別スキーマ（manifest-v5.schema.json など）
  points/           地点データと地点カタログの版別スキーマ
release_tools/      データ種別共通の手動公開入口
.github/workflows/ GitHub Actions の定義
```

地点 ID はデータセット内で一意な文字列（例: `"3403990450"`、`"example-tower"`）とし、アプリ側では「データセット名＋id」で区別する。山頂の新規生成は schemaVersion 5 で、旧版の整数 `osmId` から変更するため、利用するアプリの対応確認後に公開する。

地点データの共通形式は [points/README.md](points/README.md) を参照する。取得元・対象ごとの生成処理は `points/` の下に置く。山頂の manifest は `points/osm_peaks/`、開発版は `points/osm_peaks-dev/` から配る。Release タグは従来の `peaks` を維持する。

地形データの実装時は `terrain/` を追加する。Actions の定義は `.github/workflows/` に置き、データごとのフォルダを作業ディレクトリにして処理を実行する。

## 配布の方針

生成したデータは [GitHub Releases](https://github.com/shohei0205/yamamuki-data/releases) に置く。元データとデータ本体は git に含めない。最新版を示す小さな `manifest.json` は、Actions の生成物として GitHub Pages に公開する。

データの種類ごとに公開時期と最新版の参照先を分ける。正式版と開発版も独立して扱い、リポジトリ全体の `releases/latest` はアプリの参照先に使わない。具体的な URL とタグは各データの文書を参照する。山頂データの manifest には、実際に取得・検証した日付付き URL を `sourceUrl` として記録する。

| ブランチ | 配布先 |
|---|---|
| `main` | 正式版 |
| `dev` | 開発版（Pre-release） |

開発中の処理は `dev` で確認し、正式版に採用するときは変更を `main` に取り込む。実行タイミングと確認方法はデータごとに定める。山頂データは手動実行時に取得対象日も指定できる（[取得対象日の指定](points/osm_peaks/README.md#取得対象日を指定する)）。

正式版も開発版も、このリポジトリの同じ [Releases 一覧](https://github.com/shohei0205/yamamuki-data/releases) に公開する。`main`・`dev` は生成処理の実行元で、配布ファイルは各 Release の **Assets** に添付する。データの種類と正式版・開発版の違いは、Release のタグで区別する。

山頂データ本体は、正式版の `peaks-<version>` と開発版の `peaks-dev-<version>` の Release に置く。各版の Assets は `manifest.json` と `japan-mountains.json.gz` で、公開後は差し替えない。

最新版を示す manifest は GitHub Pages に置く。`main`・`dev` のコードでサイトのファイルを生成して直接配置するため、配布専用ブランチは作らない。生成物も git に入れない。

データごとの manifest と公開履歴の URL、データ本体の取得方法は、各データの文書にまとめる。山頂データは [置き場所](points/osm_peaks/README.md#置き場所)と[公開履歴](points/osm_peaks/README.md#公開履歴)を参照する。

## manifest.json

このリポジトリで配布する各データセットに共通する、UTF-8（BOM なし）の JSON オブジェクト。データ本体の取得先・大きさ・ハッシュ・出典を記載する。`schemaVersion` は manifest 自体の形式の版を表す。現在は **版5**。データ本体のスキーマの版は独立して管理し、manifest の版と同じ番号であるとは限らない。使用するデータ本体の版は `dataSchemaVersion` に記載する。データ本体の構造は種類ごとの仕様に従い、地点データは [points/README.md](points/README.md#地点の形式) に記載する。

機械検証用の [manifest の版5](schemas/manifest/manifest-v5.schema.json)（JSON Schema Draft 2020-12）を用意している。地点データの manifest では同ファイルの `#/$defs/pointManifest` を使い、`pointCount` も必須として検査する。共通スキーマはデータ種別ごとの追加項目を許可する。版1〜4はこのスキーマの対象外で、既存の山頂検査処理で確認する。

`format` の検査を有効にした検証ツールを使う。日時の前後関係・本体のサイズ・ハッシュ・件数・取得 URL と Release の一致は JSON Schema では照合できないため、公開処理で別途検証する。

### 項目

| 項目 | JSON の型 | 必須 | 内容・制約 |
|---|---|---|---|
| `schemaVersion` | integer | 必須 | 形式の版。新規生成は `5` |
| `dataSchemaVersion` | integer | 新規生成では必須 | データ本体のスキーマの版。正の整数。manifest の `schemaVersion` と独立して管理する |
| `version` | string | 必須 | データセットの配布版。英数字で始まり、英数字・ピリオド・ハイフン・下線で構成する |
| `downloadUrl` | string | 必須 | この版の gzip データ本体を取得する HTTPS URL |
| `fileName` | string | 必須 | gzip データ本体のファイル名。データセットごとに定める |
| `sha256` | string | 必須 | gzip ファイルそのものの SHA-256。小文字の16進数64文字 |
| `sizeBytes` | integer | 必須 | gzip ファイルのバイト数。正の整数 |
| `uncompressedSizeBytes` | integer | 必須 | 展開後の JSON のバイト数。正の整数 |
| `pointCount` | integer | 地点データでは必須 | JSON 配列に収録した地点数。正の整数。地点以外のデータでは省略 |
| `sourceTimestamp` | string | 任意 | 元データ全体の基準日時。取得日時とは区別する。不明なら省略 |
| `latestPointTimestamp` | string | 地点データで任意 | 収録地点の元データ上の最終編集日時の最大値。不明なら省略 |
| `sourceUrl` | string | 任意 | 元データの取得・参照 URL。特定できない場合は省略 |
| `license` | string | 必須 | データセットの利用条件。OSM 由来では `ODbL-1.0` |
| `attribution` | string | 必須 | 表示すべき出典。OSM 由来では `© OpenStreetMap contributors` |

地点以外のデータに必要な件数・更新日時などの追加項目は、そのデータの仕様に記載する。`pointCount` や `latestPointTimestamp` に地点以外の情報を入れない。

日時は UTC の ISO 8601 形式（例: `2026-09-30T20:21:22Z`）で記載する。`latestPointTimestamp` は現地調査日・標高の測定日・生成日時ではない。両日時がある場合、`latestPointTimestamp` は `sourceTimestamp` 以下とする。手動作成のデータでも記録していない日時を推測して埋めない。データセットの仕様でこれらの任意項目を必須にできる。

`dataSchemaVersion` の追加は既存項目を変更しないため、manifest の版は5のままとする。追加前のデータを検証できるよう、JSON Schema 上は省略を許可するが、新規生成では必ず記載する。

### 読み込みと公開

利用するアプリは対応する `schemaVersion` を確認し、`downloadUrl` から本体を取得する。展開前に `sizeBytes` と `sha256`、展開後に `uncompressedSizeBytes` を照合し、地点データでは `pointCount` と配列の件数も照合する。通信や検証に失敗した場合は保存済みデータを維持する。

データセットと正式版・開発版ごとに最新版の manifest の URL を分ける。Release に添付する manifest と最新版として配置する manifest は同じ内容を使い、公開済みの版の本体は差し替えない。公開先の URL・Release タグ・ファイル名は各データセットの資料に記載する。

スキーマファイルは種類ごとに保存する。manifest は `schemas/manifest/manifest-v<版>.schema.json`、地点データと地点カタログは同じ `schemas/points/` 内の `pointdata-v<版>.schema.json`・`catalog-v<版>.schema.json` に置く。manifest と地点データはそれぞれ必要なときに版を上げ、公開済みの版は原則変更しない。版5より前のスキーマファイルは未作成。

地点データの公開済み一覧とダウンロード URL・サイズ・ハッシュなどは 正式版の [points/catalog.json](https://shohei0205.github.io/yamamuki-data/points/catalog.json)、開発版の [points/catalog-dev.json](https://shohei0205.github.io/yamamuki-data/points/catalog-dev.json) から取得できる。形式と使い方は [地点カタログの仕様](points/README.md#公開データのカタログ) を参照する。

## 確認済みデータの公開

Actions の「確認済みのデータを公開」を共通の入口とする。確認した Release の URL（またはタグ）を入力すると、タグからデータ種別を判定し、その種別の検証・公開処理を実行する。正式版は `main`、開発版は `dev` を選ぶ。確認理由は任意。SHA-256 は検査結果から自動取得できる。

共通の振り分け処理は `release_tools/`、データ固有の検証・公開処理は各データのフォルダに置く。現時点で対応するのは山頂（`peaks`）のみ。将来は地形用の検証・公開処理を実装し、`release_tools/publish_reviewed.py` の登録表に追加する。地形専用の手動公開アクションを増やす必要はない。地形の生成アクションや公開時期は別に設定できる。

山頂データの生成は配布先ごとに直列化する。自動公開と手動公開のジョブは共通の `publish-data-pages` グループで直列化し、既存のサイトを引き継いでから配置する。正式版・開発版は別のパスに保ち、片方の公開で他方の参照先を消さない。未対応の種別や最新版参照タグを公開対象に指定すると停止する。

最新版 manifest の固定 URL と Pages の配置結果は公開ジョブの Summary に表示する。下書きで保留された場合は参照先を更新しない。

両方の Actions（生成・検査と確認済みデータの公開）は、処理の冒頭に Summary へ実行パラメーターを表示する。未入力の場合の扱いも明記する。生成した Release へのリンクも Summary に表示し、下書きの確認へ移動できる。

全国 PBF は Actions のキャッシュに保存し、同じ配布日・MD5 のデータを再利用する。配布元の情報と復元ファイルは毎回照合し、キャッシュがない場合や一致しない場合は再取得する。

地点データの配布一覧から取り下げる場合は、手動 Actions「地点データをカタログから削除」を使う。操作と削除範囲は [地点カタログの削除手順](points/README.md#カタログから削除する) を参照する。

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

CI の「Text format」はすべての PR で文字コード・改行を確認する。共通の手動公開入口のテストは `.github/workflows/test.yml` で push・PR 時に実行する。

データ固有のテストは各 README を参照する。

- [山頂データのテスト](points/osm_peaks/README.md#テスト)

## 開発ルール

作業の進め方は [AGENTS.md](AGENTS.md) を参照する。データを追加するときは種類ごとのフォルダにスクリプト・テスト・README をまとめ、この README のデータ一覧からリンクする。

## ライセンス

データの出典は **© OpenStreetMap contributors**。配布データは [Open Database License（ODbL）1.0](https://opendatacommons.org/licenses/odbl/1-0/) に従って利用・再配布する。[OpenStreetMap の著作権とライセンス](https://www.openstreetmap.org/copyright)を参照。

元の日本全国データは [Geofabrik](https://download.geofabrik.de/asia/japan.html) が提供する。アプリなどで配布データを使う場合も、出典とライセンスを表示する。

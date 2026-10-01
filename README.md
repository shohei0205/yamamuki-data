# yamamuki-data

山むきアプリ（[shohei0205/yamamuki](https://github.com/shohei0205/yamamuki)）が使うデータを作って配るためのリポジトリ。

## データ一覧

| データ | 状態 | 詳細 |
|---|---|---|
| 山頂 | 生成・検査・公開の処理を実装 | [山頂データの仕様と運用](peaks/README.md) |
| 地形 | 将来追加予定・未実装 | 実装時に専用の文書を追加する |

データごとの収録対象、JSON の形式、ダウンロード先、生成・公開手順、検査基準は、それぞれの文書にまとめる。

## フォルダ構成

```text
peaks/
  README.md       山頂データの仕様・生成・公開手順
  scripts/        山頂データの取得・生成・検査・公開
  tests/          山頂データのテスト
.github/workflows/ GitHub Actions の定義
```

地形データの実装時は `terrain/` を追加する。Actions の定義は `.github/workflows/` に置き、データごとのフォルダを作業ディレクトリにして処理を実行する。

## 配布の方針

生成したデータは [GitHub Releases](https://github.com/shohei0205/yamamuki-data/releases) に置く。元データと生成物は git に含めない。

データの種類ごとに公開時期と最新版の参照先を分ける。正式版と開発版も独立して扱い、リポジトリ全体の `releases/latest` はアプリの参照先に使わない。具体的な URL とタグは各データの文書を参照する。山頂データの manifest には、実際に取得・検証した日付付き URL を `sourceUrl` として記録する。

| ブランチ | 配布先 |
|---|---|
| `main` | 正式版 |
| `dev` | 開発版（Pre-release） |

開発中の処理は `dev` で確認し、正式版に採用するときは変更を `main` に取り込む。実行タイミングと確認方法はデータごとに定める。山頂データは手動実行時に取得対象日も指定できる（[取得対象日の指定](peaks/README.md#取得対象日を指定する)）。

正式版も開発版も、このリポジトリの同じ [Releases 一覧](https://github.com/shohei0205/yamamuki-data/releases) に公開する。`main`・`dev` は生成処理の実行元で、配布ファイルは各 Release の **Assets** に添付する。データの種類と正式版・開発版の違いは、Release のタグで区別する。

山頂データでは、公開後に次の4種類の Release が並ぶ。`<version>` は生成ごとの版を表し、各版の Release は履歴として残す。

| Release のタグ | 役割 | Assets に置くファイル |
|---|---|---|
| [`peaks-latest`](https://github.com/shohei0205/yamamuki-data/releases/tag/peaks-latest) | 正式版の最新版を案内する固定の参照先 | `manifest.json` のみ |
| `peaks-<version>` | 正式版の各版 | `manifest.json` と `japan-mountains.json.gz` |
| [`peaks-dev-latest`](https://github.com/shohei0205/yamamuki-data/releases/tag/peaks-dev-latest) | 開発版の最新版を案内する固定の参照先（Pre-release） | `manifest.json` のみ |
| `peaks-dev-<version>` | 開発版の各版（Pre-release） | `manifest.json` と `japan-mountains.json.gz` |

アプリが正式版を取得するときは、まず `peaks-latest` の `manifest.json` を読み、その `version` に対応する `peaks-<version>` の Assets からデータ本体を取得する。開発版は同じ手順で `peaks-dev-latest` → `peaks-dev-<version>` を使う。新しい版の公開後に固定の参照先の manifest を更新するため、アプリは毎回 Releases 一覧から最新版を探す必要がない。

固定の参照先は初回の公開時に作成する。下書きの手動公開は Release ページの URL で指定でき（確認理由は任意）、SHA-256 の転記は不要（[手動公開の手順](peaks/README.md#確認済みの下書きを手動公開する)）。確認待ちの下書きはアプリの取得先に含めない。将来の地形データも同じ Releases 一覧に、山頂とは別のタグで追加する。取得 URL とファイル形式の詳細は [山頂データの仕様と運用](peaks/README.md) を参照する。

## 確認済みデータの公開

Actions の「確認済みのデータを公開」を共通の入口とする。確認した Release の URL（またはタグ）を入力すると、タグからデータ種別を判定し、その種別の検証・公開処理を実行する。正式版は `main`、開発版は `dev` を選ぶ。確認理由は任意。SHA-256 は検査結果から自動取得できる。

共通の振り分け処理は `release_tools/`、データ固有の検証・公開処理は各データのフォルダに置く。現時点で対応するのは山頂（`peaks`）のみ。将来は地形用の検証・公開処理を実装し、`release_tools/publish_reviewed.py` の登録表に追加する。地形専用の手動公開アクションを増やす必要はない。地形の生成アクションや公開時期は別に設定できる。

同じ種別・配布先の生成と公開は共通の実行グループ `publish-<種別>-<stable|dev>` で直列化する。別の種別・配布先は独立して動く。未対応の種別や最新版参照タグを公開対象に指定すると停止する。

最新版の固定ページには、現在のデータ本体へのリンク・件数・日時を表示する。公開時に説明と `manifest.json` を更新し、下書きで保留された場合は更新しない。

両方の Actions（生成・検査と確認済みデータの公開）は、処理の冒頭に Summary へ実行パラメーターを表示する。未入力の場合の扱いも明記する。生成した Release へのリンクも Summary に表示し、下書きの確認へ移動できる。

## 開発ルール

作業の進め方は [AGENTS.md](AGENTS.md) を参照する。データを追加するときは種類ごとのフォルダにスクリプト・テスト・README をまとめ、この README のデータ一覧からリンクする。

## ライセンス

データの出典は **© OpenStreetMap contributors**。配布データは [Open Database License（ODbL）1.0](https://opendatacommons.org/licenses/odbl/1-0/) に従って利用・再配布する。[OpenStreetMap の著作権とライセンス](https://www.openstreetmap.org/copyright)を参照。

元の日本全国データは [Geofabrik](https://download.geofabrik.de/asia/japan.html) が提供する。アプリなどで配布データを使う場合も、出典とライセンスを表示する。

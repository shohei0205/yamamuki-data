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

データの種類ごとに公開時期と最新版の参照先を分ける。正式版と開発版も独立して扱い、リポジトリ全体の `releases/latest` はアプリの参照先に使わない。具体的な URL とタグは各データの文書を参照する。

| ブランチ | 配布先 |
|---|---|
| `main` | 正式版 |
| `dev` | 開発版（Pre-release） |

開発中の処理は `dev` で確認し、正式版に採用するときは変更を `main` に取り込む。実行タイミングと確認方法はデータごとに定める。

## 開発ルール

作業の進め方は [AGENTS.md](AGENTS.md) を参照する。データを追加するときは種類ごとのフォルダにスクリプト・テスト・README をまとめ、この README のデータ一覧からリンクする。

## ライセンス

データの出典は **© OpenStreetMap contributors**。配布データは [Open Database License（ODbL）1.0](https://opendatacommons.org/licenses/odbl/1-0/) に従って利用・再配布する。[OpenStreetMap の著作権とライセンス](https://www.openstreetmap.org/copyright)を参照。

元の日本全国データは [Geofabrik](https://download.geofabrik.de/asia/japan.html) が提供する。アプリなどで配布データを使う場合も、出典とライセンスを表示する。

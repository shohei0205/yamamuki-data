# 開発ルール（Claude / Codex 共通）

山むきアプリ（[shohei0205/yamamuki](https://github.com/shohei0205/yamamuki)）が使う山データを作って配るリポジトリ。
言葉づかい、コミットメッセージ、PR、AI が投稿するコメントのルールは、yamamuki の [AGENTS.md](https://github.com/shohei0205/yamamuki/blob/main/AGENTS.md) に従う。ここにはこのリポジトリだけのことを書く。

## データの扱い

- 配るデータは OpenStreetMap 由来で、ODbL で利用・再配布している。README の「© OpenStreetMap contributors」の表示を消さない。
- 生成したデータと元データ（`*.osm.pbf`）は git に入れず、Releases に置く。
- `manifest.json` の形式を変えるときは `schemaVersion` を上げる。公開済みのアプリが読めなくなるので、yamamuki 側の対応と順番を決めてから出す。
- 山頂と地形は公開時期が異なるため、最新版の参照先をデータ種別ごとに分ける。山頂の正式版は `releases/download/peaks-latest/manifest.json` と `peaks-<version>`、開発版は `releases/download/peaks-dev-latest/manifest.json` と `peaks-dev-<version>` を使い、リポジトリ全体の `releases/latest` を使わない。アプリへの組み込み時もこの契約に合わせる。公開後に URL やファイル名を変える場合は、yamamuki 側の対応と順番を決める。

- 正式版は `main`、開発版は `dev` のコードから生成・公開する。配布先は実行元ブランチで決め、手動の配布先選択で取り違えないようにする。

## Markdown の文字コード

- Markdown（`.md`）は UTF-8 BOM 付きで保存する。改行は LF（`.gitattributes` と `.editorconfig` を参照）。

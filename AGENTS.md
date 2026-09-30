# 開発ルール（Claude / Codex 共通）

山むきアプリ（[shohei0205/yamamuki](https://github.com/shohei0205/yamamuki)）が使う山データを作って配るリポジトリ。
言葉づかい、コミットメッセージ、PR、AI が投稿するコメントのルールは、yamamuki の [AGENTS.md](https://github.com/shohei0205/yamamuki/blob/main/AGENTS.md) に従う。ここにはこのリポジトリだけのことを書く。

## データの扱い

- 配るデータは OpenStreetMap 由来で、ODbL で利用・再配布している。README の「© OpenStreetMap contributors」の表示を消さない。
- 生成したデータと元データ（`*.osm.pbf`）は git に入れず、Releases に置く。
- `manifest.json` の形式を変えるときは `schemaVersion` を上げる。公開済みのアプリが読めなくなるので、yamamuki 側の対応と順番を決めてから出す。
- Releases のファイル名と `releases/latest/download/` の URL はアプリに埋め込まれている。変えるときは yamamuki 側の対応を先に出す。

## Markdown の文字コード

- Markdown（`.md`）は UTF-8 BOM 付きで保存する。改行は LF（`.gitattributes` と `.editorconfig` を参照）。

# yamamuki-data

山むきアプリ（[shohei0205/yamamuki](https://github.com/shohei0205/yamamuki)）が使う山データを作って配るためのリポジトリ。アプリのコードとは分けて管理する（[yamamuki#35](https://github.com/shohei0205/yamamuki/issues/35)）。

## 配るもの

| データ | 中身 | 状態 |
|---|---|---|
| 全国の山頂 | OpenStreetMap の `natural=peak` と `natural=volcano`（日本全体で 1 ファイル、gzip） | 準備中（[yamamuki#40](https://github.com/shohei0205/yamamuki/issues/40)） |

## 置き場所

生成したデータと `manifest.json` は、このリポジトリの Releases に置く。アプリは認証なしで次の URL から最新版を取る（準備中のため、今はリポジトリを非公開にしている。アプリから取れるのは公開後）。

```text
https://github.com/shohei0205/yamamuki-data/releases/latest/download/manifest.json
```

`manifest.json` には `schemaVersion`、`version`、`sha256`、サイズ、元データの日付を書く。アプリは SHA-256 でデータが壊れていないか確かめてから取り込む。

## ライセンス

このリポジトリのコード（生成スクリプトや GitHub Actions の設定）は [MIT License](LICENSE) で公開する。

配るデータは OpenStreetMap から作ったもので、コードとは別に [Open Database License (ODbL) 1.0](https://opendatacommons.org/licenses/odbl/1-0/) の条件で利用・再配布する。データを使うときは、次の出典を表示する。

```text
© OpenStreetMap contributors
https://www.openstreetmap.org/copyright
```

- 元データ: [Geofabrik](https://download.geofabrik.de/asia/japan.html) が配布する日本の抽出データ（`japan-latest.osm.pbf`）
- このリポジトリで配るデータを加工して公開する場合も、ODbL に従い、同じ ODbL で公開する。

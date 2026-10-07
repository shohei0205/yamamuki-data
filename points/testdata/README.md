# テスト用の地点データ

[地点データの共通仕様に戻る](../README.md)

`points.json` は手動作成データの入力例で、山頂・駐車場・登山口など全種別の架空の地点を収録する。座標は表示確認のための仮の値で、実在する施設の位置を表さない。山頂には内蔵 SVG と `scale: 1.5` を設定し、画像と大きさを確認できる。山頂には架空の分類タグ `テスト用百名山` と `展望確認` を付け、複数タグも確認できる。実際の日本百名山への所属は表さない。OSM 由来ではないため `osmId` は付けない。ライセンスは CC0-1.0。

## 開発版への公開

Actions の「テスト用地点を開発版に公開」を、実行ブランチ `dev` で手動実行する。正式版からは実行できず、公開処理も `dev` 以外では停止する。通常の山頂データ公開で、既存の公開一覧を初期化しておく必要がある。

- データ本体: `testdata-dev-<version>` の Release の `test-points.json.gz`
- manifest: `points/testdata-dev/manifest.json`
- カタログ: [開発版カタログ](https://shohei0205.github.io/yamamuki-data/points/catalog-dev.json) の `testdata`

既存データと公開履歴を引き継ぎ、正式版カタログには追加しない。削除は共通のカタログ削除 Action を `dev` で実行し、データ ID に `testdata` を指定する。再公開すると再び追加される。定期公開は行わない。

## テスト

リポジトリ直下で実行する。Python の標準ライブラリだけで動作する。

```bash
python -m unittest discover -s release_tools/tests -v
python -m release_tools.test_points generate --version local-test
```

単体テストでは、入力の形式、SVG、生成物の整合性、dev 以外での拒否、他のデータを残したカタログ更新を確かめる。ローカル生成物は `points/testdata/dist/` に出力する。CI の共通テストでも実行する。

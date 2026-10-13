# 国土地理院の山頂 JSON の Release 公開

国土地理院から生成済みの全国 17,040 地点の JSON を配布する。元データの取得・地点の再抽出・標高補完は Action では実行しない。

## Action の入口

Actions の「地点 / 国土地理院山頂：JSON を Release に公開」から「Run workflow」を選ぶ。実行元が `main` なら `gsi-peaks-<version>` の Release、`dev` なら `gsi-peaks-dev-<version>` のプレリリースを作る。版には UTC 日時・実行番号・再実行番号を使う。

コミット済みの `gsi-peaks.json` を検証・圧縮し、`gsi-peaks.json`、`gsi-peaks.json.gz`、`manifest.json`、`snapshot.json` を添付する。下書きにアップロードした全ファイルを取り直し、一致を確認してから公開する。既存の Release は差し替えず、リポジトリ全体の Latest にも設定しない。Pages の manifest と地点カタログは更新しない。

## JSON の更新

元データからの生成は手元で行い、生成結果のフォルダーを指定して地点 JSON と出典要約を取り込む。

```bash
python -m release_tools.gsi_peaks snapshot --source build/gsi-national-all
git add points/gsi-peaks/gsi-peaks.json points/gsi-peaks/snapshot.json
```

両方の JSON を同じコミットで更新する。出典要約にはライセンス、取得日時の範囲、地点 JSON の SHA-256 を記録する。地点 JSON だけを編集するとハッシュが一致せず、公開処理は停止する。SQLite と元データのキャッシュはローカルで保管し、コミットしない。

## テスト

```bash
python -m unittest discover -s release_tools/tests -v
python -m release_tools.gsi_peaks prepare --channel stable --version local-test
```

公開用ファイルの準備は通信せずに行い、既定で `build/gsi-release/` に出力する。CI の共通テストでも、JSON と出典の整合性、正式版・開発版のタグ、公開前の添付ファイル照合を確かめる。

## 出典

国土地理院の日本の主な山岳標高一覧・自然地名・標高タイルを加工して作成した。公共データ利用規約（第 1.0 版）で利用する。元資料の URL と取得日時は `snapshot.json` に記録する。自然地名の代表点と標高タイルで補った地表標高は、公称の山頂位置・標高とは区別する。

出典・利用条件の一覧は、[ルート README のライセンス](../../README.md#ライセンス)を参照する。

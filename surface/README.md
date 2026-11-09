# 地表データ

地表データは、地図上に配置する画像タイルをレイヤーごとに収録する。配布ファイルとメタデータは [山むきSQLiteコンテナ仕様](../formats/sqlite.md) に従う。座標、レベル、画像の復号と合成は [地表画像のSQLite形式](formats/sqlite.md) を参照する。[データビューワ](../viewer/README.md) で開ける。

## データセット

| データセット | 元データと生成手順 |
| --- | --- |
| 日本全国の地表 | [ESA WorldCover](esa_worldcover/README.md) |

## フォルダー構成

| パス | 内容 |
| --- | --- |
| `formats/` | 地表画像の共通SQLite仕様 |
| `common/` | データセット共通のRustコード。SQLiteの読み書き、画像の復号、検査、統合を含む |
| `tools/` | 共通の検査、配布manifest作成、SQLite統合を行う `surface-tool` |
| `scripts/` | 共通の配布準備・公開と、元データの取得スクリプト |
| データセット別フォルダー | 元データの取得手順、生成器、配布内容 |

## 検査と統合

リポジトリのルートで実行する。生成したデータと元データは `build/` などのGit管理外に置く。生成手順と出典・利用条件はデータセットごとのREADMEを参照する。

```powershell
cargo test --locked --manifest-path surface/Cargo.toml -p surface-tool -p yamamuki-data-sqlite
cargo run --locked --release --manifest-path surface/Cargo.toml --bin surface-tool -- validate 入力.sqlite3
cargo run --locked --release --manifest-path surface/Cargo.toml --bin surface-tool -- manifest 入力.sqlite3 https://github.com/shohei0205/yamamuki-data/releases/download 2026-11-09T00:00:00Z
cargo run --locked --release --manifest-path surface/Cargo.toml --bin surface-tool -- merge 出力.sqlite3 入力1.sqlite3 入力2.sqlite3
```

`validate` はSQLiteの整合性、参照、地表画像を検査する。`manifest` は配布用のJSONを標準出力に書く。`merge` は出力に新規ファイルを指定し、データセットIDが重複するとエラーにする。

## 地表データの公開

公開対象は1データセットを収録したSQLite。`surface/<データセットのフォルダー>/release.json` に、台帳の `datasetId`、`versionPattern`、配布単位 `packages` を記載する。各配布単位で `releaseId`、`fileName`、収録するレイヤーIDの配列 `layers` を指定する。タグは `<releaseId>-<データ版>`。同じデータ版の一式とレイヤー別ファイルを並行して公開できるよう、配布単位ごとに異なる `releaseId` を使う。

```powershell
pwsh -NoProfile -File surface/scripts/prepare_release.ps1 `
  -Dataset '<データセットのフォルダー>' -Package all `
  -Database '入力.sqlite3' -OutputDirectory 'build/surface/配布準備' `
  -TargetCommit '<生成に使ったコミットの40桁のSHA>' -Publish
```

`-Publish` は下書き作成、添付、公開Actionの起動を行う。GitHub CLIでログインし、生成に使ったコミットを先にpushして、公開Actionを `main` に配置する。`-CreateDraft` は下書き作成まで行う。どちらも省くと配布ファイルの準備だけを行う。出力先は新規ディレクトリを指定する。生成日時は同じ場所のmanifestから引き継ぐ。記録がないファイルや抽出したSQLiteには、元の生成日時を `-DataTimestamp` にUTCで指定する。

Actionは添付したSQLite、manifest、ライセンスを取得し直し、収録画像、ハッシュ、データセットとレイヤーの構成、タグと対象コミットを照合してから同じ下書きを公開する。失敗時は下書きが残る。Actionの起動だけが失敗した場合は、表示された `gh workflow run` コマンドで再実行できる。

### レイヤー別配布の構造

通常は全レイヤーを1つのSQLiteに収録する。分割配布を採用する場合は、次の構造を使う。

| 単位 | 構造 |
| --- | --- |
| SQLite | ファイルごとに、選んだレイヤーの設定・レベル・画像タイルを収録する。元のデータセットIDとデータ版を引き継ぐ。タイル画像のBLOBはそのままコピーする |
| manifest | ファイルごとに作る。`surfaceLayerIds` で収録レイヤーを示し、`downloadUrl`・サイズ・SHA-256はそのSQLiteを指す |
| GitHub Release | 配布単位ごとに `releaseId` と版付きタグを分ける。各Releaseに対応するSQLite、manifest、ライセンスを添付する |

分割ファイルを同じデータセットとして扱う場合は、台帳のデータセットIDとデータ版を照合する。レイヤーは文字列の `id` で区別する。`dataset_key` と `layer_key` はファイル内の参照に使う。現行のビューワーには、分割ファイルを自動で集めて重ねる機能はない。

分割配布を行うときは、次のコマンドで抽出し、そのレイヤー構成に対応する `-Package` を指定して公開する。

```powershell
cargo run --locked --release --manifest-path surface/Cargo.toml --bin surface-tool -- `
  select-layers 入力.sqlite3 出力.sqlite3 forest
```

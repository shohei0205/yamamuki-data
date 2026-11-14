# 地形データ

[リポジトリの README に戻る](../README.md)

山の形を再現するための標高データ。自由配置の制御点をXYZタイルに分け、自然近傍補間で連続した地表面を再構成する。元DEM、作業用SQLite、配布SQLiteはコミットせず、配布物はGitHub Releasesへ添付する。

## データセット

| データセット | 内容 | 生成 |
|---|---|---|
| [国土地理院DEM](gsi_dem/README.md) | 日本の標高タイルから作るz6〜z13の地形 | ローカルのRust生成器 |

## 共通形式

[山むきSQLiteコンテナ](../formats/sqlite.md) の `kind = 'terrain'`、`format_version = 1` を使う。地表・地点と同じSQLiteへ統合できる。[地形版1の完全な仕様](formats/sqlite.md) に座標、量子化、独立・差分BLOB、復号順序と表示方法を定める。

配布SQLiteには制御点、差分の親参照、復元後XYZのSHA-256を保存する。三角形一覧、描画メッシュ、Voronoi探索用GPU補助情報は保存しない。表示時に作る。

## フォルダー構成

```text
terrain/
  common/          地形の圧縮・復号・補間と共通SQLite読取
  formats/         地形固有のSQLite形式
  gsi_dem/         国土地理院データの生成器・設定・公開設定
  tools/           作業結果を配布形式へ格納するCLIと全XYZ検証
  scripts/         共通Release処理の地形用入口
```

共通コンテナの実装は `surface/common`、manifest・Draftの共通処理は既存の `surface/tools`・`surface/scripts`・`release_tools` を共有する。データセット固有の地形生成は `gsi_dem` に閉じる。

## 検査

```sh
cargo test --locked --workspace --manifest-path terrain/Cargo.toml
cargo test --locked --workspace --manifest-path viewer/Cargo.toml
```

外部データを必要としない描画用データも作れる。これは配布用ではない。

```sh
cargo run --locked --release --manifest-path terrain/Cargo.toml --bin terrain-tool -- fixture build/terrain/fixture.sqlite3
cargo run --locked --release --manifest-path terrain/Cargo.toml --bin terrain-tool -- validate build/terrain/fixture.sqlite3
cargo run --locked --release --manifest-path viewer/Cargo.toml -- --check-terrain build/terrain/fixture.sqlite3
```

## 地形データの公開

生成・全画素監査をローカルで終えてから、[データセットの公開設定](gsi_dem/release.json) を使う。手順は [国土地理院DEM](gsi_dem/README.md#配布sqliteと公開) を参照。Release名、DraftではGitタグを作らないこと、試用公開から正式公開へ進める規則は [ルートREADME](../README.md#release名の規則) に従う。

「地形：下書き Release 加工・検査」はアップロードされたSQLiteを再復号して確認し、配布識別子をUTC日時・Run ID・試行番号で確定する。SQLite内の `data_version`、manifestの `version`、Releaseの予定タグをそろえる。加工だけでは一般公開しない。その後の試用公開・正式公開には共通Actionを使う。

Windowsで空き容量100GiB以上のドライブを指定できる場合、PNG準備から全レベル生成・保存後監査までのCLI試験も実行できる。

```powershell
$env:TERRAIN_SMOKE_ROOT = (Resolve-Path build/terrain).Path
cargo test --locked --release --manifest-path terrain/Cargo.toml -p terrain-gsi-dem --test cli -- --ignored --nocapture
```

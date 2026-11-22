# 地形データ

[リポジトリの README に戻る](../README.md)

山の形を再現するための標高データ。自由配置の制御点をXYZタイルに分け、自然近傍補間で連続した地表面を再構成する。元DEM、作業用SQLite、配布SQLiteはコミットせず、配布物はGitHub Releasesへ添付する。

## データセット

| データセット | 内容 | 生成 |
|---|---|---|
| [国土地理院DEM](gsi_dem/README.md) | 日本の標高タイルから作るz6〜z13の地形 | ローカルのRust生成器 |

## 共通形式

[山むきSQLiteコンテナ](../formats/sqlite.md) の `kind = 'terrain'`、`format_version = 1` を使う。地表・地点と同じSQLiteへ統合できる。現行ビューワで開ける構成は [統合 SQLite を開く](../viewer/README.md#統合-sqlite-を開く) を参照。[地形版1の完全な仕様](formats/sqlite.md) に座標、量子化、独立・差分BLOB、復号順序と表示方法を定める。

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

外部データを使わずに、描画確認用のSQLiteを作れる。リポジトリのルートで実行する。

```sh
mkdir -p build/terrain
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

## 既存SQLiteの再圧縮

整数XYZ・点の順序・親参照・データセット情報を保ち、制御点本文だけを最適化する。

```powershell
cargo run --release --locked --manifest-path terrain/Cargo.toml -p terrain-tool -- recompress build/terrain/gsi-terrain.sqlite3 build/terrain/gsi-terrain-optimized.sqlite3
```

出力は新規ファイルとし、既存のファイルを上書きしない。変換中と変換後に全XYZ・SHA-256・親依存を照合する。レベルごとの本文容量と復号時間も表示する。計測はI/O・親復元・SHA検査を除いた本文復号で、アプリの読み込み全体とは区別する。SQLiteの他のデータも保持する。

通常生成も、小さくなるタイルに大きなRiceブロック＋Zstandardを採用する。本文が増える場合は従来形式を維持する。形式の詳細は[地形SQLite形式](formats/sqlite.md#nqz1ndz1大きなriceブロックzstandard)を参照する。

圧縮候補の比較は `cargo run --release --locked --manifest-path terrain/Cargo.toml -p yamamuki-terrain --example codec_probe -- <SQLite> 1000` で実行できる。元データは従来形式のSQLiteを指定する。全域から一定間隔で約1,000タイルを抽出し、全XYZを照合して容量・復号時間を比較する。

全国z6〜z13の40,180タイルで全整数XYZ・順序・SHA-256・親依存を照合した結果：SQLite全体は223,195,648 → 216,030,208 bytes（約3.2%削減）、制御点本文は208,090,638 → 201,081,349 bytes（約3.4%削減）。28,542タイルを新方式へ変更した。

変更前の復号器と新方式を同じ全国タイルで各5回比較すると、本文復号の合計は1,205.886 → 778.641ms（約35%短縮）だった。I/O・親の読み込み・ハッシュ検査は含まない。既存形式を読む場合にも64bit単位のビット読取の最適化が適用される。Windowsで計測した値であり、アプリ全体の読込時間やmacOS実機の速度とは区別する。

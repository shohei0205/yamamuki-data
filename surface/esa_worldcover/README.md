# ESA WorldCover 日本全国の地表データ

ESA WorldCover 2021 v200から森林・水域を作り、**国土地理院の標高タイル（DEM10B）から生成した陰影**を重ねる。ESAで分類済みの画素はその分類を使う。ESAの値が0の画素は、対応するz12の標高が正の場合は「その他」、それ以外は水域とする。出力は [山むきSQLiteコンテナ](../../formats/sqlite.md) と配布manifest。地表レイヤーの読み込み・描画は [地表画像のSQLite形式](../formats/sqlite.md) を参照する。

[ESA WorldCover](https://esa-worldcover.org/en/data-access) · [国土地理院の標高タイル](https://maps.gsi.go.jp/development/ichiran.html)

## 対象地域と言語

新規に梱包する地表の共通 metadata には `coverage.regions: ["JP"]` を記録する。文字列の焼き込みはないため、既定では `language` を記載しない。中間 SQLite の `metadata` テーブルにある `manifest` に `coverage`・`language` が明示されていれば、その値を検証し、国・地域と言語を配布 SQLite に引き継ぐ。形式は [共通 metadata](../../formats/metadata.md#対象地域と提供範囲) を参照する。

提供範囲は配布 SQLite に実際に収録されたタイルの領域から生成する。`coverage.geometry` は全レイヤー・全 LOD の和集合、`coverage.finest.geometry` は `tilePixels × 2^z` が最大になる解像度のタイルの和集合とする。空の宣言レベルは使わず、透明なタイルも収録行があれば含める。画素ごとの有効範囲を表すものではない。

梱包時とレイヤー・LOD の抽出後に、そのファイルの内容から両方を再計算する。元の図形は引き継がず、国・地域と言語を保つ。公開準備でも、入力原本のハッシュを照合した後、配布用の複製だけを更新してから manifest と最終ハッシュを作る。原本は変更しない。作業用 SQLite を直接更新する場合は `surface-tool update-coverage <SQLiteファイル>` を使う。

## 必要なツール

- Rust stable、C/C++ビルドツール、CMake、`sqlite3` コマンド、PowerShell 7を用意する。Windowsでは Visual Studio Build Tools の「C++ によるデスクトップ開発」を使う。`cmake` と `sqlite3` は初回ビルド時に PATH から実行できるようにする。
- CargoはGDAL・PROJ・libwebpを同梱ソースから静的リンクする。`sqlite3` コマンドはビルド時にPROJの座標系データを作るために使う。
- ESAの取得には `curl`（Windowsでは `curl.exe`）を使う。

## 元データの収集

コマンドはリポジトリのルートで実行する。原本は `build/surface/source/` などのGit管理外に保存し、再生成時に使い回す。

### 国土地理院の標高タイル（DEM10B）

`download_gsi_dem10b.ps1` は、ズーム14のDEM10B PNGを地理院タイルから収集し、`-Output` の下に `<x>/<y>.png` として保存する。既存のPNGは再利用し、配信なし（HTTP 404）と取得失敗は別々に取得記録CSVへ出力する。[地理院タイル一覧の「標高タイル」](https://maps.gsi.go.jp/development/ichiran.html)

富士山周辺を緯度経度の範囲で取得する例。

```powershell
pwsh -NoProfile -File surface/scripts/download_gsi_dem10b.ps1 `
  -West 138.68 -South 35.32 -East 138.80 -North 35.42 `
  -Output 'build/surface/source/gsi-dem10b/14'
```

対象のXYZ座標が決まっている場合は、`x,y` をヘッダーとするCSVを `-TilesCsv` で指定する。全国版では離島を含む対象座標の一覧を使うと、範囲外の海域へのリクエストを抑えられる。取得前に対象だけ確認するには `-PlanOnly` を付ける。出力されたCSVは次の実行の `-TilesCsv` に使える。

```powershell
pwsh -NoProfile -File surface/scripts/download_gsi_dem10b.ps1 `
  -TilesCsv 'build/surface/source/dem10b-tiles.csv' `
  -Output 'build/surface/source/gsi-dem10b/14'
```

地表生成の `-Dem14` には、このスクリプトの `-Output` と同じディレクトリを指定する。保存したPNGを基に地表の収録範囲を決める。

### ESA WorldCover 2021 v200

生成コマンドから `-WorldCoverDirectory` を省くと、Rustの生成器が作業用標高SQLiteを読み、余白を含む範囲のタイルを選ぶ。PowerShellの取得スクリプトが、対応するGeoTIFFをESAの公開バケットから取得する。保存先は作業ディレクトリ内の `worldcover/`。取得ファイルと配信元に存在しない候補は `japan-download-manifest.json` に記録する。[ESA WorldCoverの配布ページ](https://esa-worldcover.org/en/data-access)

原本を再利用するために先に集める場合は、次のコマンドを実行する。作業用標高SQLiteは入力PNGの範囲を変えたときに作り直す。出力先には新規ファイルを指定する。

```powershell
cargo run --locked --release --manifest-path surface/Cargo.toml --bin surface-esa-worldcover -- `
  prepare-dem build/surface/source/gsi-dem10b/14 build/surface/source/dem.sqlite3

pwsh -NoProfile -File surface/esa_worldcover/scripts/download_esa_worldcover.ps1 `
  -TerrainDatabase build/surface/source/dem.sqlite3 `
  -Output build/surface/source/worldcover
```

対象のタイル名だけ確認する場合は取得スクリプトに `-PlanOnly` を付ける。事前取得した原本を使うときは、生成コマンドに `-WorldCoverDirectory 'build/surface/source/worldcover'` を追加する。対象範囲の `ESA_WorldCover_10m_2021_v200_*_Map.tif` を同じディレクトリに置く。

## ローカル生成

リポジトリのルートで実行する。`-WorkDirectory` には新規ディレクトリを指定する。

```powershell
pwsh -NoProfile -File surface/esa_worldcover/scripts/build_esa_worldcover.ps1 `
  -Dem14 'build/surface/source/gsi-dem10b/14' `
  -WorkDirectory 'build/surface/japan-20261110T032217Z'
```

実行スクリプトは、DEM10B PNGから作業用標高SQLiteを作り、GeoTIFFのモザイクを作り、標高SQLiteから作業区画を決める。次に、森林・水域の分類と穴埋め、LOD別の縮小、陰影生成、タイルのパック、PNG／WebP圧縮、共通SQLiteへの格納と検査を順に行う。森林と水域は4ビット相当、陰影は北西からの単方向で明暗の変化に応じて8〜3ビット相当。タイルの一辺はz10〜z12が384画素、z5〜z9が512画素。森林は最も細かいz12からz5まで順に生成し、各レベルは直上のタイルの森林被覆率を面積平均する。z10からz9では384画素の子タイルを親の256画素四方に縮小する。直上のタイルがない区画は元のESAデータから生成した結果を使う。

z5〜z7では、収録タイル全域を森林・水域・陰影の共通範囲として処理する。この範囲に入る日本国外の陸地もESA原本の分類で収録する。標高のない箇所の陰影は中立値。z8〜z12の収録範囲は作業用標高SQLiteから選ぶ。z11とz12では、z9の作業区画でESA原本を従来の2倍の細かさで読み取り、各レベルのタイルに縮小する。陰影はz13とz14のDEMを使う。

ローカル生成時は生成完了時のUTC日時から `yyyyMMddTHHmmssZ` 形式の仮識別子を自動生成する。加工・検査Actionで `yyyyMMddTHHmmssZ-<Run ID>-<試行番号>` 形式の配布識別子に確定し、SQLite・manifest・Releaseの予定タグへ反映する。セマンティックバージョン番号は指定しない。SQLite台帳の `data_version` とmanifestの `version`・`datasets[].dataVersion` はこの識別子を共有する。

完了すると、作業ディレクトリに `esa-worldcover.sqlite3` と `manifest.json` ができる。全国データと作業ファイルはGit管理外の `build/` に置く。配布ファイルは次の手順でそろえる。

## 配布準備

[地表データの共通公開手順](../README.md#地表データの公開)に従い、`-Dataset esa_worldcover` を指定する。`-Package all`（既定）に、森林・陰影・水域のz5〜z12を収録したSQLiteを渡す。

```powershell
pwsh -NoProfile -File surface/scripts/prepare_release.ps1 `
  -Dataset esa_worldcover -Package all `
  -Database 'build/surface/japan-20261110T032217Z/esa-worldcover.sqlite3' `
  -OutputDirectory 'build/surface/esa-worldcover-release-20261110T032217Z' `
  -TargetCommit '<生成に使ったコミットの40桁のSHA>' -ProcessDraft
```

### 生成済みSQLiteの配布識別子を更新する

画像を再生成せず、配布識別子だけを更新する場合はRustツールで新しいSQLiteに複製する。入力ファイルと圧縮済み画像BLOBは変更しない。出力には未使用のパスを指定する。

```powershell
cargo run --locked --release --manifest-path surface/Cargo.toml --bin surface-tool -- `
  set-data-version '入力.sqlite3' 'build/surface/esa-worldcover-datetime.sqlite3' '20261110T032217Z'
```

その後、上の配布準備コマンドに新しいSQLiteを指定する。元の生成manifestがない場合は `-DataTimestamp` に元データの実際の生成日時を指定する。配布識別子を更新したSQLiteはハッシュも変わるため、manifest・ライセンス・分割版は再作成する。

### 最大レベル別の配布

各ファイルに森林・陰影・水域の3レイヤーを収録する。確定後の配布識別子 `20261113T010203Z-12345-1` の場合、すべてのファイルを同じ `esa-worldcover-20261113T010203Z-12345-1` Release に添付する。各SQLiteのデータセットIDは `esa-worldcover-japan`。

| 収録レベル | SQLite ファイル | manifest |
|---|---|---|
| z5〜z9 | `esa-worldcover-z9.sqlite3` | `esa-worldcover-z9.manifest.json` |
| z5〜z10 | `esa-worldcover-z10.sqlite3` | `esa-worldcover-z10.manifest.json` |
| z5〜z11 | `esa-worldcover-z11.sqlite3` | `esa-worldcover-z11.manifest.json` |
| z5〜z12 | `esa-worldcover-z12.sqlite3` | `esa-worldcover-z12.manifest.json` |

全レベル版の配布ファイル名は[公開設定](release.json)の `packages.all.fileName` で指定する。公開準備では生成済みSQLiteをその名前でコピーする。加工・検査Actionは全レベル版を検査し、`splitPackages` に従って最大レベル別の3ファイルを抽出する。配布用Draftが整ったら、共通ActionでPre-release、正式版の順に公開する。

## 出典とライセンス

| 用途 | 出典 | 利用条件 |
| --- | --- | --- |
| 森林・水域の分類、穴埋め、縮小による地表マスクの生成 | [ESA WorldCover 10 m 2021 v200](https://doi.org/10.5281/zenodo.7254221) | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
| 地表分類の補助判定と陰影の生成 | [国土地理院の標高タイル](https://maps.gsi.go.jp/development/demtile.html)（DEM10BのPNG） | [国土地理院コンテンツ利用規約](https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html)（公共データ利用規約 第1.0版） |

公開地図に表示するときは、使用したレイヤーの出典と加工内容を示す。ESAの帰属表示は[配布元の案内](https://esa-worldcover.org/en/data-access#citation)に従う。

> © ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021) processed by ESA WorldCover consortium

国土地理院の出典は[地理院タイル一覧](https://maps.gsi.go.jp/development/ichiran.html)を示し、加工したことを明記する。

> 地理院タイル（標高タイル）を加工して作成

出典・帰属表示・利用条件・加工内容は、SQLiteの `datasets.metadata_json` 内の `sources` と、配布manifestの `metadata.sources` に記録する。利用・再配布時は出典ごとの利用条件を確認する。

## 検査

共通 SQLite のテストでは、地点と共用する GeoJSON・言語の検証例も確認する。`surface-tool` のテストでは ESA 梱包時の対象範囲・言語の引き継ぎと、レイヤー抽出時の範囲の扱いを確かめる。

GitHubに接続せず、受け渡し用Draftへの整理マーカー付与、配布版の加工と再実行、試用公開・正式公開を確認するには、生成済みSQLiteを指定する。

CIはRustの `release_fixture` で小さなSQLiteを作り、同じ公開フローを検査する。ローカルでも次のコマンドで実行できる。

```powershell
New-Item -ItemType Directory -Force build/surface/release-test
cargo run --locked --release --manifest-path surface/Cargo.toml -p yamamuki-data-sqlite --example release_fixture -- build/surface/release-test/source.sqlite3
pwsh -NoProfile -File surface/tests/publish_release.ps1 -Database build/surface/release-test/source.sqlite3 -DataTimestamp '2026-11-11T00:00:00Z'
```

全国データで検査するときは、次を実行する。

```powershell
pwsh -NoProfile -File surface/tests/publish_release.ps1 -Database build/surface/japan-20261110T032217Z/esa-worldcover.sqlite3
```

```powershell
cargo test --locked --manifest-path surface/Cargo.toml -p surface-esa-worldcover
cargo run --locked --release --manifest-path surface/Cargo.toml --bin surface-tool -- `
  validate build/surface/japan-20261110T032217Z/esa-worldcover.sqlite3
```

配布manifestの作成やSQLiteの統合は [地表データの共通手順](../README.md#検査と統合)を参照する。

## 全国データの収録内容

`esa-worldcover-japan` の収録設定例。描画順は台帳から読む。表のRGBA値はファイルに収録される参考値で、アプリは既定色として利用できる。

| レイヤーID | 描画順 | 設定 |
| --- | ---: | --- |
| `forest` | 0 | 4ビット相当。色 `[111,148,88,255]`、背景 `[224,216,196,255]` |
| `hillshade` | 10 | 中立値136、強度0.65。森林の色に適用し、水域の色には適用しない |
| `water` | 20 | 4ビット相当。色 `[114,172,204,255]`。濃度を透明度に使用 |

| レベル（実XYZズーム） | タイル寸法 |
| --- | ---: |
| z12 | 384 |
| z11 | 384 |
| z10 | 384 |
| z9 | 512 |
| z8 | 512 |
| z7 | 512 |
| z6 | 512 |
| z5 | 512 |

| レイヤーID | 圧縮 | 欠損行の扱い |
| --- | --- | --- |
| forest | 可逆WebPまたはPNG | 地表の色を描画しない |
| water | 可逆WebPまたはPNG | 水域を描画しない |
| hillshade | WebPまたはPNG | 中立値136を使う |

この表は全国データの照合例。タイルごとの画像形式は `surface_tiles.encoding` を使う。

- `10/906/404` は384×384。森林・水域・陰影のWebPが各1枚。
- `5/26/13` は512×512。森林・水域・陰影が各1枚。
- 収録された既定色を使う場合、森林濃度0、陰影136、水域濃度0の有効画素は背景色 `[224,216,196,255]`。水域濃度255の画素は `[114,172,204,255]`。森林タイルの行がなく、水域濃度0なら出力は透明。

森林・水域・陰影を別のレベルにしても位置が一致することと、タイル行のない箇所が背景へ抜けることを確認する。

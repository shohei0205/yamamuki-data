# 国土地理院DEMの地形データ

[地形データに戻る](../README.md)

国土地理院の標高PNG（DEM10B、z14、256×256画素）を使う。PNGの画素は符号付きcm標高であり、画像の見た目の色ではない。元DEMの無効画素は誤差評価から除外し、無効画素が一部あるだけでタイル全体を欠損扱いにはしない。

## 生成設定

| XYZズーム | 下向き許容誤差m | 上向き許容誤差m | 独立アンカー |
|---|---:|---:|---|
| z6 | 500 | 125 | ○ |
| z7 | 400 | 100 | |
| z8 | 300 | 75 | |
| z9 | 250 | 62.5 | ○ |
| z10 | 200 | 50 | |
| z11 | 125 | 31.25 | |
| z12 | 60 | 15 | ○ |
| z13 | 25 | 6.25 | |

[settings.json](settings.json) が実際の設定。LPFなしで最詳細レベルを元DEMの有効全画素へ適合する。その後は細レベルの点集合を引き継いで不要点を削除する。局所変更の影響範囲だけを再評価し、最後に保存済みデータから曲面を復元して元DEMと全画素照合する。上向き誤差を小さくし、山体が不自然に広がることを抑える。

点のXYは1/16m、Zは1/8m単位の整数。Rice・FOR・ゼロ連続を128整数ごとに選択する。外側zlibは使わない。親のXYを正確に継承し、Zの差分を保存する。差分は最大2段、直接親は最大4タイル。

## 環境

生成器はRust stable、Windows x64、MSVC Build Toolsを使う。常駐メモリ8GiB・private bytes 10GiBと空きディスク100GiBを実測で制限する。資源制限の計測はWindows実装であり、現時点の全国生成はWindowsで行う。ネイティブビューワーはWindowsとmacOS（Apple Silicon）に対応する。

QGIS、GDAL、Pythonは地形生成に不要。取得補助とRelease操作にPowerShell 7、Release操作にGitHub CLIを使う。

## 元データの取得

地理院標高タイルのDEM10B配信元は `https://cyberjapandata.gsi.go.jp/xyz/dem_png/14/{x}/{y}.png`。既存のキャッシュがあれば再利用する。標準の元データ置場は `D:/Work/EarthData/GSI`。実際の `14/x/y.png` の14ディレクトリを指定する。

地表と共通の [DEM10B取得スクリプト](../../surface/scripts/download_gsi_dem10b.ps1) を使う。全国対象の座標は `x,y` ヘッダー付きCSVで管理し、同じ取得CSVと取得結果CSVを保管する。配信のない404と通信失敗を区別し、失敗が0であることを確認する。無制限の並列取得は行わない。

```powershell
./surface/scripts/download_gsi_dem10b.ps1 -Output D:/Work/EarthData/GSI/DEM10B/14 -TilesCsv D:/Work/EarthData/GSI/DEM10B-tiles.csv
# 範囲を指定して三地点の試験入力を取得する例
./surface/scripts/download_gsi_dem10b.ps1 -Output D:/Work/EarthData/GSI/DEM10B/14 -West 138.6 -South 35.2 -East 138.9 -North 35.5
```

全国であることはキャッシュのファイル数だけから判定できない。生成器は指定されたPNG全件を対象とするため、全国の対象CSV・取得結果と照合してから準備する。対象範囲外のPNGも同じキャッシュに混ぜると生成対象に入る。

## 準備と三地点試験

すべてリポジトリのルートで実行する。出力先は新規ディレクトリにする。

```powershell
cargo build --locked --release --manifest-path terrain/Cargo.toml
$engine = './terrain/target/release/terrain-gsi-dem.exe'
$dem = 'D:/Work/EarthData/GSI/DEM10B/14'
& $engine prepare --dem14 $dem --output build/terrain/national
& $engine prepare --dem14 $dem --output build/terrain/fuji
& $engine prepare --dem14 $dem --output build/terrain/yari
& $engine prepare --dem14 $dem --output build/terrain/tsukuba
# --windowはz14の[x最小,y最小,x最大,y最大]、右下を含まない。
& $engine run --prepared build/terrain/fuji --window 14503 6467 14508 6472
& $engine run --prepared build/terrain/yari --window 14454 6412 14459 6417
& $engine run --prepared build/terrain/tsukuba --window 14566 6419 14571 6424
& $engine qualify --prepared build/terrain/national --pilots build/terrain/fuji build/terrain/yari build/terrain/tsukuba
& $engine run --prepared build/terrain/national
```

座標は取得範囲に合わせて指定する。三地点の完成SQLiteに記録された全画素監査・整数往復・支持領域証明と生成器SHAを `qualify` で確認する。同じ範囲の使い回し、別設定・別生成器の試験結果は認めない。実行ファイルを更新した場合は同じ実行ファイルで試験をやり直す。

準備時にPNGを復号しSHA-256を固定する。生成時・再開時にも元ファイルが変更されていないか検査する。進行は作業ディレクトリの `progress.json` に記録する。中止する場合は `stop-requested` という空ファイルを作る。停止後にそれを削除し、同じ `run` コマンドで再開する。既存の完成SQLiteは上書きしない。

```powershell
& $engine verify --prepared build/terrain/national
```

`verify` は保存済み制御点と親参照を復元し、整数ハッシュ、元DEM全画素、同品質の境界を監査する。生成時の誤差保証は元DEM有効画素上の自然近傍曲面が対象であり、C1調整量・画面メッシュ分割・異品質境界の表示変更を含む最終ピクセルの保証ではない。

## 配布SQLiteと公開

完成した作業結果を共通コンテナへ格納する。独立・差分の圧縮本文は維持し、親参照と整数ハッシュをBLOBにまとめる。全タイルのXYZ照合が成功してから出力を確定する。

```powershell
$tool = './terrain/target/release/terrain-tool.exe'
$version = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ')
& $tool import-shared build/terrain/national/terrain-shared.sqlite3 build/terrain/gsi-terrain.sqlite3 $version
& $tool validate build/terrain/gsi-terrain.sqlite3
```

ソースと生成器のコミットを記録してからDraftを用意する。生成日時には実際の生成完了日時をUTCで指定する。

```powershell
./terrain/scripts/prepare_release.ps1 -Dataset gsi_dem -Database build/terrain/gsi-terrain.sqlite3 -OutputDirectory build/terrain/release -DataTimestamp '2026-11-13T00:00:00Z' -TargetCommit (git rev-parse HEAD) -ProcessDraft
```

SQLite・対応manifest・LICENSEを添付し、`地形：下書き Release 加工・検査` を起動する。`-CreateDraft` は添付まで、どちらも省くとローカル検査まで。Actionは `main` に配置後に利用できる。生成はz13まで行い、配布時には最大z12へ選別する。粗レベルを細レベルから削減しているため、生成上限と配布上限は分ける。

| 配布SQLite | 収録ズーム | 対応manifest |
|---|---|---|
| `gsi-terrain-z9.sqlite3` | z6〜z9 | `gsi-terrain-z9.manifest.json` |
| `gsi-terrain-z10.sqlite3` | z6〜z10 | `gsi-terrain-z10.manifest.json` |
| `gsi-terrain-z11.sqlite3` | z6〜z11 | `gsi-terrain-z11.manifest.json` |
| `gsi-terrain-z12.sqlite3` | z6〜z12 | `gsi-terrain-z12.manifest.json` |

4種類は1つのReleaseへ添付し、LICENSEを共通にする。ローカル準備がz12版を作り、加工・検査Actionがz9〜z11版と各manifestを作る。制御点を再近似・再圧縮せず、圧縮BLOBをそのまま選択する。各出力の全XYZ・ハッシュ・親参照を検査し、必要な親レベルが欠ける選択は拒否する。

個別に分割して確認する場合:

```powershell
& $tool select-levels build/terrain/gsi-terrain.sqlite3 build/terrain/gsi-terrain-z9.sqlite3 '[6,7,8,9]'
```


## ビューワー

正式ビューワーの「地形SQLiteを開く」、または以下で3D表示する。

```powershell
cargo run --locked --release --manifest-path viewer/Cargo.toml -- --terrain build/terrain/gsi-terrain.sqlite3
# 地表SQLiteを重ねる場合は2番目のファイルに指定する
cargo run --locked --release --manifest-path viewer/Cargo.toml -- --terrain build/terrain/gsi-terrain.sqlite3 build/surface/esa-worldcover-z12.sqlite3
```

## 出典とライセンス

- 国土地理院の [標高タイル](https://maps.gsi.go.jp/development/demtile.html)（DEM10B）を加工して作成。
- 利用条件は [国土地理院コンテンツ利用規約](https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html)。利用時は「地理院タイル（標高タイル）を加工して作成」と表示し、加工内容を明記する。
- データセットの `metadata.sources` と配布 `LICENSE.txt` にも出典・利用条件・帰属表示・加工内容を収録する。

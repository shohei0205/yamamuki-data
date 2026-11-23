# 国土地理院DEMの地形データ

[地形データに戻る](../README.md)

国土地理院の標高PNG（DEM10B、z14、256×256画素）を使う。PNGの画素は符号付きcm標高であり、画像の見た目の色ではない。元DEMの無効画素は誤差評価から除外し、無効画素が一部あるだけでタイル全体を欠損扱いにはしない。

## 生成設定

| XYZズーム | 下向き許容誤差m | 上向き許容誤差m | 独立アンカー |
|---|---:|---:|---|
| z6 | 312.5 | 312.5 | ○ |
| z7 | 250 | 250 |  |
| z8 | 187.5 | 187.5 |  |
| z9 | 156.25 | 156.25 | ○ |
| z10 | 125 | 125 |  |
| z11 | 78.125 | 78.125 |  |
| z12 | 37.5 | 37.5 | ○ |
| z13 | 15.625 | 15.625 |  |

[settings.json](settings.json) が実際の設定。LPFなしで最詳細レベルを元DEMの有効全画素へ適合する。その後は細レベルの点集合を引き継いで不要点を削除する。全レベルで、証明済みの離れた領域ごとに候補をまとめて削減する。影響がその領域を超える候補は一候補ずつの証明へ戻す。局所変更の影響範囲だけを再評価し、最後に保存済みデータから曲面を復元して元DEMと全画素照合する。上下の許容誤差は同じ値とし、従来の上向き・下向き許容量の算術平均に揃える。非対称な誤差評価によって粗レベルが低い側へ偏り、精細化で一方向に持ち上がる傾向を抑える。粗細の高さの大小関係は制限しない。自然近傍で再近似するため、上下対称にしてもレベル切替時の形状差や標高の平均差がゼロになる保証はない。

設定プロファイルは `sibson-shared-no-lpf-z6-z13-symmetric-v2`。旧設定の作業結果・完成SQLiteは新設定として再開しない。新しい出力先へ準備・生成・全画素監査を行い、既存の配布SQLiteの設定値だけを書き換えない。

全国生成の対象はz6〜z13とし、z4・z5は生成・配布対象に含めない。

点のXYは1/16m、Zは1/8m単位の整数。Rice・FOR・ゼロ連続を128整数ごとに選択する。外側zlibは使わない。親のXYを正確に継承し、Zの差分を保存する。差分は最大2段、直接親は最大4タイル。

## 環境

生成器はRust stable、Windows x64、MSVC Build Toolsを使う。常駐メモリ8GiB・private bytes 10GiBと空きディスク100GiBを実測で制限する。資源制限の計測はWindows実装であり、現時点の全国生成はWindowsで行う。ネイティブビューワーはWindowsとmacOS（Apple Silicon）に対応する。

元DEMの復号キャッシュは既定4,096枚（約2GiB）。地域生成で入力全件を保持したい場合、`TERRAIN_DEM_CACHE_TILES` を128〜8,192枚の範囲で指定できる。キャッシュの設定にかかわらず、プロセス全体の実測資源制限は維持する。まとめ削減で扱う元DEM範囲は1作業あたり最大196枚に制限し、粗いWebタイルの全域を一度に復号しない。切り出した証明範囲を超える変更は個別評価へ戻す。

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

`verify` は保存済み制御点と親参照を復元し、整数ハッシュ、元DEM全画素、同品質の境界を監査する。生成時の誤差保証は元DEM有効画素上の自然近傍曲面が対象であり、C1調整量・画面メッシュ分割・異品質境界の表示変更を含む最終ピクセルの保証ではない。後述の `fit-slopes`・`prune-slopes` は、これとは別に局所係数付き曲面を倍率1で監査する。

## 配布SQLiteと公開

完成した作業結果を共通コンテナへ格納する。独立・差分の圧縮本文は維持し、親参照と整数ハッシュをBLOBにまとめる。全タイルのXYZ照合が成功してから出力を確定する。

```powershell
$tool = './terrain/target/release/terrain-tool.exe'
$version = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ')
& $tool import-shared build/terrain/national/terrain-shared.sqlite3 build/terrain/gsi-terrain.sqlite3 $version
& $tool validate build/terrain/gsi-terrain.sqlite3
```

### 元DEMに合わせた局所傾き調整

制御点ごとに0〜1の傾き調整係数を持たせる場合は、取り込み後に次を実行する。`--prepared` はそのSQLiteの生成に使った準備ディレクトリを指定する。固定した元PNGのSHA-256と生成世代が一致しない入力は拒否する。

```powershell
& $engine fit-slopes --prepared build/terrain/national --database build/terrain/gsi-terrain.sqlite3 --output build/terrain/gsi-terrain-local-slope.sqlite3 --data-version $version
& $tool validate build/terrain/gsi-terrain-local-slope.sqlite3
```

制御点の削減結果とXYZを維持し、元DEMの全有効画素から8bit係数を求めてNPA1本文へ保存する。同じレベルの共有点では値を統一し、CPU・GPUの両方で係数も自然近傍補間する。誤差保証は調整倍率1だけを対象とし、補正前の曲面の誤差は制約しない。ビューワーもこのデータは倍率1で開き、表示設定のリセット時も1へ戻す。ビューワーの傾き調整は局所係数に掛ける全体倍率になる。追加の削減には次の `prune-slopes` を使う。入力SQLiteは変更せず、XYZ・ハッシュ・元DEM誤差の照合後に新規出力を確定する。公開する場合は以降の手順のDatabaseにこの出力を指定する。

`--window x0 y0 x1 y1` でz14の半開区間へ学習を絞れる。外形全体が窓内に入る地形タイルだけを学習し、窓外のタイルに影響する共有点は係数0にする。出力には未学習タイルも残るため全国版の代わりに公開しない。`metadata.generation.localSlopeWindowZ14` と `scope=window-test` を記録する。再学習済みのNPA1入力の再学習には対応しない。

槍ヶ岳周辺のz14窓 `[14456,6412,14460,6416]` の5タイル・2,097,152画素で、全体倍率1のRMSEは9.8013mから9.3228mになった。上下許容誤差を満たし、整数XYZは全タイルで一致した。全国への適用・容量比較は別途行う。

### 局所係数を使った制御点の追加削減

```powershell
& $engine prune-slopes --prepared build/terrain/national --database build/terrain/gsi-terrain-local-slope.sqlite3 --output build/terrain/gsi-terrain-local-slope-pruned.sqlite3 --data-version $version
& $tool validate build/terrain/gsi-terrain-local-slope-pruned.sqlite3
```

最終曲面を倍率1で評価して制御点を追加削減する。残った点の係数は維持し、点の削除後に補間の重みと勾配を再計算する。補正前の倍率0では誤差を超える候補も、倍率1で許容値を満たせば認める。共有点、その隣接点、および他のタイルから親参照されるタイルは維持する。タイルの外側へ影響が及ぶ削除と、最小角度10度を下回る削除は行わない。

各候補の変更領域、削減後の全画素、保存点から再構築した全画素を順に監査し、最後にすべてのタイルのXYZ・SHA-256・親参照を確認する。検査失敗時は完成ファイルを残さない。`--window` で対象タイルを絞り、`--candidate-limit N` で1タイルあたりの試行数を制限できる。未指定では対象の全候補を試す。削減数などは `metadata.generation.localSlopePruning` に記録する。

槍ヶ岳周辺の上記窓に入るz13の4タイルでは、3,635候補から175点を追加削減した。1,048,576画素の最大誤差は15.624583mで、許容値15.625m以内だった。親タイル・共有点を維持し、保存後の全XYZ照合も成功した。この試験は地域限定であり、全国版はまだ生成していない。

### 係数を保存しない自動計算方式の地域比較

```powershell
& $engine compare-automatic-slopes --prepared build/terrain/national --database build/terrain/gsi-terrain.sqlite3 --stored-database build/terrain/gsi-terrain-local-slope-pruned.sqlite3 --output build/terrain/automatic-slope-comparison --data-version $version --z 13 --window 14456 6412 14460 6416
```

`--database` は補正前の完成SQLite、`--stored-database` は同じ元DEMから作った係数保存方式のSQLite。対象領域が全体として窓内に収まるタイルを比較する。元DEMのSHA-256と生成世代を確認し、自動係数の最終曲面を倍率1で監査する。許容誤差を超える場所には、XY=1/16m・標高=1/8mへ量子化したDEM制御点を追加する。`--point-limit` は共通点集合の点数上限で、既定131072点。収束しない場合は完成ディレクトリを作らない。

この入口は地域比較用。全国の基本生成器を置き換えない。基準版・保存係数版・自動係数版を、同じ領域・独立圧縮・XYZ辞書順・32mの周辺領域と完全な一周近傍という条件へ揃える。自動係数版は点の追加後に、係数を再計算しながら追加削減する。削減後もNPA1を使わず、係数を配布物へ記録しない。

出力は `baseline.sqlite3`（倍率0の基準面）、`stored.sqlite3`、`automatic.sqlite3`、`automatic-pruned.sqlite3` と、全画素誤差・容量を含む `comparison.json`。対象レベルだけを収録し、親タイルを含めた容量差を混ぜない。生成途中は一時ディレクトリを使い、全検査が成功してから出力ディレクトリを確定する。各SQLiteをネイティブビューワーで開いて比較できる。

槍ヶ岳周辺z13・4タイル・1,048,576有効画素での比較結果：

| 方式 | 制御点数（共有分を含む） | タイル本文 | SQLite全体 | RMSE | 最大誤差 |
|---|---:|---:|---:|---:|---:|
| 基準面（倍率0） | 8,657 | 36,060 bytes | 50,176 bytes | 5.4235m | 15.6246m |
| 保存係数＋削減 | 8,482 | 41,523 bytes | 57,344 bytes | 5.1898m | 15.6246m |
| 自動係数＋削減 | 9,463 | 38,816 bytes | 53,760 bytes | 5.0846m | 15.6249m |

自動係数へ単純に切り替えた場合は17,668画素が誤差超過し、最大31.1541mだった。共通点集合へ1,056点を追加して適合させ、出力タイルから329点を削減した。点順序・支持領域も揃えた比較では、タイル本文は保存係数版から約6.5%減ったが、基準面より約7.6%大きい。全国の容量へは外挿しない。

自動係数の式は[形式仕様](../formats/sqlite.md#制御点からの自動傾き係数)に記載する。係数は読み込み時に計算し、ネイティブビューワーでは既存のGPU制御点領域へ転送して再利用する。毎フレーム近傍を調べ直す処理は行わない。

### 係数1固定と突起抑制の地域比較

上の比較コマンドに `--fixed` を付けると、点別係数なしで滑らかなH1曲面を使う。出力は `fixed.sqlite3`、`fixed-pruned.sqlite3` と基準面・保存係数版・比較JSONになる。係数1で元DEMの誤差を監査し、さらに周囲8画素に対する突出量が元DEMより0.5m超増える箇所を補点する。固定1の曲面で生成した後も、山頂・窪地を保持し、突起を再発させる削除を拒否する。全国生成の置き換えではなく地域試験用である。

実データの独立した形状監査は次で行う。環境変数には生成したファイルと準備ディレクトリの絶対パスを指定する。

```powershell
$env:YAMAMUKI_FIXED_SHAPE_DB = (Resolve-Path build/terrain/fixed-slope-comparison/fixed-pruned.sqlite3).Path
$env:YAMAMUKI_FIXED_SHAPE_PREPARED = (Resolve-Path build/terrain/national).Path
cargo test --release --locked --manifest-path terrain/Cargo.toml -p terrain-gsi-dem real_fixed_surface_has_no_new_sharp_peaks -- --ignored --nocapture
```

試験では山頂へ近づく方向ごとの傾きを確認し、元DEMにある本来のピークは突出量の比較対象として残す。判定は元DEM格子上であり、あらゆる表示倍率での外観を保証するものではない。

### 山の形・稜線を読むための斜面の平滑化

`--fixed --smooth-slopes` は、細かな鉛直起伏を省いたDEMを生成前の基準にする。元PNGを書き換えず、主要な山頂・窪地・稜線・谷筋を保護する。画面だけをぼかす処理ではなく、平滑化後の標高へ制御点の高さと配置を適合し直す。

```powershell
& $engine compare-automatic-slopes --fixed --smooth-slopes --prepared build/terrain/national --database build/terrain/gsi-terrain.sqlite3 --stored-database build/terrain/gsi-terrain-local-slope-pruned.sqlite3 --output build/terrain/smoothed-slope-comparison --data-version $version --z 13 --window 14456 6412 14460 6416
```

既定はσ=4 DEM画素（槍ヶ岳付近で約30m）、標高変更の上限15m。二つのガウス平滑化を組み合わせて広い斜面の曲率による標高のずれを相殺し、短い周期の凹凸を落とす。複数スケールの勾配・曲率から主要な稜線・谷筋を保護し、半径12画素で有意な山頂・窪地は元DEMの標高を維持する。原DEMの3×3標高範囲にも制限し、フィルターが新しい極値を作るのを抑える。

周囲1枚分のDEMタイルも読み、SHA-256を検査してから処理する。欠損は埋めない。対象域と周辺域を共通に処理するため、タイルごとの独立平滑化による継ぎ目を作らない。元DEM、平面・二次曲面、欠損、重複領域の一致、特徴点の保存をテストする。平滑化後は1cm単位へ揃えて再現性を確認する。

誤差保証は**平滑化後のDEMに対する倍率1**で評価する。原DEMとの差は別途 `comparison.json.rawDemComparisonAfterPruning` に記録する。`sourceSmoothing` に設定・原本の識別子・対象窓・処理後標高のSHA-256・変更量・最高標高を保存する。追加削減では細かな局所極値をすべて保護せず、特徴点マスクで選んだ主要地形を保護する。突起監査も平滑化後のDEMを基準に継続する。ビューワーは従来と同じ固定1の曲面を描画でき、元DEMや平滑化処理を必要としない。

槍ヶ岳z13・4タイルで、既存の固定1版と比較した結果：

| 項目 | 平滑化前 | 平滑化後 |
|---|---:|---:|
| 制御点数（共有分を含む） | 10,910 | 7,923 |
| タイル本文 | 44,125 bytes | 33,781 bytes |
| SQLite全体 | 58,880 bytes | 49,664 bytes |
| 斜面の曲面粗さ（同じ画素・保護域を除外した4近傍ラプラシアンRMS） | 1.5577m | 0.8683m |
| 元DEM格子上の描画曲面の最高標高 | 3180.75m | 3180.75m |

平滑化後DEMに対する最大誤差は15.5991m、原DEMに対する最大差は30.1541m。斜面の曲面粗さは約44%、SQLiteは約15.7%減った。これは地域限定の比較であり、全国版への適用・容量の外挿は行っていない。

ソースと生成器のコミットを記録してからDraftを用意する。生成日時には実際の生成完了日時をUTCで指定する。

```powershell
./terrain/scripts/prepare_release.ps1 -Dataset gsi_dem -Database build/terrain/gsi-terrain.sqlite3 -OutputDirectory build/terrain/release -DataTimestamp '2026-11-13T00:00:00Z' -TargetCommit (git rev-parse HEAD) -ProcessDraft
```

SQLite・対応manifest・LICENSEを添付し、`地形：下書き Release 加工・検査` を起動する。`-CreateDraft` は添付まで、どちらも省くとローカル検査まで。Actionは `main` に配置後に利用できる。生成はz13まで行い、z13を含む全レベル版と、上限をz9〜z12にした版を配布する。

| 配布SQLite | 収録ズーム | 対応manifest |
|---|---|---|
| `gsi-terrain-z9.sqlite3` | z6〜z9 | `gsi-terrain-z9.manifest.json` |
| `gsi-terrain-z10.sqlite3` | z6〜z10 | `gsi-terrain-z10.manifest.json` |
| `gsi-terrain-z11.sqlite3` | z6〜z11 | `gsi-terrain-z11.manifest.json` |
| `gsi-terrain-z12.sqlite3` | z6〜z12 | `gsi-terrain-z12.manifest.json` |
| `gsi-terrain-z13.sqlite3` | z6〜z13 | `gsi-terrain-z13.manifest.json` |

5種類は1つのReleaseへ添付し、LICENSEを共通にする。ローカル準備がz13版を作り、加工・検査Actionがz9〜z12版と各manifestを作る。制御点を再近似・再圧縮せず、圧縮BLOBをそのまま選択する。各出力の全XYZ・ハッシュ・親参照を検査し、必要な親レベルが欠ける選択は拒否する。

個別に分割して確認する場合:

```powershell
& $tool select-levels build/terrain/gsi-terrain.sqlite3 build/terrain/gsi-terrain-z9.sqlite3 '[6,7,8,9]'
```


## 制御点削減の実行と計測

削減候補は、互いに影響しない領域を複数レーンへ分けて並列評価する。休止した領域の接続復元も領域ごとに並列実行し、同一領域内では削除履歴の順序を維持する。各レーンの不足DEMを一括して読み込み、同じ画素配列を共有する。削除だけでは無効にならない外接円の証明を全レベルで再利用し、点の追加・復元・レベル変更では無効化する。

`TERRAIN_PRUNE_THREADS` でスレッド数、`TERRAIN_PRUNE_LANE_FACTOR` でスレッド数に対する作業レーン数の倍率を指定できる。SQLiteへの確定保存と干渉する領域の調整は直列で行うため、増やせば必ず速くなるわけではない。既定の局所バッチ時間は250ms。証明キャッシュの既定上限は200万件で、`TERRAIN_CIRCLE_CERTIFICATE_LIMIT` で調整する。常駐メモリ等の上限検査は別に維持する。

`progress.json` の `profile` に準備・評価・保存・同期の時間を出す。`restore_seconds` は準備時間に含まれる領域キャッシュ復元の内訳。`parallel_cache_restore` と `coalesced_dem_preparation` は実行方式を示す。

削減だけの性能比較には、作業DBを独立した試験用フォルダーへコピーして使う。

```powershell
& $engine benchmark-prune --prepared build/terrain/prune-benchmark --z 11
```

このコマンドは `meta` に `benchmark_only=true` と `benchmark_prune_only=true` の両方がある試験DBだけを受け付ける。通常の全国生成DBは拒否する。試験コピーの `benchmark_candidate_limit` で候補数を制限できる。結果は `benchmark-prune.json` に保存し、配布可能な生成完了とは扱わない。`TERRAIN_PRUNE_BENCH_AUDIT=1` で変更の影響を受けたDEMの全画素を独立に再検査する。

比較時だけ `TERRAIN_SERIAL_PRUNE_CACHE_RESTORE=1` で復元を直列へ、`TERRAIN_SERIAL_PRUNE_DEM_PREPARE=1` でDEM準備をレーン別へ戻せる。同一キュー・同一誤差・同一スレッド数で比較する。実際の生成へ適用する前に、同じ生成器で三地点の全レベル生成と全画素監査を通す。

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

### 全国用の平滑化基準の準備

地域試験と同じ `ridge-preserving-gaussian-v2`（σ4画素、最大変化15m）を、z14の8×8タイルと周囲1タイルの窓で分割処理する。元PNG・既存の全国SQLiteは変更しない。

```powershell
& $engine prepare-smoothed-slopes --prepared build/terrain/national-symmetric-v2/run --output build/terrain/national-smoothed-fixed-v1/smoothed-dem
```

`smoothed-dem.work.sqlite3` は生成用キャッシュであり、配布用地形SQLiteではない。センチメートル単位の標高と主要地形保護マスクをZstandardで保存し、窓単位のトランザクションで確定する。同じコマンドで再開できるが、元DEM世代・原本一覧ハッシュ・設定・実行バイナリが一致する必要がある。再開時は完了済み本文のSHA-256も照合する。`progress.json` の `complete` は平滑化基準の準備完了を表し、地形の適合・削減・全レベル監査の完了ではない。

### 平滑化DEM・固定係数1で全国生成する

上の全国前処理を完了してから、固定係数1専用の新しい作業領域を作る。従来版の作業SQLite・監査キャッシュは流用しない。完成済みの制御点SQLiteはXY配置の初期値として読み取り、標高を平滑化済みDEMへ合わせ直す。元の誤差保証を引き継ぐ操作ではなく、全画素を再評価する。

```powershell
& $engine prepare-fixed-terrain `
  --prepared build/terrain/national-symmetric-v2/run `
  --smoothed build/terrain/national-smoothed-fixed-v1/smoothed-dem/smoothed-dem.work.sqlite3 `
  --seed-database build/terrain/codec-optimized/gsi-terrain.sqlite3 `
  --output build/terrain/national-smoothed-fixed-v1/run
```

`--prepared` と `--seed-database` は同じ原DEM世代である必要がある。上記はローカル配置例であり、それぞれ手元の検証済み原本一覧・完成済み制御点SQLiteのパスへ置き換える。平滑化キャッシュの原本一覧・設定・件数を確認し、原DEMのPNGも本実行時にSHA-256で再照合する。

同じ `prepare-fixed-terrain` で別の三つの試験ディレクトリを作り、各地域を `run --window ...` で生成する。全レベルの生成・誤差・形状・接合・整数復号が成功した三地域を `qualify` に渡してから、窓を指定せず全国実行する。

```powershell
& $engine qualify --prepared build/terrain/national-smoothed-fixed-v1/run --pilots $yari $fuji $tsukuba
& $engine run --prepared build/terrain/national-smoothed-fixed-v1/run
& $engine verify --prepared build/terrain/national-smoothed-fixed-v1/run
& $cli import-shared build/terrain/national-smoothed-fixed-v1/run/terrain-shared.sqlite3 build/terrain/national-smoothed-fixed-v1/gsi-terrain.sqlite3 $version
& $cli validate build/terrain/national-smoothed-fixed-v1/gsi-terrain.sqlite3
```

生成対象はz6〜z13。各レベルの上下許容誤差は従来の表と同じで、基準は平滑化後DEM、評価倍率は1とする。8bitの点別係数は保存しない。勾配を復元する `local-hermite-v1` は適合・削減・保存後の全画素監査で共通に使う。点の追加・削除では隣接点の勾配も変わるため、自然近傍の変更域に加えて隣接点の支持域を再監査する。従来のSibson面にだけ成立する区間保証は固定曲面の受理判定に使わない。

保護マスク内の主要な山頂・窪地は、半径12画素・突出量3mを基準に制御点へ追加し、各レベルで削減から保護する。点数を固定上限に収めるために保護点を省かない。各レベルでは元の細レベルに含まれる点だけを残す。周囲8画素への突出量が平滑化DEMより0.5m超増える箇所は補点し、角度・誤差も再監査する。粗レベルの補修には一段細かいレベルの点を使う。配布用整数XYZの復元後も同じ曲面で誤差・余分な突起・接合を検査する。

`run` は窓単位の前処理とは別のチェックポイントを持つ。同じ設定・原本・実行バイナリで同じコマンドを実行すると再開する。`stop-requested` を作ると未確定処理を取り消して停止でき、再開前にそのファイルを取り除く。最終検査を通過するまでは `terrain-shared.partial.sqlite3` を完成版として扱わない。`progress.json` の `complete=true`・`distributable=true` は地形の全段階が完了した場合だけ記録される。

生成器の局所点数と配布タイルの点数には、131,072点・262,144点という固定上限を設けない。`--local-point-limit`・地域試験の `--point-limit` は診断時に明示する任意の上限で、既定は無制限。形式のu32表現、バイト数・整数演算・メモリの確保可否、実測の資源制限は検査する。再利用領域やキャッシュの大きさを調整しても、補間に必要な支持点を切り捨てない。

### 全国生成の進捗HTML

生成を動かしたまま、別のターミナルで次を実行する。Python 3の標準ライブラリだけを使い、生成用SQLiteは読み取り専用で参照する。

```powershell
python terrain/gsi_dem/scripts/progress_html.py --job-root build/terrain/national-smoothed-fixed-v1 --prepared run-unlimited
```

作業ディレクトリの `progress.html` をブラウザーで開く。10秒間隔で、レベル別の処理段階、現在の工程の処理率・候補数・削減点数・速度・残り時間目安・最大メモリを更新する。`--once` で一回だけ出力できる。残り時間は現在の工程だけの目安で、全国生成全体の完了予測ではない。`job-status.json` はローカル実行管理スクリプト、`progress.json` と作業SQLiteは生成器が記録する。

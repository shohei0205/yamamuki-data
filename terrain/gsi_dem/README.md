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

[settings.json](settings.json) が実際の設定。初期点配置を作る基準生成では最詳細レベルを元DEMの有効全画素へ適合する。配布用の生成には後述の「平滑化DEM・固定係数1で全国生成する」を使う。その後は細レベルの点集合を引き継いで不要点を削除する。全レベルで、証明済みの離れた領域ごとに候補をまとめて削減する。影響がその領域を超える候補は一候補ずつの証明へ戻す。局所変更の影響範囲だけを再評価し、最後に保存済みデータから曲面を復元して元DEMと全画素照合する。上下の許容誤差は同じ値とし、従来の上向き・下向き許容量の算術平均に揃える。非対称な誤差評価によって粗レベルが低い側へ偏り、精細化で一方向に持ち上がる傾向を抑える。粗細の高さの大小関係は制限しない。自然近傍で再近似するため、上下対称にしてもレベル切替時の形状差や標高の平均差がゼロになる保証はない。

設定プロファイルは `sibson-shared-no-lpf-z6-z13-symmetric-v2`。旧設定の作業結果・完成SQLiteは新設定として再開しない。新しい出力先へ準備・生成・全画素監査を行い、既存の配布SQLiteの設定値だけを書き換えない。

全国生成・配布の対象はz6〜z13。

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

`verify` は保存済み制御点と親参照を復元し、整数ハッシュ、元DEM全画素、同品質の境界を監査する。生成時の誤差保証は元DEM有効画素上の自然近傍曲面が対象であり、画面メッシュ分割・異品質境界の表示変更を含む最終ピクセルの保証ではない。

## 配布SQLiteと公開

完成した作業結果を共通コンテナへ格納する。独立・差分の圧縮本文は維持し、親参照と整数ハッシュをBLOBにまとめる。全タイルのXYZ照合が成功してから出力を確定する。

```powershell
$tool = './terrain/target/release/terrain-tool.exe'
$version = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ')
& $tool import-shared build/terrain/national/terrain-shared.sqlite3 build/terrain/gsi-terrain.sqlite3 $version
& $tool validate build/terrain/gsi-terrain.sqlite3
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

`ridge-preserving-gaussian-v2`（σ4画素、最大変化15m）を、z14の8×8タイルと周囲1タイルの窓で分割処理する。元PNG・既存の全国SQLiteは変更しない。

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

生成対象はz6〜z13。各レベルの上下許容誤差は上の表のとおりで、基準は平滑化後DEM、評価倍率は1とする。8bitの点別係数は保存しない。勾配を復元する `local-hermite-v1` は適合・削減・保存後の全画素監査で共通に使う。点の追加・削除では隣接点の勾配も変わるため、自然近傍の変更域に加えて隣接点の支持域を再監査する。従来のSibson面にだけ成立する区間保証は固定曲面の受理判定に使わない。

保護マスク内の主要な山頂・窪地は、半径12画素・突出量3mを基準に制御点へ追加し、各レベルで削減から保護する。点数を固定上限に収めるために保護点を省かない。各レベルでは元の細レベルに含まれる点だけを残す。周囲8画素への突出量が平滑化DEMより0.5m超増える箇所は補点し、角度・誤差も再監査する。粗レベルの補修には一段細かいレベルの点を使う。配布用整数XYZの復元後も同じ曲面で誤差・余分な突起・接合を検査する。

`run` は窓単位の前処理とは別のチェックポイントを持つ。同じ設定・原本・実行バイナリで同じコマンドを実行すると再開する。`stop-requested` を作ると未確定処理を取り消して停止でき、再開前にそのファイルを取り除く。最終検査を通過するまでは `terrain-shared.partial.sqlite3` を完成版として扱わない。`progress.json` の `complete=true`・`distributable=true` は地形の全段階が完了した場合だけ記録される。

生成器の局所点数と配布タイルの点数には、131,072点・262,144点という固定上限を設けない。`--local-point-limit` は診断時に明示する任意の上限で、既定は無制限。形式のu32表現、バイト数・整数演算・メモリの確保可否、実測の資源制限は検査する。再利用領域やキャッシュの大きさを調整しても、補間に必要な支持点を切り捨てない。

### 全国生成の進捗HTML

生成を動かしたまま、別のターミナルで次を実行する。Python 3の標準ライブラリだけを使い、生成用SQLiteは読み取り専用で参照する。

```powershell
python terrain/gsi_dem/scripts/progress_html.py --job-root build/terrain/national-smoothed-fixed-v1 --prepared run-unlimited
```

作業ディレクトリの `progress.html` をブラウザーで開く。10秒間隔で、レベル別の処理段階、現在の工程の処理率・候補数・削減点数・速度・残り時間目安・最大メモリを更新する。`--once` で一回だけ出力できる。残り時間は現在の工程だけの目安で、全国生成全体の完了予測ではない。`job-status.json` はローカル実行管理スクリプト、`progress.json` と作業SQLiteは生成器が記録する。

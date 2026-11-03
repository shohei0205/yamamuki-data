# 全国共有曲面生成の性能改善

更新: 2026-11-02。目標は全国の全工程を10時間以内。現時点では**総時間の性能合格は未確認**。一工程の倍率や小範囲の時間を、全国全工程の完了見積もりとして扱わない。

## 現在の状態

本番 `build/national-shared-z6-z13-v1/work.sqlite3` は停止要求を受けて安全に停止した。保存済み点は保持している。比較はSQLiteバックアップ `build/national-profile-snapshot.sqlite3` と別の性能試験用DBで行う。性能試験用DBに `meta.benchmark_only` がある場合、通常の全国Runは拒否する。

許容誤差、上方向E/4、最小角10°、細品質のXYZを粗品質が継承する条件、Rice保存形式は緩和していない。全PNG原本のSHA256照合と最終の全有効画素監査も残している。コミット・pushはこの最適化依頼では行わない。

## 実測の記録

| 試験 | 条件 | 実測 | 解釈 |
|---|---|---:|---|
| 同じ16点の挿入 | 全国停止時の点集合を複製。毎回SQL空円照合 | 24.81秒 | 局所更新部分のみ |
| 同じ16点の挿入 | 証明済み領域の接続を再利用 | 0.042秒 | 確認した曲面サンプルの差0。総生成時間ではない |
| 全国10帯の160入力 | 初期点を全国に置く前、10並列 | 390.34秒、0.410入力/秒、119提案を再計算 | 未生成地域の巨大空円が干渉する |
| 同じ160入力 | 全PNGの最高点を先に配置、10並列 | 25.30秒、6.323入力/秒、再計算0 | 最詳細の初回適合だけなら約4.81時間の単純換算。再巡回・削減・角度補修・保存検証は別 |
| 全国初期点 | 全109,506入力をSHA256照合・復号 | 65.66秒 | 各PNGの実在最高点を一つ。不要なら後で削減 |
| 槍ヶ岳の隣接16入力 | 初期改善版・全8品質 | 367.71秒 | 最新の全国並列化前の小範囲試験 |
| 同じ16入力 | 動的接続更新版・全8品質 | 161.06秒 | 838万画素の保存後監査、119境界サンプルが成功。全国への単純外挿はしない |

試験結果は `build/national-shared-parallel-bench-v2/benchmark-fit.json`、`build/national-shared-parallel-bench-v3/benchmark-fit.json`、同 `timings.json`。全品質の性能試験は `benchmark-levels.json` に記録する。この試験では対象入力の外側を固定した文脈として使うため、全国本番と同じ点集合の縮退ではない。

## 改善した処理

### 全国の初期点と原本検査

各入力PNGを復号して実在する最高標高の画素を一つ選ぶ。同高なら最初の画素。全て欠損なら点を置かない。XYは元画素の中心、Zは元DEM値を従来と同じ整数に量子化する。規則格子の制御点を固定する方式ではなく、初期点も通常の削減対象。これにより、未生成地域が遠い原点番兵とだけ接続して巨大な空円になる状態を避ける。

CPU論理プロセッサー数の半分のRayonプールで128入力ずつ復号・照合し、点をまとめて保存する。`seed-global-cursor` は保存後に進める。途中停止で同じ点を再試行しても重複登録しない。初期点の完了状態は `seed-global-v1`。

### 空間索引と支持集合

DBが正本。活動中の一品質について、同じ世代の点をメモリRTreeに保持する。世代が一致しなければSQL照合へ戻す。上限は3,200万点。常駐8GiB／private10GiBの実測停止条件は維持する。次の品質の索引を作る前に旧索引を解放し、同品質・同世代なら再構築しない。

局所領域の初期取得に近傍64点までを加え、仮の巨大空円を早く狭める。メモリ照合の反例は最大512点、SQL照合は従来通り64点ずつ。上限で支持点を打ち切るのではなく、空円証明が完成するまで反復する。局所点数上限を超えたら停止する。

### 動的なDelaunay接続

`natural_incremental.rs` に実装。追加は堅牢なincircle判定による挿入空洞だけを除き、新頂点への面を張る。削除は頂点の周囲の多角形を耳切りで三角形化し、内辺を堅牢な辺交換でDelaunayへ戻す。面配列の末尾との交換で空きを詰め、外側半辺・接続面番号を更新する。

変更した面、半辺、頂点近傍、Voronoiセルのキャッシュだけを記録する。棄却時は完全に戻す。点配列には空きが残ってよいが、活動しない点は面に含めず、局所IDは0として区別する。DBの点レコードは削除しない。原点番兵と凸包の制約点は通常の局所削減では残す。

初回は全国点集合へ空円を照合済みの接続を使う。証明領域内の追加・削除は、その既知の空洞／頂点の周囲だけを変更する。範囲外で影響する点が読み込まれている場合は接続を再取得する。全域復元との一致、連続更新、共円格子、棄却復元をテストする。

面探索は近い面のヒントから開始する。長い番兵面で通常の差積が桁落ちする場合はrobust::orient2dで歩き直し、必要なら全候補面を堅牢な符号で検査する。補間方式を別方式へ置き換えるフォールバックではない。

### 最詳細の並列適合

入力を緯度方向の離れた帯に分け、同じスナップショットから提案を作る。提案はPNG内で従来の最大方向別誤差順に追加する。先行確定した挿入の旧外接円が次の提案の証明領域に触れた場合だけ、その提案を現在の全国点集合から作り直す。

確定した一提案を一つのFULL同期トランザクションで保存する。頂点一つずつの同期保存を避ける。全入力を再巡回し、追加0の巡回と最後の全画素監査を満たしてから完了する。小範囲の試験では帯を分けず順次適合する。

### 頂点削減の並列化

`natural_shared_prune.rs`。候補を空間順に均等な帯へ配分し、`prune_queue(z,stripe,sequence,id)` に未処理状態を保存する。各候補の旧外接円群が互いに交差しない場合だけ同時評価する。証明範囲外の元画素が必要なら、従来のDB試行削除による逐次検証へ戻す。

自点と、過去の試行で違反した元DEM画素を先に検査する。反例は実在する画素の座標と元の標高で、再利用する際も現時点の曲面を評価する。古い補間値を信用しない。採用点はまとめて活動集合から外し、正本の点レコードは残す。処理済みキューだけを消す。保存直後に停止してキューが残っても、既に活動していない点を検出して再開できる。

### 厳密な上界と最終監査

Sibson座標の非負性・一次精度を使い、支持点と全対象元画素の平面からの残差範囲を比較する。安全余裕を加えた上界が許容誤差以下の場合だけ、試行中の個々の補間を省略する。上界で判定できない場合は全対象画素を従来通り計算する。

最終の誤差監査と配布DBの独立Surface評価は全有効画素を計算する。保存後監査は行帯を並列にし、復号済み曲面は合計50万点を目安に再利用する。欠損を0mで埋めない。

### 圧縮保存

品質別の有効画素数を先に一度集計する。各出力タイルで全国の監査表を再走査しない。点参照表の挿入SQLも再利用する。配布形式・親参照上限・アンカー・量子化は変更しない。

## 検証と再開

現在47件のRustテストが成功。さらに最終release版で三地点・複数入力・全品質の実地形監査、旧保存系の回帰、全国構成の性能試験を行う。性能試験完了後も、10時間目標の総見積もりが成立するかを別に確認する。

`qualify_national_shared.ps1 -PilotSuffix optimized-v5` のように試験出力を指定できる。再開前に現在のexeのSHA256を検証結果へ固定する。`stop-requested` を残したまま二重起動しない。本番の元データ・保存済み点・旧生成物は保持する。

## 2026-11-02 追加最適化

削減試験の再開後約20.6秒の計測では、準備20.42秒、並列誤差評価0.041秒、保存0.102秒、キャッシュ同期0.049秒。準備中に周辺メッシュを317回再構築しており、誤差評価の並列数を増やすだけでは改善しない。

各処理帯に現在の領域に加え最大16個、合計131,072支持点を目安とする領域キャッシュを追加した。空間順の走査がタイル列を往復しても証明済み領域を再利用する。採用した削除を全保持領域へ同期し、同一の全域三角形と証明できない領域は従来どおり破棄する。古い世代の領域は再利用しない。誤差式・最小角・最終全画素監査は変更しない。高速化倍率は同じ候補列での測定後に判断する。全国全工程10時間以内は未確認で、本番は保存済み停止状態を保持する。

### 角度補修の循環対策

全国分散160入力の角度補修で、補修候補11点がすべて過去の削除対象となり停止した。削除前後の最小角が同じでも削除を認めていたため、挿入前の鋭角三角形へ戻りうる。削除は最小角が厳密に改善する場合だけに限定した。以前に削除した候補の再挿入を許し、復帰した補修候補は角度補修内の削除対象から除外する。最小角10度、量子化、誤差の最終監査は維持する。44件のRustテストは成功。全国本番は停止を保持し、別DBで再検証中。

最新版（v8）実地形試験は富士山7.95秒、槍ヶ岳8.33秒、筑波山5.51秒で各1入力・全8品質を完了。配布DBを復号した独立Surfaceによる全有効画素監査を含む。全国構成160入力の試験も候補枯渇箇所を通過して全画素監査へ進んだ。小範囲の秒数を全国へ直接外挿しない。

## 2026-11-02 候補準備の追加改良（標準経路v13）

全国分散160入力の試験はz13からz6までの削減・角度認証を完了した。これは全国完成品の生成・保存後監査を完了したという意味ではない。z12の従来の削減843.05秒、角度認証89.60秒が次の比較基準。

現行の標準経路は以下。全国本番は保存済み停止状態を保持する。

- 読み取り専用SQLiteスナップショットを領域ごとに開き、周辺メッシュの構築・支持証明を並列化する。すべて同一世代で開始・終了したことを確認し、ワーカー終了前に正本を変更しない。
- 全域の空間インデックスはArc共有。領域の初期点検索も、同一世代のメモリー上のRTreeを使う。SQLによる空間検索はメモリー索引がない場合の経路として残す。
- ワーカー終了後、確定前に共有参照を解放し、点の変更時に全域RTreeが不要に複製されないようにする。
- 削除の影響を囲む矩形だけでなく、旧三角形の外接円群と照合し、影響しないDEM領域を除外する。従来と同じ2画素の余裕を加える。最終全画素監査は省略しない。
- キャッシュ範囲外を含む試行の標準経路は、従来のSAVEPOINT・試行削除・全域支持再証明・ロールバックを維持する。

### 比較用の仮想削除（既定では非採用）

候補のIDを初期点検索・近傍初期点・全域外接円照会の全経路から除外する読み取り専用ビューを実装した。実際のトランザクション削除と補間結果が一致することを非平面データで検証した。しかし広域の近傍を持つ候補では再構築コストが増え、遅延実行を単純に逐次化すると性能が悪化した。現時点では標準経路には採用しない。`TERRAIN_PARALLEL_REMOVAL_PROOFS` が設定された場合だけ比較試験に使う。領域を4件ずつ並列検証し、最初の反例の区切りで残りを打ち切る。誤差式や量子化を変更する方式ではない。

47件のRustテストに成功。主な追加試験は、変更前後の並列支持証明と逐次証明の一致、仮想削除と実DB削除の一致、削除点の旧外接円群の外では非平面のSibson曲面が不変であること。実DEMの槍ヶ岳・隣接4入力・全8品質はv11で40.61秒、209万7152有効画素の保存後直接評価まで成功。標準経路v13は別DBの同じ開始状態・候補列の先頭512件で旧v8と順番に比較中。全国全工程10時間以内は未確認。

### 標準経路v13の実測

全域の開始点集合と候補列を同じにし、CPUを競合させず旧v8・標準v13を順番に実行した。先頭512件を目安に安全停止し、実際の確定件数をキューから数えた。

|方式|実行秒|処理候補|採用削除|候補/秒|
|---|---:|---:|---:|---:|
|旧v8|74.734|519|433|6.9446|
|標準v13|25.375|530|443|20.8867|

候補/秒は約3.01倍。先頭部分だけの実測で、全レベルや全国全工程の倍率ではない。共通して処理した515候補の採否は全件一致し、親品質のXYZも一致した。結果は `build/shared-prune-prefix-comparison-v13.json` と `build/shared-prune-prefix-decision-comparison-v13.json`。比較用DBは保持している。

標準v13は `build/national-shared-prune12-standard-v13` で保存済み候補から全レベル試験を再開した。全国本番は停止を維持する。全国10時間以内の見通しは、粗レベル全体と保存・監査を含む実測で別途判断する。

### 独立した領域キャッシュの同期並列化（v14）

実DEM16入力ではキャッシュ同期が削減時間の大半を占める区間も見つかった。DBの世代を一度読み、現在領域・保持領域を同じRayonプールで並列に同期する。各ワーカーは自分のPatchだけを変更し、正本DB・全域RTreeを変更しない。削除確定が0件の区切りでは同期を行わない。支持の同一性判定・不一致時のキャッシュ破棄は従来どおり。

47件のRustテストは成功。v13の槍ヶ岳16入力・全8品質は保存後8,388,608有効画素の直接評価、119境界サンプルの確認まで完了した。境界の最大標高差1.37e-12m、最大勾配差1.96e-15。これはv14で再生成・監査中。v13試験は別の性能試験とCPUを共有しており、285.48秒という総実行時間を単独実行の旧試験と直接比較しない。

v14の全レベル性能試験は `build/national-shared-prune12-standard-v13/resume-v14.launch.json`、実DEM16入力試験は `build/national-shared-yari-optimized-16tiles-v14`。本番データ・旧試験・ログは保持。今回の作業ではcommit/pushを行わない。

v14の実DEM16入力・全8品質も165.55秒で完了し、保存後8,388,608画素・119境界サンプルの監査に成功。全14配布タイルについて、v13とv14のXYZハッシュ・圧縮BLOB・親参照深度などの保存内容がバイト単位で一致した（`build/shared-cache-sync-comparison-v14.json`）。全国性能試験と同時実行した秒数なので、単独環境での高速化倍率とは区別する。

## 2026-11-02 レベル連動キャッシュと支持閉包の増分更新（v17）

粗レベルでも元PNGの約2.45km単位の狭い範囲をキャッシュしていた。現在は作業中のWebメルカトルレベルの一辺を基準にし、各辺に25%の余裕を持つ。配布タイルの仕様を変えるものではない。密集地は周辺余白も含めた実際の点数で範囲を縮小し、局所支持点予算の1/4を初期領域の目安とする。支持証明が参照する点を打ち切らず、領域外は従来の全域照会・試行監査へ戻す。

さらに、支持閉包の反例が見つかるたびに既知の点をすべて三角形分割し直す処理を廃止した。最初だけ認証済みDelaunayを構築し、その後はID順の反例点を局所挿入する。各反復の全域空円検証は従来通り行い、反例が0になるまで完了としない。凸包外・厳密な共線などで局所挿入できない場合だけ、同じ全XYZによる従来の再構築へ戻す。点の精度・誤差・最小角・追加順の優先度式を緩める処理ではない。

### 実測と検証

- 同一の開始点集合・候補列の先頭約520件で、前版v14は19.57候補/秒、レベル連動範囲だけのv15は26.57候補/秒。共通511候補の採否は全件一致。
- 増分支持証明を加えたv16は519候補を16.547秒、31.37候補/秒で処理。先頭部分だけの実測であり、全国全工程の倍率ではない。
- 49件のRustテストに成功。追加した試験は、密集地で縮めた領域と全点曲面の一致、全国と同じ大座標・遠い番兵点を使った局所支持証明と全点三角形分割の一致。並列テスト用の一時パスには時刻に加えて原子カウンターを使い、Windowsの時計粒度による衝突を防ぐ。

### 全粗レベルの比較手順

`build/national-shared-full-coarse-incremental-v16` と `build/national-shared-full-coarse-baseline-v15` は同じ最詳細z13の完成点集合から作った試験用コピー。z12～z6のactive・epochs・stages・queues・audit・pagesをコピー内で初期化し、すべて新規生成する。全国本番・元の試験DBは変更しない。最詳細z13の再利用は結果に明示する。各粗レベル終了時に親品質の頂点包含関係も監査する。

現在の実行順は、最新版で実DEM槍ヶ岳16入力・全8品質の配布後監査、最新版で160入力の全粗レベル生成、前版v14で同じ全粗レベル生成。CPUを競合させず順番に実行する。記録は `build/shared-full-coarse-comparison-v17.json`、各試験の `full.launch.json`、`full.stdout.log`、`full.stderr.log`。prefix測定分もtimingsのaccumulated_secondsに含まれるため、再開後の秒数だけを全生成時間と取り違えない。

全国本番は保存済み停止状態。全工程10時間以内はまだ確認していない。今回もcommit・pushは行わない。

### 全粗レベルの新規生成結果

全国分散160入力のz12～z6をすべて新規生成し、各レベルの全有効画素誤差・最小角・親子の頂点包含を確認した。最詳細z13は全方式で同じ完成点集合を再利用しており、この秒数にz13の生成・最終配布パックは含まれない。prefix測定・途中再開分も累積値へ含めた。

|レベル|頂点削減秒|角度・誤差認証秒|
|---|---:|---:|
|z12|285.51|96.31|
|z11|105.19|35.22|
|z10|41.63|18.18|
|z9|37.58|18.85|
|z8|38.17|16.81|
|z7|74.01|16.05|
|z6|82.06|12.13|

工程の累積合計は877.69秒（約14分40秒）。全粗レベルで親に存在しない点は0。メモリのピークは作業セット約451MB、private約486MB。前版v14の同条件比較は `build/national-shared-full-coarse-baseline-v15/full.launch.json` で実行中。全体の倍率は比較完了後に確定する。

### 追加の標準経路改良

- audit_allは同じレベル・点集合世代の保存済み最大上方誤差・下方誤差・有効画素数を再利用する。元PNGをDem.readへ渡す検証手順は維持し、点集合の更新時は必ず再計測する。保存後の独立Surfaceによる全画素監査はこの再利用とは別で、従来通り実施する。
- 点の局所挿入では、直前に追加した近傍の面を次の探索開始位置にする。毎回0番の面から探索しない。不正・古い開始番号は面数で補正し、従来と同じ幾何判定で点を探す。最詳細適合、キャッシュへの挿入、支持閉包の反例挿入に適用する。
- 50件のRustテストが成功。同世代の監査再利用、点更新後の再計測、元PNG変更の検出、挿入・削除を繰り返しても全点再構築と一致することを確認した。これらの最終2項目は現在のreleaseへ反映済みで、進行中の全体比較用にコピーしたv17とは区別する。

全国本番は引き続き保存済み停止状態。全国全工程10時間以内は未確認。

## 全国生成の再開（2026-11-02 23:41 JST）

ユーザーの全国生成開始指示により、最新releaseで三地点の1入力・全8品質を新規生成し、最終バイナリでのverifyと50件のRustテストを実施した。qualify_national_shared.ps1 -PilotSuffix start-v19でready.jsonを更新。SHA256は77342f4ecffd28b9fab6a6c4c52af0ef268296af56c2db5da5eb62a3ae740f99。比較試験を保存して停止し、全国チェックポイント9220点を同一XYZ・IDで保持したことを確認後、start_national_shared.ps1 -Mode Runで再開した。起動PID5648。各種情報は全国作業フォルダのlaunch.json、resume-v19-check.json、checkpoint-before-v19-resume.sqlite3。

進捗サーバーもポート61494で復旧（PID31684）。HTTP200と全国のseed-global進捗の更新を確認した。全国全工程10時間以内の所要時間は未確認であり、この再開を完了や10時間達成と扱わない。元データ・旧生成物・保存済み点は維持し、commit/pushは行っていない。


## 2026-11-03 v20: finest fit preparation and durable resume

- Stopped at a committed batch boundary; preserved 17,204,967 points in work.sqlite3 and checkpoint-before-v20-resume.sqlite3.
- Imported a conservative cursor from observed acknowledged progress: 72,890 / 109,506 inputs. Later unreported committed batches may replay safely; original XYZ and IDs are retained.
- Parallelized support-certified patch preparation. Large tentative circumcircles first seek a nearest counterexample; absence still requires the complete global empty-circle certificate. No support or error tolerance was relaxed.
- Bulk rebuild the spatial index after 25% growth, dropping the old index before rebuilding. Added stop checks during support scans and persistent cursors after committed batches.
- On 40 unprocessed real inputs, fit core improved from 25.234 s to 13.645 s (preparation 21.267 s to 9.732 s). All 10,534 added XYZ and insertion order matched exactly. Startup/copy/index time is excluded from these core timings.
- A previously mostly committed 10-input replay improved from 4.088 s to 0.452 s, adding only eight points; this is not representative nationwide throughput.
- 52 Rust tests passed. Fresh Fuji, Yari and Tsukuba single-PNG pilots passed all eight output levels and decoded full-valid-pixel verification. Existing 128/512 km resource qualification also passed.
- Restarted hidden at 2026-11-03 07:55 JST, PID 20828, engine SHA256 a33ee16139e3b747aa0090e546a5e41ad2ce23fb842f9a10c909d5f6b6aba340.
- The nationwide ten-hour target remains unconfirmed. Later passes, coarse generation and final verification remain required. No commit or push performed.


## 2026-11-03 v22: fuse support preparation and fitting

- Kept national generation running during exploratory comparisons, then paused safely at acknowledged input 87,580 and epoch 20,704,498 for uncontended measurements. The persisted cursor matched the canonical point epoch; no bootstrap was needed.
- Fused support-certified patch preparation and fitting into each immutable-snapshot Rayon job. Ordered collection and the existing conflict checks preserve commit order. Workers no longer wait at an intermediate all-patches-ready barrier.
- Cached only globally empty circumcircles identified by their immutable three point indices within one fixed-epoch closure. Changed triangles still receive the full support certificate. Pending but not yet inserted counterexamples prevent certification. Also used membership-only hash lookup and amortized stop-file checks every 128 faces (long scans still check every 65,536 candidates).
- Proof reuse alone did not show a stable isolated gain under concurrent national load; do not attribute the combined result to that change alone.
- Uncontended same-input 40-source-PNG comparison: original production v20 = 13.8875 s; v22 fused = 12.9916 s, about 6.5% less core time / 1.069x throughput. The same new version with separate stages took 14.4853 s. Startup, DB copying and national index loading are excluded. These are small samples, not a nationwide ETA.
- All 10,534 added XYZ and insertion order matched across the three outputs. Evidence: build/national-fused-shape-v22.json and benchmark-fit.json in the corresponding three benchmark directories.
- 52 Rust tests passed. Three fresh single-PNG pilots passed all eight levels and decoded full-valid-pixel checks; existing 128/512 km qualification remained valid.
- Fused timing currently records preparation plus proposal together under preparation_seconds; proposal_seconds is zero in fused benchmark mode.
- Restarted hidden: PID 23388, UTC 2026-11-03T00:33:30.8062420Z, SHA256 6DEF45D84549B2FD2AE47A406D07238FE26428FAA17B740960504F6A7BE3BFD8. Resumes input 87,580, preserving canonical points and cursor.
- Ten-hour nationwide completion remains unconfirmed. No commit, push or PR.


## 2026-11-03 v24: radial support search and directional error certificates

- Safely paused pass 2 at input 890, epoch 25,970,271; saved checkpoint-before-v23-resume.sqlite3 and exact durable resume-check-v23.json. Original points and previous outputs were preserved.
- Found broad rectangle queries around huge circumcircles scanning large irrelevant corner regions. Cached support now uses the R-tree distance query with radius padding four times the former envelope padding, intersects the former envelope and keeps the same robust incircle predicate and full closure certificate. Nearest probes remain 16 by default; expanding probes alone was insufficient.
- Split affine interval certificates into upward and downward bounds. Use max(4*up, down), matching the existing asymmetry; previously the bound multiplied both directions by four and needlessly evaluated valid regions. The fitting error target and final independent decoded all-pixel audit are unchanged.
- At the actual second-pass snapshot (25,970,271 points), the same 40 inputs at batch offset 89 took 198.190 s with rectangle/probe16, 138.258 s with rectangle/probe4096, and 0.9543 s with radial search plus directional bounds. These core times exclude DB copying, source initialization and index loading. Comparisons ran separately, but some preparation/build work overlapped the older baseline; do not treat the ratio as a whole-nation guarantee. All 83 added XYZ and their order matched exactly (national-second-shape-v24.json). Optional per-input profile separates support preparation from fitting.
- Insertion-heavy first-pass regression: the same 40 inputs at offset 7296 took 4.4484 s versus v22 12.9916 s. All 10,534 added XYZ and order matched (national-radial-insertion-shape-v24.json).
- 54 Rust tests passed, including radial versus rectangular robust candidate equality for remote guards / nearly collinear quantized points and directional interval tests. Three freshly generated single-PNG, eight-level pilots passed decoded all-valid-pixel verification. Existing 128/512 km resource qualification passed.
- The earlier 56-hour extrapolation used startup/rebalancing-inclusive averages and was too coarse; a revised full remaining-time estimate needs production measurements. No ten-hour completion guarantee.
- Restarted hidden at 2026-11-03T04:46:46.2018803Z UTC, PID 30236, SHA256 9913A390994ABBB804E7079CACC9674FB9FB86B9D0164C78345CE09F8CC3F1E6; resumes pass 2 at input 890. No commit/push/PR.


## 2026-11-03 v29: locality and bounded parallel removal proofs

- Production z13 prune was spending about 92% of time in preparation. Safely paused with 25,864,318 remaining candidates and preserved checkpoint-before-v25-resume.sqlite3, old launch/progress and all prior generated data. Existing accepted removals are retained.
- Added a rejection-only preflight in a small globally certified region before constructing a missing full working patch (z13 only). A failed own-DEM sample can reject a candidate; passing never authorizes deletion. Existing complete affected-region validation still decides acceptance. The small virtual-deletion field was tested against fresh global certification.
- Restoring and retaining tiny preflight caches was slower (26.02 s versus 22.76 s on 2,000 candidates), so that experiment was not adopted.
- Reorder remaining candidates within each existing stripe by Web Mercator tile, then y,x and old sequence. This prevents repeated country-wide row traversal and improves geometry-cache locality. A single SQLite transaction preserves the old queue under prune_queue_before_locality_z{z}, creates the reordered queue, checks counts and saves prune-locality-{z}-v1. Interrupted sorting rolls back; successful sorting is not repeated. Canonical points, stripe ownership, tolerances and already processed candidates are unchanged. Future levels use the same locality step.
- Changed candidate order can change which equivalent points remain. For the same 10,000 candidate IDs, baseline and spatial variants each removed 1,868 points; the symmetric difference was six IDs (three substitutions). Do not claim bit-identical output across the order change. Both use the existing error checks, and final angle repair plus independent decoded all-valid-pixel audits remain mandatory.
- With the improved radial support search, z13 parallel virtual-removal proofs are now faster than serial global SAVEPOINT fallbacks. Enabled these at z13; coarse qualities retain the previous default unless explicitly tested. Global R-tree references are dropped before commits.
- Full lazy PNG loading was slightly slower (33.58 s versus 31.29 s), so the final version preloads up to 64 regions per candidate from the shared cache and only loads additional regions in lazy parallel groups of four. No PNG checksum or DEM error validation is skipped.
- Same selected 10,000 real candidates: previous order/prepare 119.0868 s versus final 31.1331 s (3.83x throughput). Preparation 110.6541 -> 13.3305 s; each accepted 1868 removals. Core timings exclude DB copying, input validation and index loading. The limited benchmark does not perform the final nationwide audit and is marked non-distributable.
- 56 Rust tests passed, including queue membership/epoch preservation/idempotent restart. Three freshly generated single-PNG pilots passed all eight output levels and decoded valid-pixel audits. Existing 128/512 km resource qualification passed. No nationwide ten-hour guarantee follows from this sample.
- Restarted hidden: PID 25492, UTC 2026-11-03T08:48:33.5641689Z, SHA256 D00B27731119910BA93BED63C39AC5783EA74ECED6369B8AAC0B29B2CCF8FF2C. Remaining production queue will be localized by the Rust engine before continuing. No commit/push/PR.


### v30: resume directly at pruning

- A restart during pruning was unnecessarily calling finest fitting again because pruning updates the epoch. The accidental rescan was stopped; verified that epoch 26,234,476 and all 25,864,318 pending candidates were unchanged, with 43,157 previous removals preserved.
- Only the initial execute path skips redundant fitting when a completed fit stage and an already committed prune queue (or completed prune stage) exist. Explicit re-fitting after a certify audit failure is unchanged. Source identity validation and all final audits remain mandatory.
- 57 tests passed, including unfitted-input / prune-phase restart checks. Three freshly generated pilots passed all eight output levels and decoded valid-pixel audits. The pruning implementation and measured 119.0868 -> 31.1331 s benchmark from v29 are unchanged.
- Restarted PID 15576 at 2026-11-03T08:56:19.9909055Z UTC; engine SHA256 BF6EE5776A0E4739104BA540E5F16AE14B69A01B7DC6808F1EE53D6459C3E0F4. Rust will localize the remaining queue atomically before pruning.

- Production confirmation: queue localization completed with original 25,864,318 members preserved and old queue table retained. Pruning advanced 121,140 candidates in 148.964 s (813.2/s) after localization; total done 448,137 / 26,191,315. This is an early production sample, not a full-run ETA. Progress HTML HTTP 200.


## 2026-11-03 v31-v47: one-hour pruning target (measurement in progress)

- Safely paused canonical production at epoch 26,628,433, 22,614,828 pending z13 candidates and 437,114 committed removals. `checkpoint-before-v37-resume.sqlite3` and `resume-check-v37.json` retain the restart evidence. Benchmarks clone this database; benchmark outputs are not distribution data. No source/checkpoint/old queue is deleted.
- Target throughput for the remaining candidate phase is at least 6,282 candidates/s (22,614,828 / 3600), excluding initialization and the subsequent nationwide final audit. Small favorable samples do not establish a whole-phase guarantee.
- Implemented lazy ID-to-slot lookup, old-influence-circle masking of changed DEM samples, rejection-only direct deleted-point Voronoi estimates, independent local deletion batches, a 2048-PNG shared decode cache (about 1 GiB), parallel verified PNG reads, and bounded deferred replay of removals into parked patches. Cached global R-tree snapshots are dropped before canonical writes. Local accepted batches require their full influence to be inside the globally certified patch and disjoint from other selected patches.
- Deferred parked synchronization: same middle-stripe 100,000 candidates fell from 20.1620 s to 12.4716 s, each accepting 11,057 removals. This 8,018/s sample is favorable and was NOT accepted as evidence for the entire phase.
- Current-checkpoint 1,000,000 candidates: v37 199.7094 s, v38 fused patch preparation 196.5026 s, each accepting 121,491 removals. Independent fresh-global audits covered 7,979 modified input PNGs / 461,023,631 valid pixels; maximum directional score 24.999974881847947 <= 25. Audit time 180-195 s is separate. These measurements did not meet the one-hour target.
- Shared closed-circle certificates are enabled only during z13 deletion-only pruning. Keys are canonical triples, frame sharing requires exact subtraction conditions, proofs with a skipped known fourth point or a virtual omission are not added, and insertion/reactivation invalidates shared certificates. Maximum 500,000 bounded entries; all uncertainty falls back to the existing robust complete global proof. Cache reuse and generation invalidation tests were added.
- Rejected/default-off experiments: skipping the direct rejection estimate, bypassing small-block affine bounds, and widening the forward patch. v43 combined skip-direct / min64-bound test was 256.3196 s for the same million candidates, with the same 121,491 accepted removals and independent audit maximum. Keep the original direct estimate and bound behavior by default. Forward widening and support padding remain opt-in diagnostics.
- v46 removes mandatory single-candidate rounds between successful independent batches; fallback remains for blocked/noncontained deletions. Small ordered sets in local removal undo collection now use sorted contiguous arrays with the same deterministic order. v47 reuses interpolation work buffers across candidates and input rasters instead of reallocating them every time. 63 Rust tests pass, including combined disjoint deletions versus a fresh global rebuild and safe stop handling.
- No tolerance, upward-error penalty, minimum-angle requirement, final decoded all-valid-pixel audit, or resource guard was relaxed. A final exact-binary qualification and production restart are required after performance/quality selection. No commit/push/PR.


### v48-v51: avoid remote walks and repeated global circle indexes

- The own-sample rejection query now starts its complete Delaunay cavity walk from an incident face of the candidate instead of face zero. Same inclusion tests and robust fallbacks remain.
- Patch support-closure iterations now collect the same ordered support vertices through complete connected rectangle/circle walks instead of building an all-face R-tree after every insertion round. Added exact face-set comparisons between R-tree and adjacency walks for small/large rectangles, distant guards and almost collinear quantized points. Point walks with different hints also return identical face sets.
- v49, actual current-checkpoint million: 171.7502732 s; 121,491 accepted removals. Independent fresh-global audit: 7,979 inputs / 461,023,631 valid pixels; maximum directional error 24.999974881847947. Entire removed-ID set equals v38 (both EXCEPT counts zero; `national-prune-v38-v49-shape.json`). Some build/qualification work overlapped this run; use the final uncontended measurement for ETA.
- v51 raises verified PNG cache from 2048 to 4096 entries (about 2 GiB of f64 pixels). Total process resource guards remain 8 GiB working set / 10 GiB private. No automatic cache increase past 4096. Final exact binary qualified with 63 tests, three freshly generated all-level pilots and existing 128/512 km resource tests. SHA256: 8b6a5cb78fb68a44557f6411fee8b523f6aa3f4bfd89123389e37af61aa5bdc8. Final uncontended million-candidate measurement and independent audit pending at this note's time.


### v52-v55: hot-loop overhead, clip buffers and support batching

- v51 uncontended million: 165.494165 s (6,042.51 candidates/s), projected remaining candidate work 3,742.62 s (62.38 min). Independent audit passed all 461,023,631 valid pixels; removed-ID set equals v38. This did NOT meet the strict one-hour extrapolation and was not automatically restarted as success.
- Hoisted Windows environment-variable lookups out of per-circle loops; point/circle tests no longer call a generic rectangle clamp for a zero-size rectangle. v52 million 169.0453202 s, same accepted count and all-pixel audit; do not claim an isolated speed gain from this noisy comparison.
- Reuse all four direct-deletion Voronoi clipping vectors within local batches. This retains the exact same clipping order/formula and the fallback full proof.
- Added explicit `build.ps1 -NativeCpu` (release-only cargo rustc target-cpu=native, no fast-math). `qualify_national_shared.ps1 -NativeCpu` rebuilds and verifies the exact matching binary. The executable is host-specific; distribution data remains unchanged/portable. Normal builds remain portable by default.
- v55 experimentally raises the maximum support counterexample intake per closure round from 512 to 4096 (diagnostic override `TERRAIN_SUPPORT_BATCH`, clamped 64..4096). The total local point limit remains 131,072 and all robust empty-circle/DEM/angle/decoded checks remain. This aims to reduce repeated support traversal for coastal/empty-area cases requiring many closure rounds. 63 tests pass. Three newly generated all-level pilots and resource qualification pass. Qualified v55 native SHA256 ac87d83336744e71a8899c55bdac7b6cd13137e02bd9739b85ed3c697652c45f. Full performance and independent audit remain pending at this note's time.


### v56: additional scheduling lanes, physical thread count unchanged

- v54 native-only + clip-buffer reuse: million 166.5111776 s, independent audit passed; no decisive native-only gain. v55 support intake4096: million 219.3062825 s, same accepted count and all-pixel audit, so it was REJECTED; default intake is back to512 (4096 remains an opt-in diagnostic).
- Added 20 logical prune lanes on the existing 10-thread Rayon pool. Each old spatially ordered queue is split into two contiguous halves, preserving membership, per-half order, all saved accepted deletions and the active-point epoch. Atomic SQLite migration retains `prune_queue_before_lanes_z13` and records its scheme; restarts reuse the committed layout. Total original candidate count is unchanged. Per-lane parked-cache limits are halved so aggregate parked-cache allowance does not grow. All accepted jobs still require disjoint certified patches and full affected-region checks.
- Additional unit test verifies queue membership in both directions, unchanged active epoch and idempotent restart. 64 Rust tests pass. Fresh fuji/yari/tsukuba all-eight-level pilots and existing resource qualifications pass. Final benchmark and independent audit pending. Qualified native SHA256 bd28318fa7bb1d25e90d3eb4c581cb55b4c6f2075b9dfb4ca12beff39899edef. Physical workers stay10; progress reports `workers:10, lanes:20`.
- Note that the benchmark report's final DEM/cache counters include the optional subsequent audit; core `seconds` excludes that audit. Use in-phase progress samples when attributing candidate-phase PNG cost.


### v56 adopted and production restarted (2026-11-03 20:24 JST)

- Actual current-checkpoint 1,000,000-candidate test: 146.8283471 s / 6,810.67396 candidates/s, 121,450 accepted removals. Remaining 22,614,828 candidates extrapolate to 3,320.4978 s (55.34 min) for candidate work. This is an estimate, not a completion guarantee; initialization, one-time atomic lane migration and the later nationwide final audit are separate costs. The limited benchmark's changed-input audit itself took 145.9374 s.
- Independent fresh-global audit passed 7,979 PNGs / 461,023,631 valid pixels, maximum directional score 24.999974881847947 <= 25. Final shape compared with v38: 120 old-only / 79 new-only removed IDs (41 fewer deletions), reflecting changed inter-lane scheduling, not relaxed tolerance. Do NOT claim bit-identical output for the lane change. All 437,114 already committed production removals remain. Final decoded nationwide DEM/error/angle/seam validation remains mandatory.
- Final exact SHA256 bd28318fa7bb1d25e90d3eb4c581cb55b4c6f2075b9dfb4ca12beff39899edef qualified: 64 tests, three fresh all-eight-level pilots, resource checks. The actual benchmark binary has this same SHA. Production source generation/identity/settings and saved epoch/queue/removal counts were rechecked; no benchmark markers are present in canonical work.sqlite3.
- Restarted hidden with existing launcher at 2026-11-03T11:24:32.2146917Z UTC, PID26264, engine-shared-national-20261103-202431-9578.exe. Same prepared root and saved points. Existing progress HTTP61494 remains200; existing servers were not stopped. No commit/push/PR.
- Evidence: `build/national-prune-million-v56/benchmark-prune.json`, `build/national-prune-v56-adoption.json`, `build/national-prune-v38-v56-shape.json`, `build/qualification-v56.log`. Build/qualify this host-specific engine with `-NativeCpu`; launching the already qualified release requires no rebuild.

- Production confirmation: canonical pruning has advanced to 4,248,441 / 26,191,315; workers10 / lanes20, old pre-lane queue retained with 22,614,828 candidates. Saved points resumed; no duplicate process. Confirmation evidence: build/national-prune-v56-production-confirmed.json. Early post-restart throughput is not a whole-phase guarantee.


## 2026-11-03 v60-v62: angle certification optimization

- Bottleneck after pruning: certify scanned every patch triangle to compute the minimum angle, then scanned it again to choose the worst triangle; it evaluated acos at every corner. Collect only complete rectangle/circle-connected candidate faces, keep face-index order, compare clamped cosines and compute acos once. The same worst face and minimum are obtained in tests, including world-coordinate offsets and very small/large query rectangles; robust full-scan fallback remains if connected search fails.
- Added per-level certify-cursor checkpoint (every32 completed inputs, and at pass end), including input count, pass, next input, changes, minimum and epoch. If later committed changes exist, resumption forces at least one full follow-up pass; it repeats the partial input rather than skipping it. No final all-pixel/angle check is skipped. Invalid input count / future epoch is rejected. Initial v60 transition explicitly bootstrapped the already-observed completed prefix and forced a full follow-up; subsequent v62 restart uses automatic saved cursor101697. Old databases are retained as checkpoint-before-angle-v60.sqlite3 and checkpoint-before-angle-v62.sqlite3.
- After eight consecutive unchanged inputs, prepare up to10 upcoming globally certified patches in parallel on the existing pool. Each patch's epoch is checked before use; stale patches are rebuilt, and mutation clears prefetch. This aims at unchanged regions / follow-up certification passes; it does not parallelize conflicting topology mutations.
- Added virtual angle deletion for local candidates only when the entire influence rectangle plus extra2*STEP query margin fits inside certified bounds. Check minimum-angle improvement and the SAME rectangular full-valid-pixel error test before changing SQLite. Failed tests undo only the local mesh. Accepted deletions atomically update active/epoch/angle_banned, then update the canonical spatial cache. Boundary/wide influence falls back to the previous SAVEPOINT/global-proof path. Unlike prune circle masking, this check deliberately includes the entire old audit rectangle because interim angle insertions may not yet satisfy the global error target.
- 66 tests passed; fresh one-PNG-per-region, all-eight-level A/B pilots passed decoded valid-pixel / minimum-angle / seam checks. Legacy-vs-virtual active XYZ are exactly equal for Fuji, Yari and Tsukuba. Virtual checks were actually exercised (Fuji35, Yari9). Small pilot certify13 seconds legacy->fast: Fuji1.2546->1.0923, Yari2.6584->2.7862, Tsukuba0.9741->1.0339; these tiny cases DO NOT establish nationwide acceleration or a total ETA. v60 geometry-only pilots were also slightly slower on tiny inputs; do not claim a measured large speedup yet.
- Evidence: build/angle-ab-v62.json, build/angle-pilot-comparison-v60.json, build/shared-angle-tests-v62.log, build/qualification-angle-v62.log. Exact final host-specific native SHA256 906feda000f65de287199014243683bcdbad97c73d70a951fa3ec50b4d8aaa85. Qualified with66 tests, three all-level fast pilots, existing resource qualifications.
- Restarted hidden at 2026-11-03T14:50:41.0538903Z UTC (23:50 JST), PID32916, engine-shared-national-20261103-235040-7968.exe. Source/settings/error/upward penalty/minimum10deg were unchanged; no prior points, candidates or servers deleted. No commit/push/PR. Actual production progress and virtual-check usage must be confirmed after initialization.

- Production confirmation: resumed input101697 and advanced to 103185; saved cursor now 103169. Virtual checks since restart: 8877. Peak private bytes 4616863744. Running PID32916 / progress HTTP61494 maintained. This verifies actual fast-path use and resume, not a nationwide speed multiplier. Evidence: build/angle-v62-production-confirmed.json.


## 2026-11-04 v66: parallel whole-pixel audit and connection-page persistence

- The third z13 angle pass reached zero changes and minimum angle10.0000303763deg, but subsequent all-pixel audit found directional error277.3184104587 >25. This is an INTERIM QUALITY FAILURE, not a completed/distributable dataset. Angle geometry changes can require refitting. Error penalties/tolerances remain unchanged.
- Audit preparation, verified PNG decoding and exact all-pixel evaluation now run in bounded batches (up to10 on the existing Rayon pool). Per-input merge order, up/down/max/valid counts and same-epoch cache rules are preserved. Audit rows are committed in one transaction per batch. Source hashes and epoch checks remain. Progress now includes worker count and preparation/evaluation/save/elapsed timings.
- In certification only, a failed audit now returns failure at the first failed input and immediately invokes the existing repair path; it does NOT certify a partial scan. A successful certification still requires all inputs/all valid pixels to pass after repair. Other save=true audits retain full-scan behavior; save=false keeps its existing failure-short-circuit semantics. Batch-computed same-epoch rows may be saved beyond the first failing prefix but are never treated as successful certification.
- After successful audit, preparing and compressing NTP1 connection pages is parallelized; up to10 pages are saved atomically per transaction. The JSON payload, SHA256, NTP1 header and zstd level3 are unchanged. Stale-epoch batches are rejected before writing. These are intermediate pages; final NQC1/NDC1 Rice distribution coding is unchanged.
- 68 tests pass. Added exact serial/parallel audit result and SQLite-row comparison on multiple PNGs, same-epoch cache reuse, failed repair audit remaining uncertified, byte-identical serial/batch page BLOBs and stale-epoch rejection. Fresh one-PNG-per-region, all-eight-level pilots pass verification; Fuji/Yari/Tsukuba active XYZ match the v62 pilots exactly (audit-pilot-comparison-v66.json).
- New non-distributable `benchmark-audit` clones an immutable snapshot, clears only its OWN audit cache for each implementation, and compares the same input subset. Never use these cloned DBs as production output. Actual national snapshot, offset60000/count256: 16,302,643 valid pixels; serial3.8167s -> parallel1.2473004s (3.06x in this sample). Results (including detected failure) are EXACTLY equal; maximum directional error230.8800721794 and within_error=false in both. This is a throughput/consistency test on an interim failing field, NOT a passing national quality audit. Startup/index building/DB copy are excluded; no whole-nation speedup or ETA guarantee.
- Evidence: build/national-audit-comparison-v66/benchmark-audit.json, build/shared-audit-tests-v66.log, build/qualification-audit-v66.log, build/audit-pilot-comparison-v66.json. Original canonical DB preserved as checkpoint-before-audit-v66.sqlite3. No benchmark-only marker leaked into production.
- Exact qualified native SHA256 3efb463cc0153672bebf4f6bd56813810c133bef759a54161aab63de40ff4ed1. Restarted hidden at 2026-11-03T15:39:44.8544365Z UTC (2026-11-04 00:39 JST), PID13032 / engine-shared-national-20261104-003944-6140.exe. Production proceeded directly from failed certification to explicit fit-batch repair: input18,650/109,506, 52,374 added this pass in185.7s at confirmation time. No quality condition was relaxed. Subsequent angle/error verification and final decode/seam checks remain required.
- Progress HTTP61494 remains200. Production confirmation: build/audit-v66-production-confirmed.json. No existing server stopped, no source or checkpoint deleted, no commit/push/PR.

## 2026-11-04 v72: 適合済み入力の証明を保持

全国z13の角度補修後の再適合で、追加がない領域も毎周走査していた。入力ごとの誤差適合済みビットをwork.sqlite3のfit_verifiedへ保存する。原本リストの署名・点世代・形式が一致する場合のみ再利用し、PNGの照合・読込は省かない。挿入前と挿入後のDelaunay外接円に交差する入力だけを無効化する。異常な円は全無効化。保存途中の停止で世代が不一致なら再計算する。

明示的な角度修復後の再適合をcertify-repair-fit-13-v1で記録し、再起動しても適合処理から続ける。証明が空なら並列の全画素監査から適合済み領域を分類する。全領域合格の場合だけ適合完了とする。不合格領域は従来の誤差式・追加順で修復し、最終全画素監査も維持する。

全国スナップショットの320入力、19,840,773有効画素を使用。PNGキャッシュが温まった同一世代・追加点0の再走査本体は再利用なし1.8410503秒、再利用あり0.0160387秒、320入力を再利用。署名作成・入力索引準備・起動時間は含めない。この倍率を全国全工程へ外挿しない。誤差最大24.9997024114、対象範囲監査合格。成果物はbuild/national-fit-cache-comparison-v72/benchmark-fit-cache.json（配布不可・全国完成ではない）。

72テスト成功。挿入の影響範囲外の不変性、世代・署名不一致、局所無効化、修復途中再開を検証。三地点の1入力試験はキャッシュ無効版v71と新版v72で全8レベルの活動XYZが一致（build/fit-cache-pilot-comparison-v72.json）。全国は保存済みfit pass4・20530入力のカーソルを維持して新版へ再開。PID26900、SHA256 53da17f2f031ba01c7cfd24e7a00cf00f8a817ae70fcd8dfd2ccded91cfeb2da。再開後は全画素分類監査が進行中。品質条件は変更せず、全国完成・総10時間以内は未確認。

# 全国共通曲面 Rust生成器

更新: 2026-11-02。全国本番の生成開始前まで実装・検証した。全国完成データを生成済みという意味ではない。

## 実行入口

terrainディレクトリーで使う。

```powershell
# 初回だけ。PNG全件照合と標準XYZ候補を準備する。
./tools/terrain/prepare_national_shared.ps1

# Rustテスト・releaseビルド・三地点試験データの再監査を実行し、ready.jsonを作る。
./tools/terrain/qualify_national_shared.ps1

# 初期化だけ。全国生成は開始しない（既定のMode）。
./tools/terrain/start_national_shared.ps1

# 全国本番の開始／再開。同じコマンドを使う。今回このコマンドは実行していない。
./tools/terrain/start_national_shared.ps1 -Mode Run

# 完成済みデータを読み直す。PNG原本変更・整数復号・全画素誤差・境界も検査する。
./tools/terrain/start_national_shared.ps1 -Mode Verify -Foreground
```

起動用BATは `start_national_shared.bat`。既定は非表示のバックグラウンド起動。コピーしたrelease exe・PID・SHA256・開始時刻・stdout/stderrを出力先のlaunch.jsonへ保存する。全国入力の既定出力は `build/national-shared-z6-z13-v1`。生成器は `build/natural-fixed-lpf-target/release/terrain-natural-shared-national.exe`。

旧LPF全国生成器の起動経路とは別である。`-Build`でexeを更新した場合は、改めてqualifyを実行する。全国Runは入力世代・ready.json・実行中exeのSHA256が一致しなければ起動しない。入力準備のやり直し後もqualifyを実行する。窓付き試験は `-Mode Run -Window x0,y0,x1,y1`。これはz14番号の半開区間であり、範囲外も監査済みと扱わない。

## 生成条件

[NATIONAL_SHARED_PREPARATION.md](NATIONAL_SHARED_PREPARATION.md)と `../../NATIONAL_SHARED_SETTINGS.json` が品質・座標・独立アンカーの定義。z13/25 mからz6/500 mまで全8段、上方向誤差は各E/4。LPFなし、追加20%予算なし。標準WebメルカトルXYZ。

最詳細点集合を元PNGから適合し、不要点を削減して角度を補修する。その後z12→z6を順に生成。粗品質へ新しい独立点を作らず、細品質の点を削除／復帰させる。最後に各粗品質の全点が直前の細品質に存在することをSQLで検査する。

最小角はXY平面のDelaunay三角形で10°。条件未達を黙って許可しない。データは三角形ではなく制御点XYZで配布し、三角形は補間の内部接続である。

## 外部メモリーと同品質の接続

全国点集合をwork.sqlite3のpoints/activeへ保持する。点の空間索引はSQLite RTree。問い合わせの局所点だけを読み、Delaunatorの結果をrobust::orient2d/incircleによる辺交換で合法化する。

局所面の外接円を全国RTreeへ照会し、未読点が円の内部または境界へ入る場合は取り込んで再三角化する。巨大な未確定外接円の全点を一括で読まず、反例を64点ずつ追加して再検証する。自然近傍の支持点と、傾き復元に必要な一周近傍の接続まで確定する。固定幅の余白で探索を打ち切らない。

点集合は全国で一つ。半辺配列の全件常駐の代わりに、空円証明付きの局所接続を再構築・保存する。局所点の上限を超えたら、支持を切り捨てずチェックポイントを残して停止する。既定131072点、指定可能範囲16～262144点。上限の変更は品質を落とす手段ではない。

空間照会はRTreeを先に使うCROSS JOINで、active全点から始める結合計画を避ける。未確定点の検索を不要に全域ソートしない。入力PNGの影響範囲もRTreeで引き、粗品質で復帰させる点は空間範囲を広げて最近傍を確定する。

支持・補間計算では局所原点からの相対f64を使う。保存・共有の正本は全国整数XYZのままで、点を領域ごとに動かさない。元画素中心が量子化点のわずか外側に出て遠い番兵が重みに入る場合は、元DEMの端を一画素だけ延長する支持点を追加する。既存DEM画素があれば実値を使い、欠損を0mで埋めない。角度補修で欠損域へ幾何点が必要な場合は、現在の補間値を継承する。全ての追加後に有効DEM画素を再監査する。

## 誤差適合・削減

最詳細の各PNGを16×16画素のブロックへ分け、各ブロックの最大方向別誤差を保持する。追加の都度、旧Delaunay挿入空洞の外接円が影響するブロックだけ再評価する。方向別スコアはmax(4*(補間-DEM),DEM-補間)。点追加の順序はブロック内・ブロック間で決定的に処理する。終了判定はブロックの古い誤差だけでは行わず、全PNGを再巡回し、最終に全有効画素を監査する。

削減は点の旧外接円による影響領域の元画素を再監査して受理する。一点削除ごとに全国の全画素を走査しない。反例があればトランザクションを戻す。各品質の最終監査は全画素で行う。

最詳細の角度補修は、誤差と角度を悪化させない冗長点削除を先に試し、その後外接円中心等の点を追加する。削除済み候補を再追加して循環しないよう履歴を保持する。粗品質の補修は細品質点の復帰だけにし、最大で元の細品質へ戻るため点数は有限。補修で誤差が増えた場合は再適合・復帰し、再監査する。

画素監査は同じ曲面の不変スナップショットをOS論理CPUの半数で並列評価する。この環境の上限は10スレッド。全国点の更新・削除の受理は一つの書込担当で行い、共有点の正本を複数ワーカーが独立更新しない。

## 作業DBと再開

| テーブル | 内容 |
|---|---|
| meta | 入力世代・対象窓・方式、削減カーソル |
| points | 一意なpoint_id、量子化XYZ、番兵フラグ。UNIQUE(x,y) |
| point_index | XYZのXYを引くRTree |
| active | 各品質が保持するpoint_id |
| epochs | 品質ごとの変更版 |
| pages | 品質・領域・epoch付き接続ページ |
| audit | 品質・PNG・epoch・有効画素数・上下最大誤差 |
| stages | 適合・種のコピー・削減・角度補修の完了記録 |
| removed / angle_banned | 削減記録・角度補修の循環防止 |
| packed_points | 親候補を引くためのpoint_id索引。配布しない |

pagesはNTP1 + little-endian u64の展開長 + 展開JSONのSHA256 32バイト + Zstdレベル3。JSONの点ID・三角形ID配列は作業キャッシュであり配布形式ではない。読取時は点集合から再構築でき、古いページは正本として使わない。Zstdはこの作業キャッシュだけで、配布BLOBに外側圧縮を追加する意味ではない。

削除候補はSAVEPOINT内で反映し、監査成功時だけcommitする。プロセスを落としても未確定削除はSQLiteがrollbackする。コピー・完了記録もトランザクションで保存する。圧縮済みタイルと親のcommit後に停止しても、再開時に同じXYZハッシュから作業索引を再構築できる。

engine-lock.sqlite3のEXCLUSIVEロックで二重実行を拒否。停止する場合は作業フォルダーに `stop-requested` を置く。再開するときにそのファイルを取り除き、同じRunを呼ぶ。元PNGは毎回再ハッシュし、前回の監査済みという表示だけで信用しない。既存の完成SQLiteを上書きして別世代として再開しない。

## 配布SQLite

圧縮順はz6→z13。作業DBと分離し、`terrain-shared.partial.sqlite3`へ書く。

| テーブル | 主な列 |
|---|---|
| metadata | key、JSON value |
| tiles | z,x,y、origin_x,origin_y、kind、points、xyz_sha256、payload、depth。PK(z,x,y) |
| parents | z,x,y、slot、parent_z,parent_x,parent_y。PK(z,x,y,slot) |

kindはNQC1またはNDC1。origin_x/origin_yは整数XY単位でのタイル左上。復号後XYZの正本はglobal i32で、XY=1/16m、Z=1/8m。親は一段上の実タイルをslot順に参照する。地理的な親一枚だけでなく、実際の支持点が共通するタイルを最大4枚選ぶ。世界外周の共通番兵だけを根拠に全国全親へ探索を広げない。

独立アンカーz6/9/12、最大3段・自身込み依存16枚。親の復元点はそのタイルのパック中だけ保持し、全国の全復元点をHashMapへ保持しない。差分が不利・依存上限超過なら独立へ切り替える。独立化でも点数・BLOB上限を守れなければ失敗する。両方式で整数XYZの完全一致を検査する。符号の詳細は `natural_codec.rs` と `sibson_native_decoder.rs` を参照する。

metadataは世代、方式、品質表、EPSG:3857、世界幅、標準XYZ、共通原点、軸方向、量子化、ソース試験窓、誤差監査の対象、検証結果、complete/national_completeを保持する。共通原点はEPSG:3857の(-WORLD/2,+WORLD/2)、保存Xは東向き・Yは南向き。実際のEPSG座標はX=保存X/16-WORLD/2、Y=WORLD/2-保存Y/16。Z=保存Z/8。緯度経度→XYZは標準Webメルカトルの式で計算する。

三角形・傾き・Voronoiセル・GPU索引は配布しない。最終クライアントはブラウザーのSQLiteアクセスとWASM復号で親XYZを共有する方針。現行61493のサーバー復号を常駐ワーカーへ変更する作業は行わない。全国DBをブラウザーへ接続するクライアント実装は別作業。

## 完成判定

パック後はSQLiteから復号し、SHA256、親・深さ・点数、元DEM全有効画素の上下誤差、最小角を再検査する。誤差監査は事前計算Fastとは別の直接Surface評価器を使う。検証キャッシュはXYZハッシュと検証方式版を組み合わせる。同品質の隣接タイルでは辺上の高さと局所傾きを照合する。

検査後にcomplete等をcommitし、WALをcheckpointしてjournal_mode=DELETEに戻し、接続を閉じてから `terrain-shared.sqlite3` へrenameする。途中データを完成名で配信しない。窓付き試験はscope=window-test、national_complete=falseで、外側の元DEMまで監査したとは表示しない。

誤差の保証対象は有効元DEM画素中心でのSibson基準曲面。表示時C1補正やGPUキャッシュ・描画メッシュによる誤差は別であり、同じ保証としない。角度はXY平面の内部接続の条件。

## 資源と失敗

単一Rustプロセス（子生成プロセスなし）の常駐8GiB、private commit10GiBを上限とし、Windows APIで計測。PNG展開はLRU128枚（約64MiB）、SQLiteページキャッシュ64MiB、局所点は既定131072点。上限指定自体も262144点を超えられない。進捗へpeak working set/private、出力先の空きを記録する。

出力先の空き100GiBを下回った場合も保存済み状態から停止する。この値は必要ディスク量の厳密な予測ではない。品質・支持・点数上限を黙って緩めて続行しない。ソース変更、復号不一致、角度未達、支持閉包超過、ディスク不足の理由をprogress.jsonへ残す。

## 検証済みの範囲

Rustテストは33件。ディスクからの支持抽出と一括参照の一致、堅牢な辺交換、全8段の圧縮・復元、接する二保存タイルの境界、再開、未commit削除のrollback、PNG改変と配布ハッシュ改変の検出、資源上限拒否を含む。Python入力準備は3テスト成功。

槍ヶ岳・富士山・筑波山それぞれ実PNG一枚の小範囲で全8品質を生成し、元DEM65,536画素×8段を保存後に再監査した。三地点128km全体の再生成ではない。各試験の完成SQLiteは `build/national-shared-{region}-pilot-v1/terrain-shared.sqlite3`。試験完了を全国完了に数えない。

外部メモリー試験は、既存の実地形制御点を独立複製して実行。新しい512kmの元DEM適合を試したものではない。結果は `build/shared-disk-stress-yari128-v1/report.json` と `build/shared-disk-stress-tsukuba512-v1/report.json`。入力データ共有で描画負荷を省く試験ではなく、別座標・別DB行の点を作成する。

全国本番の準備済み入力はPNG109506枚、SHA照合9,280,731,082バイト。標準XYZ候補は全8段合計40,180枚。全画素無効の保存領域は生成時に除外するため、これは最終タイル数ではない。全国ジョブは初期化までで、Runは未実行。

# SQLite XYZからの表示復元の検証

2026-11-02。20.83 MiBのbuild/natural-shared-packed-v1/terrain-shared-xyz.sqlite3を読み取り専用で使用する。GPU補助情報は配布DBへ追加しない。現行61491・61492は保持する。

## 表示経路

http://127.0.0.1:61493/?region=tsukuba&quality=200&source=sqlite&span=109013&px=64000&py=64000&yaw=-5.3413&tilt=0.972&join=0&c1Amount=0.35

SQLiteのタイルと親を取得 → RustでNQC1/NDC1を整数復号 → 共有支持点からDelaunay/Voronoiを復元 → 局所傾きを計算 → GPU検索索引を生成 → WebGPUへ制御点・セル・索引を送る → GPUで曲面を評価して描画。

復号・補助生成は今回サーバー側のRust。ブラウザWASMでSQLiteを開く経路を実装したという意味ではない。実際のネイティブアプリへは同じRust処理を組み込む設計とできるが、アプリ接続は未実装。

## 25 mで見つかった対角線依存

初回の復元では、富士山25 mの境界で高さ差約0.462 m、勾配差約0.00953を検出した。整数XYZは一致していたが、局所傾きがDelaunayの隣接点すべてを同じように使っていたため、共円の格子で一意でない対角線の選択に依存していた。XYZだけから元の傾きを安定復元する条件を満たしていなかった。

natural_local_gradient.rsへ計算を共通化。共有辺の両側の外接円中心が一致し、双対Voronoi辺長が実質0となる対角線を、傾きの最小二乗の近傍から除外した。判定は中心差二乗<=1e-14*max(辺長二乗,1)。これは補間を別方式へ置き換える変更ではなく、自由な対角線を傾きの根拠にしない変更。共円格子の入力順反転・部分集合で同じ傾きになるテストを追加した。

旧表示と黙って同一と扱わないため、比較元の全24組をbuild/natural-shared-128-v2へ再生成した。全域最小二乗の比較は省き、安定近傍の一括生成と作業領域分割の二方式を用意した。元のv1・61492は変更していない。61493の選択肢はSQLite再生成、安定近傍の比較元、旧局所方式（61492）。旧局所方式とは対角線の影響があった点の曲面・陰影が変わる。

## 検証結果

- Rustのパッカー・再生成・Sibson・GPU索引の関連27テスト、比較生成の18テストが成功。
- 元の生成フォルダーが存在しない条件でも、SQLiteの深さ2の親参照（9依存タイル）を復号し、補助情報を再生成できた。ディスクキャッシュからの再読込も同一。
- Chrome/WebGPUで三地点×8品質の24ケースについて、既存の安定近傍パケットとSQLite再生成を比較した。GPUキャッシュの検査サンプルで最大高さ差0.000213623 m（約0.22 mm）、最大勾配差9.1553e-5。
- 同品質の描画境界は最大高さ差3.7253e-8 m、最大勾配差1.1921e-7。取得失敗・GPU/JavaScriptエラーなし。
- 578タイルの新規再生成の中央値は、Rust内合計4.26 ms、うちSQLite依存取得・復号1.28 ms。子プロセス起動・待ち時間を含むHTTP要求内の中央値30.78 ms、最大133.93 ms。測定はこのPC・試験中の可視タイルに対する値で、全タイル・全端末の保証ではない。
- ブラウザーの方式切替から完了待ちまでの中央値0.476秒、最大1.014秒。テストの150 ms待機も含むため、純粋な処理時間とは区別する。

表示の一致は検査点での比較。番兵の量子化やfloat32の数値差を含む。元DEMへの新たな全画素監査、C1適用後の絶対誤差保証、全国版・混合LODの完成とは異なる。

## キャッシュと再実行

新規生成は最大2子プロセスを同時実行。タイルごとのロックで二重生成を防ぐ。キーにはSQLiteとRust実行ファイルのSHA-256を含め、異なる傾き方式のキャッシュを混用しない。GPUパケットは派生データとしてbuild/natural-shared-packed-v1/viewer-cache/<hash>/以下へ保存する。SQL元ファイルのサイズは21,844,992 Bのまま。

```powershell
./tools/terrain-rs/build.ps1 -NaturalSharedPack -TargetDirectory build/natural-fixed-lpf-target
./tools/terrain-rs/build.ps1 -NaturalSharedPack -Test -TargetDirectory build/natural-fixed-lpf-target
./tools/terrain/start_natural_shared_sqlite_viewer.ps1
# または tools/terrain/start_natural_shared_sqlite_viewer.bat
./build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/test_natural_shared_sqlite.py
# NODE_PATHへPlaywrightを設定後:
node tools/terrain/test_natural_shared_sqlite_viewer.cjs
```

比較元v2の生成:

```powershell
./tools/terrain-rs/build.ps1 -NaturalSharedCompare -TargetDirectory build/natural-fixed-lpf-target
./build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/build_natural_shared_compare.py --output build/natural-shared-128-v2 --local-only
```

比較結果build/natural-shared-packed-v1/viewer-comparison.json、再生成時間sqlite-viewer-status.json、ブラウザー実行ログbuild/natural-shared-sqlite-comparison.log。http://127.0.0.1:61493/sqlite-status で今回のサーバーの再生成件数・時間を確認できる。

作業でSQLite本体・元DEM・v1を削除／更新していない。コミット・push・公開なし。


### 2026-11-02 SQLiteビューワーに自動LODを追加

61493で可視深度と画面誤差に応じた25～500 mの自動選択、異品質の境界接合を実装。同品質は共通曲面のまま評価。三地点の広域・近景・混合、連続ズーム、固定品質の回帰、150 ms通信遅延中の境界検査が成功。余白0を1 m帯と誤認した仮表示の過剰分割も修正。[仕様と検証記録](NATURAL_SHARED_AUTO_LOD.md)。SQLite本体・元データ・旧61491/61492は保持。

# MLSの補間半径を滑らかに復元する

2026-10-27。

**採用状況: Zを再適合しない旧版は非採用・比較用。** ユーザーの「表示時に強い補間をかけて荒れを解消する方式はNG」という条件により、半径平滑化を標準方式から外した。生成時の適合・制御点配置・曲面表現で荒れを抑え、表示時に追加の平滑化を行わない方針とする。以下は比較実験の記録として保持する。

## 修正対象

50 m以上の粗いMLSに残っていた細い荒れは、主に支持半径の勾配不連続に由来していた。従来のDirectSupportは、評価点から24番目に近い制御点までの距離d24から `r=max(2*SPAN/sqrt(N),1.5*d24)` を決める。24番目の点が入れ替わる境界では半径の値は連続でも勾配が滑らかにつながらず、地形の傾き・陰影に面状模様が出る。

今回の修正では保存した制御点XYZを一切変更せず、半径を復元する関数だけを差し替える。再適合・再量子化・再圧縮はしない。元MLQ1の圧縮BLOBを同一バイト列でコピーして使う。形状BLOBのデータ量は同じ。方式を指定するメタデータは追加する。

## 復元規則

対象地域はWebメルカトル上の32 km四方、相対XYは-16,000～16,000 m。

1. 保存XYからcKDTreeを作る。
2. 地域を256分割した位置で従来のrを評価する。外側に8サンプルの余白を設ける。サンプル座標は `(i/256 - 0.5) * 32000`、i=-8～264。配列は273×273。
3. log(r)に標準偏差1サンプル、打切り4σのGaussianをかける。外側の境界条件はmirror。
4. 平滑化したlog(r)に三次Bスプラインの係数変換を適用する。SciPy spline_filter(order=3, mode=mirror)。
5. 任意の評価XYを `(xy/32000 + 0.5)*256 + 8` へ変換し、係数を `[y,x]` の順序で三次評価する。map_coordinates(order=3, prefilter=False, mode=mirror)。結果をexpして半径に戻す。
6. 得た半径で従来と同じWendland C2重み・二次MLS・正則化1e-5を評価する。

地域内の評価点には十分な余白があり、地域端で半径関数を打ち切らない。補間後の半径をhard maxで切り直すと再び勾配の折れを作るので、log空間での補間後にその処理は行わない。半径は常に正となる。地域外の評価はエラーにする。

この格子は高さを持つ地形の格子ではなく、保存XYから毎回導ける補間半径だけの一時的な補助情報。形状保存は引き続き自由配置XYZ。半径用格子も係数も配布BLOBには保存しない。係数配列の実行時メモリは約0.57 MiB/地域曲面（近傍木などは別）。

元DEMにかけるLPFの幅とは別の処理。元DEMのLPF、制御点XYZ、曲率抑制の条件は前回のLPF＋弱い曲率抑制版と同じまま。

## メタデータと互換性

- supportMode: smooth-log-radius-cubic-v1
- radiusIntervals: 256
- radiusSigmaCells: 1.0
- radiusHaloCells: 8
- radiusFieldStored: false
- sourceBlobSha256: 参照した元MLQ1圧縮BLOBのSHA-256
- pointCoordinatesUnchanged: true
- fittingRepeated: false

BLOBの形式MLQ1だけでは半径の復元方式を区別できないため、読み込み側はsupportModeを必ず見る。mls_smooth_radius.model_from_metadataで新旧を区別して再構築する。未対応の方式名を黙って従来方式に読み替えない。

学習時のaudit/validationは元の曲面に対する値なので、新方式の評価値と混ぜずsourceFittingAudit/sourceFittingValidationへ退避する。新方式は圧縮復元後の曲面で元DEMとLPF目標に対し全11,222,500画素を再評価する。元DEMの目標未達が自動で解決する変更ではない。

## 生成・表示

```powershell
build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/generate_smooth_radius.py --errors 50
build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/generate_smooth_radius.py --errors 50 100 200 400 800 1600 3200
build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/render_lpf_dem.py
build/terrain-venv/Scripts/python.exe -m unittest discover -s tools/terrain -p test_mls_smooth_radius.py
build/terrain-venv/Scripts/python.exe -m unittest discover -s tools/terrain -p test_smooth_radius_results.py
node tools/terrain/test_smooth_radius_browser.cjs
```

出力はbuild/mls-smooth-radius-v1。元データはbuild/mls-lpf-dem-v2の各独立MLQ1。元PNGと形状BLOBのSHA-256、支持半径の設定を確認し、以前と違う条件での再開を拒否する。既存生成物は削除しない。地域ごとのログ・PID・進捗を記録する。

比較URLは http://127.0.0.1:61484/dem-lpf/?variant=lpf-smooth 。生成方式「半径平滑化（非採用・比較用）」で選ぶ。従来の半径方式は「LPF＋曲率抑制（弱）」として残し、同じ視点で比較できる。地形の表示そのものにBスプライン補間を追加したわけではない。表示はMLSを評価した描画用格子を三角形として描画し、今回はその前のMLS評価で半径関数を変更している。


## 全画素評価結果

各地域11,222,500元画素、欠損0。最大差とRMSEは圧縮復元後の新しい曲面で評価したもの。目標未達は引き続き画面で明示する。

| 地域 | 段階 | 形状BLOB | 元DEM最大差 | 元DEM RMSE | LPF目標最大差 | LPF目標 RMSE |
|---|---:|---:|---:|---:|---:|---:|
| 富士山 | 50 m | 39.0 KiB | 77.50 m | 6.86 m | 42.69 m | 3.58 m |
| 富士山 | 100 m | 10.8 KiB | 133.13 m | 11.42 m | 51.06 m | 5.37 m |
| 富士山 | 200 m | 3.0 KiB | 229.42 m | 20.30 m | 115.12 m | 9.83 m |
| 富士山 | 400 m | 0.9 KiB | 433.74 m | 38.73 m | 181.98 m | 22.19 m |
| 富士山 | 800 m | 0.6 KiB | 635.49 m | 68.63 m | 295.03 m | 53.50 m |
| 富士山 | 1600 m | 0.3 KiB | 835.46 m | 83.28 m | 199.18 m | 52.16 m |
| 富士山 | 3200 m | 0.3 KiB | 932.18 m | 133.47 m | 359.34 m | 101.49 m |
| 槍ヶ岳 | 50 m | 45.8 KiB | 153.38 m | 33.40 m | 84.92 m | 21.28 m |
| 槍ヶ岳 | 100 m | 11.1 KiB | 244.00 m | 63.14 m | 140.94 m | 37.81 m |
| 槍ヶ岳 | 200 m | 3.2 KiB | 424.06 m | 120.02 m | 284.94 m | 69.58 m |
| 槍ヶ岳 | 400 m | 1.0 KiB | 678.51 m | 194.80 m | 436.88 m | 102.49 m |
| 槍ヶ岳 | 800 m | 0.6 KiB | 858.62 m | 251.38 m | 451.93 m | 134.17 m |
| 槍ヶ岳 | 1600 m | 0.4 KiB | 1012.61 m | 308.14 m | 488.18 m | 162.81 m |
| 槍ヶ岳 | 3200 m | 0.3 KiB | 998.24 m | 317.14 m | 397.04 m | 155.82 m |
| 筑波山 | 50 m | 38.1 KiB | 64.45 m | 7.13 m | 38.39 m | 3.23 m |
| 筑波山 | 100 m | 10.5 KiB | 111.88 m | 11.88 m | 40.26 m | 4.85 m |
| 筑波山 | 200 m | 3.0 KiB | 199.11 m | 20.86 m | 77.42 m | 8.42 m |
| 筑波山 | 400 m | 0.9 KiB | 312.04 m | 38.84 m | 94.88 m | 17.71 m |
| 筑波山 | 800 m | 0.5 KiB | 456.98 m | 59.32 m | 156.95 m | 33.06 m |
| 筑波山 | 1600 m | 0.3 KiB | 535.77 m | 74.15 m | 186.17 m | 39.80 m |
| 筑波山 | 3200 m | 0.3 KiB | 553.48 m | 83.19 m | 170.34 m | 46.59 m |

50 m版・中心8 kmでの高周波誤差勾配RMSは、従来の半径に比べ富士山75.7%、槍ヶ岳79.6%、筑波山68.3%低下。標高RMSEはほぼ同じでわずかに改善。これはLPF目標との差の細部成分を使った診断値で、視覚上の完全な無模様や元DEM誤差目標を保証しない。

検証: 支持半径の決定性・微分連続性・平面保持・不正条件4件、既存ネイティブMLSとの一致2件、新21段階のBLOB同一・復元と描画データの一致・50 mの改善2件に成功。Chromeで3地域×7段階、新旧切替、参照面切替、視点保持を確認。JavaScript/WebGLエラーなし。


## 生成時から同じ半径を使う後継版

ユーザーが「生成時にも同じ半径計算を使い、曲面を再適合する」方針を選択したため、生成時と復元時を統一した版を追加した。これは本書のZを変更しない比較実験とは別で、表示時の追加平滑化を行わない。詳細はMLS_SMOOTH_REFIT.mdを参照。

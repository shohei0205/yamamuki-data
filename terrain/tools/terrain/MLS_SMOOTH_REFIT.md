# 滑らかな補間半径を生成時から使うMLS再適合

2026-10-27。

## 方針

ユーザーが選択した「生成時にも同じ半径計算を使い、曲面を再適合する」方式。表示時に強い補間や平滑化を追加して荒れを隠す方式は採用しない。

前回の比較版は、従来の半径で適合した点XYZをそのまま使い、復元時の半径だけを変更していた。今回は適合行列を作る段階からSmoothSupportを使い、LPF後のDEMへ制御点Zを再適合する。圧縮後の復元・検証・描画データ生成でも同じSmoothSupportを使う。

XYと点数は既存の各段階と同じ。Zは最適化・量子化・再圧縮するため、形状BLOBが元版と同一になることは保証しない。旧版の全データを保持する。

## 生成・復元の共通定義

支持半径の定義はMLS_SMOOTH_RADIUS.mdのsmooth-log-radius-cubic-v1。保存XYから半径用の一時的な補助関数を復元する。256分割＋外側8サンプル、log半径のGaussian σ1サンプル、三次Bスプライン評価。保存形状は自由配置XYZで、半径用の補助格子は保存しない。

生成時にこの半径で二次MLSの設計行列を作り、従来どおり近傍一次面からの逸脱に対する弱い罰則λ=1/目標誤差を追加してZを適合する。元DEMへのLPF幅は平均点間隔の1/4。8反復までの重み付きLSMR、damp=0.003、最大500反復。Z量子化0.125/0.5/1/2/4 mを比較する。評価には生成時と同じ半径・二次MLS・正則化を使う。

復元時には半径の補助関数を構築する計算は必要だが、これは保存曲面の定義そのものの復元。表示時に高さ・法線・画像へ別の平滑化を追加しない。表示専用に補間次数や半径を強めない。

## メタデータ

- supportMode: smooth-log-radius-cubic-v1
- radiusIntervals: 256
- radiusSigmaCells: 1.0
- radiusHaloCells: 8
- radiusFieldStored: false
- fittingRepeated: true
- sameSupportAtFitAndReconstruction: true
- additionalDisplaySmoothing: false
- xyCoordinatesUnchanged: true

model_from_metadataで復元する。MLQ1のBLOBだけで新旧の支持半径は判別できないため、メタデータを必ず併用する。今回のaudit/validationは生成時から新しい支持半径を使った実測値。再適合前の値を流用しない。

## 評価と比較

元PNG DEMとLPF目標それぞれについて、全11,222,500元画素で最大差・中心2 km最大差・RMSEを測る。目標未達も表示する。再適合が、すべての指標を必ず改善することは保証しない。

50 mの初回比較では、半径だけを変更した旧比較版に対して、中心8 kmでのLPF目標へのRMSEは富士山4.240→4.230 m、槍ヶ岳19.066→19.128 m、筑波山4.492→4.481 m。細かな陰影誤差はほぼ同じ。全域の最大誤差などには増減があるため、再適合なし版も比較用として保持する。今回の主な目的は生成と復元の計算の一貫性である。

## 実行

```powershell
build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/generate_smooth_refit.py --errors 50
build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/generate_smooth_refit.py --errors 50 100 200 400 800 1600 3200
build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/render_lpf_dem.py
build/terrain-venv/Scripts/python.exe -m unittest discover -s tools/terrain -p test_smooth_refit_results.py
node tools/terrain/test_smooth_refit_browser.cjs
```

出力はbuild/mls-smooth-refit-v1、参照XYはbuild/mls-lpf-dem-v2の各独立モデル。元PNG、LPF用の周辺PNG、参照BLOBのSHA-256と支持半径の設定を確認する。保存済み設定と異なる条件での再開を拒否する。地域別ログ、PID、進捗、段階別の設定・適合係数・圧縮BLOB・全画素評価を保存する。

比較URL: http://127.0.0.1:61484/dem-lpf/?variant=lpf-refit 。「LPF＋滑らかな補間半径（生成時に再適合）」で選ぶ。「半径平滑化（非採用・比較用）」はZを再適合せず復元方式だけ変えた旧実験であり、区別する。


## 全元画素評価結果

各地域の全11,222,500元画素で評価、欠損0。目標未達も画面に明示する。

| 地域 | 目標 | 圧縮容量 | 元DEM最大差 | 元DEM RMSE | LPF目標最大差 | LPF目標 RMSE |
|---|---:|---:|---:|---:|---:|---:|
| 富士山 | 50 m | 39.0 KiB | 77.45 m | 6.85 m | 42.40 m | 3.58 m |
| 富士山 | 100 m | 10.8 KiB | 133.17 m | 11.39 m | 51.05 m | 5.36 m |
| 富士山 | 200 m | 3.0 KiB | 229.99 m | 20.27 m | 115.66 m | 9.83 m |
| 富士山 | 400 m | 0.9 KiB | 433.47 m | 38.71 m | 181.96 m | 22.13 m |
| 富士山 | 800 m | 0.6 KiB | 634.60 m | 68.58 m | 295.71 m | 53.54 m |
| 富士山 | 1600 m | 0.3 KiB | 834.97 m | 83.15 m | 199.32 m | 52.03 m |
| 富士山 | 3200 m | 0.3 KiB | 931.10 m | 133.48 m | 360.58 m | 101.61 m |
| 槍ヶ岳 | 50 m | 45.8 KiB | 153.71 m | 33.41 m | 91.42 m | 21.31 m |
| 槍ヶ岳 | 100 m | 11.1 KiB | 244.96 m | 63.05 m | 139.44 m | 37.73 m |
| 槍ヶ岳 | 200 m | 3.3 KiB | 423.65 m | 119.90 m | 284.46 m | 69.50 m |
| 槍ヶ岳 | 400 m | 1.0 KiB | 677.79 m | 194.75 m | 437.14 m | 102.46 m |
| 槍ヶ岳 | 800 m | 0.6 KiB | 859.30 m | 251.31 m | 450.84 m | 134.11 m |
| 槍ヶ岳 | 1600 m | 0.4 KiB | 1011.44 m | 308.16 m | 488.29 m | 162.84 m |
| 槍ヶ岳 | 3200 m | 0.3 KiB | 998.24 m | 317.14 m | 397.04 m | 155.82 m |
| 筑波山 | 50 m | 38.1 KiB | 64.67 m | 7.12 m | 38.39 m | 3.22 m |
| 筑波山 | 100 m | 10.5 KiB | 112.11 m | 11.87 m | 40.55 m | 4.84 m |
| 筑波山 | 200 m | 3.0 KiB | 200.19 m | 20.86 m | 77.32 m | 8.40 m |
| 筑波山 | 400 m | 0.9 KiB | 310.48 m | 38.81 m | 94.36 m | 17.71 m |
| 筑波山 | 800 m | 0.5 KiB | 456.91 m | 59.25 m | 155.88 m | 33.00 m |
| 筑波山 | 1600 m | 0.3 KiB | 535.95 m | 74.16 m | 186.40 m | 39.81 m |
| 筑波山 | 3200 m | 0.3 KiB | 553.37 m | 83.17 m | 170.26 m | 46.57 m |

検証: 既存LPFの欠損・平面保持等3件、支持半径の決定性・連続性・平面保持等4件、再適合21件のXY保持・圧縮復元と適合時の行列評価の一致・描画データの一致・50 mの保存済み検証値の再計算2件に成功。Chromeで全21件、新旧方式・参照面切替、視点保持を確認。JavaScript/WebGLエラーなし。

# Laplace 自然近傍の比較

2026-10-28。元の自由配置制御点を変えず、61485の補間方式へLaplaceを追加した。URLは /?region=yari&method=laplace&level=100&span=18000&draw=accurate&shade=vertex 。元のSQLite・生成処理・圧縮形式は変更しない。

## 計算

照会点を挿入したときのVoronoiセルを求め、各辺を共有する制御点iに対して w_i=辺長/制御点までの距離 とし、総和で正規化してZを補間する。Sibsonの面積重複計算は行わない。Delaunay探索とセル切断は既存方式を再利用し、座標原点を照会点へ移動して倍精度で計算する。辺の所有者は中点と各垂直二等分線の距離で判定する。完全一致点は元のZを返す。矩形凸包上は既存の線形辺座標を使う。

C実装はlaplace_eval.c、Pythonはlaplace_surface.py。sibson_surface.pyは既定DLLを保持し、任意DLL指定だけを追加した。Laplace DLLは既存Sibsonと同じ呼出ABIを使い、別ファイルのbuild/laplace_eval.dllへビルドする。旧DLLを置き換えない。

ビルド: build/rust-tools/llvm-mingw/llvm-mingw-20261006-ucrt-x86_64/bin/clang.exe -O3 -shared tools/terrain/laplace_eval.c -o build/laplace_eval.dll

方式の参考: [NaturalNeighbours.jl の補間式](https://danielvandh.github.io/NaturalNeighbours.jl/stable/interpolation_math/)。実装は本リポジトリのセル切断から作成し、ライブラリのコードは移植していない。

## 実測

各地域の同じ代表タイルと同じ16,384評価点。Delaunay構築を除き、同じ少数点でウォームアップ後に1回評価した時間。キャッシュ済みのfield応答を測ったものではない。

| 地域 | 制御点 | Sibson | Laplace |
|---|---:|---:|---:|
| 槍ヶ岳 | 711 | 19.68 ms | 7.76 ms |
| 富士山 | 145 | 20.17 ms | 8.02 ms |
| 筑波山 | 175 | 18.94 ms | 7.63 ms |

曲面評価は約2.5倍速いが、ブラウザー描画FPSの倍率ではない。表示はサーバーで高さと1 m差分勾配を求めたポリゴンを描画し、画素単位のLaplace計算は未実装。Sibson向け生成点の誤差保証をそのまま適用できない。

## 検証と目視所見

test_laplace_surface.pyにて、SciPy Voronoiへ照会点を独立に追加した共有辺長との30点照合（誤差1e-8以内）、元点の高さ再現、平面再現に成功。test_laplace_viewer.cjsにてSibsonとの往復切替、同じ制御点数、WebGLエラーなしを確認。

槍ヶ岳100 m比較画像ではLaplace側に弧状の陰影境界が目立つ。高速化の効果は確認したが、地形表示として品質が良くなったとは判断していない。表示側に強い平滑化を足してこの差を隠す対応はしていない。比較画像はbuild/sibson-region128-v1/laplace-yari.png、sibson-yari-comparison.png。数値結果はlaplace-benchmark.json。

# LPF・共通XY・レベル別高さの曲面比較

更新日: 2026-10-25

## 比較画面

[曲面誤差で点を追加し局所近似する版](http://127.0.0.1:61465/) と、[既存点の高さだけを変更する版](http://127.0.0.1:61464/) を用意した。富士山・槍ヶ岳・筑波山の、投影上32 km四方・z6～z14を対象とする。地表の一辺は緯度に応じて短くなる。元の全国SQLite、元標高、Web版とネイティブ版の表示は変更しない。

左右比較はカメラを共有する。左は共通の点配置で元標高を平面表示し、右をLPF＋平面／LPF＋3次曲面に切り替える。曲面適合版では点を追加しているため、左も追加後の点配置を使う。現在の全国メッシュを無加工で描いた画面ではない。点数・追加数・親から継承した点数・容量を画面に表示する。東西断面も比較できる。

## 生成

1. Rustの `tools/terrain-viewer/examples/lpf_export.rs` で全国のYMV4を復元し、3地域の64 km四方の点を別SQLiteへ抽出する。読み取り専用で全国DBを開く。
2. 比較はその中央32 km四方を使い、四隅を補う。粗いレベルのXYを次の詳細レベルにも必ず含める。点を移動する処理は行わない。
3. 元標高を投影31.25 m間隔で生成時だけ評価し、各レベルにGaussian LPFを適用する。地表のσを緯度のcosで投影長に換算し、最大σの6倍以上の余白を取る。FFTでフィルタリングする。元データの値なしは0 mで扱うため、海岸の再現性を評価する試験ではない。
4. フィルタ済み標高と勾配からClough–Tocherの区分3次曲面を作る。曲面適合版は投影約248.75 m間隔の129×129評価点で高さの差を測り、誤差超過点を追加する。離れた候補を最大32点まとめて追加した後、全候補を再評価する。追加点は詳細レベルにも継承する。
5. XYは共通のまま、レベル別の高さを保存する。共有点の高さは親との差分、新規点は絶対高さ。float32で復元した誤差を1 mm未満と照合する。
6. 各三角形を重心で3分割するClough–Tocherの各小領域について、3次三角形Bézierの高さ係数を10個ずつ求める。別の点でSciPyの参照曲面と照合し、差が2 cm以上なら生成を失敗させる。

| z | 地表のLPF σ | 曲面適合の評価点での許容差 |
|---|---:|---:|
| 6 | 2,000 m | 400 m |
| 7 | 1,000 m | 200 m |
| 8 | 500 m | 100 m |
| 9 | 250 m | 50 m |
| 10 | 125 m | 25 m |
| 11 | 62.5 m | 12.5 m |
| 12 | 31.25 m | 12.5 m |
| 13 | 15.625 m | 12.5 m |
| 14 | なし | 12.5 m |

これは試作用の設定で、全国生成の許容誤差を変更するものではない。評価点は約249 m間隔なので、全元標高標本に対する最大誤差を保証しない。特にσが評価標高の31.25 m間隔に近い詳細レベルはLPFの近似精度に制約がある。z14の頂点の高さは元の値を使うが、曲面の勾配は生成時の評価格子から求める。

点配置は既存点と追加点から再度Delaunay分割する。元の全国メッシュの三角形列や最小角10度の保証をそのまま引き継ぐものではない。領域外周と、異なるz同士の接合はこの単一レベル比較には統合していない。

## 曲面の表示

ブラウザーに標高格子を渡さず、点・三角形・Bézier係数だけを渡す。GPUの頂点シェーダーがXYの線形な重心座標と高さの3次式を評価する。XY方向に曲面を膨らませない。法線は同じ3次式の解析的な微分から計算する。Loopや、分割後に形を丸める処理は使わない。

各面の3つの小パッチを同じ分割数で描画する。描画分割は曲面を三角形に近似するための設定で、係数や保存メッシュを変えない。400万描画面を超える設定は実際の分割数を下げて表示する。今回の画面は単一レベルの比較であり、自動LODの評価ではない。

Clough–Tocherでは元の三角形の間も高さと傾きをつなぐ。実装では同じ頂点勾配を全隣接面で共有する。SciPyの `grad` 配列を差し替える試作実装なので、SciPyの更新時には連続性テストと係数の参照照合を再実行する。

## 保存形式と容量

出力は `build/lpf-curve-fit-prototype/comparison.sqlite3`。高さ変更のみの版は `build/lpf-curve-prototype/comparison.sqlite3`。生成物はコミットしない。

- `regions(name, info)`: 地域名、共通中心、評価範囲、標高評価間隔、緯度。
- `levels(region,z,info,xy_new,height_original,height_delta,triangles,coefficients)`: 各レベルの比較データ。
- `xy_new`: このレベルで増えたXYだけ。共通座標UNIT=40075016.68557849/268435456 mの符号付きint32を2つ並べる。親の点列を先頭に連結して全点列を復元する。
- `height_delta`: 全点数分のfloat32。先頭の親点数分は親との差、残りは絶対高さ。
- `height_original`: 比較表示専用の元高さ。配布に必須なデータではない。
- `triangles`: このレベル独立のuint32×3の点番号列。
- `coefficients`: 元三角形ごとに小パッチ3つ×10係数のfloat32。係数はXYを含まず、高さだけ。素朴な展開済み形式なので、容量効率の良い最終形式ではない。

係数の順は重心座標a,b,cについて a³,b³,c³,3a²b,3ab²,3a²c,3ac²,3b²c,3bc²,6abc。小パッチのXYは元の三角形の辺と重心から復元する。

地域内のz6から順に復元する試作形式で、全国形式の親参照深度や読み込み単位の仕様ではない。将来はXY参照と高さ差分を分離し、必要な傾きや制御点から係数を復元する形式も比較する必要がある。

HTTPの `LPC1` は16バイトのヘッダー（magic、頂点数、元三角形数、係数数のu32）に、各点のfloat32 XY・元高さ・フィルタ後高さ、uint32の面、float32の係数を並べる。HTTP転送だけgzipを使う。現在のTFC2や配布用コーデックを置き換えていない。サイズ表示はSQLite内の未圧縮BLOBの内訳で、HTTP転送量とは別。

## 再現方法

標準のRust/Cargo環境で、terrainディレクトリから実行する。新規出力を指定し、元の生成物は上書きしない。

```powershell
cargo run --locked --release --manifest-path tools/terrain-viewer/Cargo.toml --example lpf_export -- build/terrain-national-xyz-20261024/terrain.sqlite3 build/lpf-input-new.sqlite3
./build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/lpf_curve_compare.py generate --fit --input build/lpf-input-new.sqlite3 --output build/lpf-curve-new
./build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/lpf_curve_compare.py serve --output build/lpf-curve-new --port 61465
```

生成はPython/SciPyによる比較用実装。既存点の復元はRust、曲面の表示はWebGL 2。全国生成器をPythonへ戻す変更ではない。`--resume` で保存済みレベルを復元して続けられる。`progress.json` とログ、`report.json` に状態・点数・容量を残す。

```powershell
./build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/test_lpf_curve_compare.py
$env:LPF_CURVE_DB='build/lpf-curve-fit-prototype/comparison.sqlite3'
./build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/test_lpf_curve_compare.py
node tools/terrain/test_lpf_curve_compare.cjs build/lpf-curve-fit-prototype/browser-check http://127.0.0.1:61465
```

ブラウザーテストにはPlaywrightとChromeが必要。単体テストは平面の再現・共有辺の勾配連続性・全27レベルのXY包含と高さ差分復元を検証する。曲面係数は生成時にも全パッチを参照照合する。

## 検証結果（2026-10-25）

曲面適合版の3地域・全27レベルが完了。富士山z6は7点から11点、槍ヶ岳z6は8点から40点、筑波山z6は6点のまま。粗いレベルで追加した点も全詳細側に含まれる。各評価点で設定した曲面誤差以下になったことを確認した。保存係数と参照曲面の照合差の最大は約0.000103 m。曲面適合版SQLiteの実サイズは98,037,760 bytesで、比較専用の元高さと展開済み係数も含む。

Pythonの3テストは、高さ変更のみの版と曲面適合版の両方で成功。Chromeの7表示条件（3地域のz6/z8、富士山z14）、方式の切り替え、右ドラッグ、WebGLエラーなしを確認した。記録は `build/lpf-curve-fit-prototype/browser-check/verification.json` と画像。旧データと生成器は保持しており、全国再生成・混在LOD接合・配布形式の確定は行っていない。

## z14の星形模様への対処（2026-10-25）

z14で補間点の周囲に細かい星形の凹凸が生じた。画素ごとに解析法線を計算しても残り、面法線でも現れたため、法線の補間だけの問題ではなかった。実データの共有辺における法線差を数値照合したところ、変更前の富士山z14の最大は約0.00024で、辺で法線が断絶する現象とは区別できた。

高さを厳密に通す曲面を、局所的な2次近似を許す曲面へ変更した。XY・三角形・頂点数は保持し、各点の近傍最大24点を用いて2回近似する。高さ調整の合計は元のLPF後高さから最大12.5 m、領域の凸包上の点は固定。調整後の高さ群からClough–Tocherの共通勾配を推定する。旧DEM勾配の強制は使わない。法線は頂点法線の線形補間をやめ、フラグメントシェーダーでBézierの解析微分を毎画素評価する。

旧出力は保持し、新しい出力を `build/lpf-curve-smooth-prototype/comparison.sqlite3` に保存した。61465はこの出力を使う。高さ調整分は、変更後の親高さを基準に差分を書き直している。従来の曲面適合評価の結果は `curveFitGridErrorBeforeRelaxation` に保持し、調整後に同じ誤差上限を再保証した値としては扱わない。曲面の近似によって小さな地形も滑らかになる。星形の抑制と細部保持のバランスは、引き続き画面で比較する。

```powershell
./build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/lpf_curve_compare.py smooth --base build/lpf-curve-fit-prototype/comparison.sqlite3 --output build/lpf-curve-smooth-new
./build/terrain-venv/Scripts/python.exe -X utf8 tools/terrain/lpf_curve_compare.py serve --output build/lpf-curve-smooth-new --port 61465
```

5テスト成功。平面の保持・ノイズを含む入力の改善・高さ調整12.5 m以内・全27レベルのXYと三角形の保持・高さ差分を照合した。Chromeで3地域すべてのz6/z8/z14、計9表示条件と法線ON/OFF・接近表示を確認した。画像と検証JSONは `build/lpf-curve-smooth-prototype/browser-check/`。SQLiteは135,651,328 bytes。格納したレポートの長さ増加や空きページも含む実ファイルの値であり、配布時の圧縮容量ではない。

## 曲面との直接交差による描画（2026-10-25）

61465 の「曲面の描画」に「直接交差（レイキャスト）」と「三角形で近似（従来）」を追加した。初期値は直接交差。曲面係数と保存メッシュは共通で、全国データの再生成はしていない。

直接交差は、各3次Bézierパッチを囲む三角柱を8枚の補助三角形で描き、その画素で視線と曲面の交点を求める。補助三角形は描画を開始する範囲だけを定め、地表形状や法線には使わない。視線上の高さ差を3次式にし、微分の根で単調区間に分けて、最も手前の交点を選ぶ。二分探索とNewton補正を併用する。深度は実際の交点から書き込み、陰影は曲面の解析微分から求める。

この方式では近似描画用の分割数と法線チェックを無効にする。値自体は保持し、従来方式へ戻すと再利用できる。ワイヤー表示は曲面パッチの境界を表示する。GPUへの三角形送信自体を廃止したわけではなく、地表を小さな平面で近似する処理を置き換えた。

MSAAのサンプル位置と画素中心の交点判定が一致しないため、比較キャンバスは単一サンプルにした。細い欠けは減ったが、実データ画像には点状の小さな欠けが一部残る。完全な接合品質や実用速度を保証する段階ではない。曲面自体の小さな凹凸も残り得る。

検証: Python 5テスト成功。CPUの3次式8条件、実際のWebGLで10条件（複数交点、接線、深度の前後関係、近クリップ、柱内の視点、共有辺等）が成功。Chromeで3地域のz6/z8/z14の9表示条件と従来方式への切り替え、WebGLエラーなしを確認した。直接交差では近似用分割数・法線設定を変更してもフレームバッファのハッシュが一致する。記録は `build/lpf-curve-smooth-prototype/ray-check/` と `ray-browser-check/`。

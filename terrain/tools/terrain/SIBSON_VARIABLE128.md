# 128 km・可変サイズ・補助情報付きSibsonタイル

2026-10-28。富士山・槍ヶ岳・筑波山の周囲128,000 m四方を全7品質、品質別の保存タイルサイズで生成した。既存の8 km固定版と全元データは保持する。新しく曲面を適合し直したのではなく、生成・全画素監査済みの各128 km共通曲面を分割し直す。

ビューワー: http://127.0.0.1:61485/variable 。元の61485ルートは8 km固定版のまま。

## 品質とタイル

| 最大標高誤差の品質 | 一辺 | 1地域の枚数 | 3地域合計 | 保存 |
|---:|---:|---:|---:|---|
| 800 m | 128 km | 1 | 3 | 独立MLQ1 |
| 400 m | 64 km | 4 | 12 | 独立MLQ1 |
| 200 m | 32 km | 16 | 48 | 400 mからの部分参照NSS2 |
| 100 m | 16 km | 64 | 192 | 200 mからの部分参照NSS2 |
| 50 m | 8 km | 256 | 768 | 独立MLQ1 |
| 25 m | 4 km | 1,024 | 3,072 | 50 mからの部分参照NSS2 |
| 12.5 m | 2 km | 4,096 | 12,288 | 25 mからの部分参照NSS2 |
| 合計 | | 5,461 | 16,383 | |

距離はWebメルカトル平面上の値で、地表実距離と一致するとは限らない。このタイル番号は地域内のx/yであり、国土地理院スリッピータイルのz/x/yそのものではない。

## 出力

build/sibson-variable128-v1/ 以下：

- terrain.sqlite3: 補助情報SAU1を全タイルに含む作業用。192,823,296 B（約183.9 MiB）。
- terrain-distribution.sqlite3: 同じ形状BLOB、補助情報列はNULL。83,406,848 B（約79.5 MiB）。読み込み時に補助情報を作る。旧データと比較する際は作業キャッシュの容量と区別する。
- generation-complete.json / verification.json / distribution.json: 生成、復元・共有辺検証、配布用の実測記録。
- progress.json / verification-progress.json: 生成と検証の状態。
- generation.pid / launch.json / generation.log / generation.err.log、verify.pid / verify.log / verify.err.log: 非表示実行のPIDとログ。
- all-work-complete.json: データ生成・全タイル検証・配布DB・ビューワー確認の完了記録。

形状BLOB合計49,853,394 B、作業用の補助情報BLOB合計105,445,866 B。SQLite実サイズにはページやinfoも含む。補助情報の事前保存を配布要件にはしていない。配布用と作業用の形状BLOB・SHA-256は全件一致確認済み。

## 座標とmetadata

metadata(key TEXT PRIMARY KEY,value TEXT) の値はJSON。

- config: format、span=128000、levels（error/side/tiles/parent）、xyzQuantization=[0.0625,0.0625,0.125]、source、regions。
- config.regions: 地域ごとの元PNGタイル系Webメルカトル中心座標。地域内は北西原点、X東向き・Y南向きで0～128000。
- source_audits: 既存128 km共通モデルの品質ごとの全画素監査結果。今回の分割で新たに元DEM全画素を走査し直したわけではない。

元タイル系のメートル座標への復元はglobal=local+center−64000。WORLD=40075016.68557849とすると、lon=globalX/WORLD*360−180、lat=atan(sinh(pi*(1−2*globalY/WORLD)))*180/pi。これにより緯度経度と地域内x/yの相互変換が可能。保存タイル番号はfloor(localX/side), floor(localY/side)。地域外は範囲外、右・下の端の扱いは端の最終タイルへ制限する。

XYは0.0625 m、Zは0.125 m量子化で既存モデルを保持する。制御点のDEM頂点法線や傾きを保存する形式ではない。

## tilesテーブル

`tiles(region TEXT,error REAL,x INTEGER,y INTEGER,side REAL,parents TEXT,blob BLOB,auxiliary BLOB,sha256 BLOB,info TEXT,PRIMARY KEY(region,error,x,y)) WITHOUT ROWID`

- parents: JSON配列。各要素は[親error,親x,親y]。地域は自分と同じ。
- blob: 独立は既存MLQ1、差分はNSS2。
- auxiliary: SAU1。配布用ではNULLでも有効。SIBSON_AUXILIARY.mdを参照。
- sha256: 復元したXYZ float32 little endianの配列全体の32バイトSHA-256。
- info: bounds（本来の範囲）、fieldBounds（表示用250 m余白を含む範囲）、点数、BLOBサイズ、全域曲面との照合差、親タイル等。

親が大きい場合、その全点を子へ追加しない。子が実際に必要な点だけを参照する。親品質のタイルのうち子の評価範囲に接するものを参照リストに保持し、最大4タイル。親の再帰先も複数になり得る。アンカー配置は従来通りで最大深さ3（自分を含む）だが、必要なタイルの総個数が3枚という意味ではない。

## NSS2の詳細

ヘッダー `<4sIII`（little endian、16バイト）: magic=NSS2、子の点数n、参照親タイル数、追加点数nn。その後にzlib level6の本文。

本文は3ブロック：

1. 各子点のuint32参照n個をバイト分離した4*nバイト。
2. 各子点のint32 Z差分n個をバイト分離した4*nバイト。1単位0.125 m。
3. 新規点のint32 XYZ、nn*3個の32bitワードをバイト分離した12*nnバイト。単位はXY0.0625 m、Z0.125 m。

バイト分離とは、32bitワード列をN×4のuint8として転置し、4×Nのバイト列にすること。復元時は逆の転置を行う。

参照番号の上位8bitはparentsのスロット、下位24bitはその親を復元した点列内の番号。0xffffffffは新規点で、本文の新規点列から順に取る。共有点は親のXYとZを復元し、Zだけ差分を加算する。新規点と共有点をヘッダーの点数順へ並べてSHA-256を照合する。親品質の曲面の評価は必要ない。参照先の点列はその親BLOBの復元順であり、元のDelaunay頂点番号ではない。

MLQ1/NSP1の旧データは上書きしない。NSS2はこの可変サイズ版の追加形式で、旧NSP1デコーダーで読めると扱わない。

## 影響点の抽出と補助情報

元モデルのDelaunay面の外接円と、タイル本来の範囲＋片側250 mの矩形の交差を調べる。該当面の点と、その点の隣接点を保存する。セルの半径別にcKDTreeを作ることで、すべての面をタイルごとに全走査するのを避ける。抽出結果は従来の全走査と乱数矩形20件で同一を確認。粗い品質の必要な外側点が250 mより遠くなることはある。

各タイルは保存後のXYZを復元して配列一致を確認し、その点からSAU1を生成。タイルごとに16乱数点で全域曲面と照合し、差が1e-5 m以内であることを必須にした。同品質は同じ全域曲面を使い、タイルごとの再適合はしない。

## ビューワー

自動LODは128 km根タイルから再帰し、投影標高誤差が指定pxより大きければ子へ分ける。固定品質も選択可能。必要なデータが未生成の場合は粗い側へ戻る。視界に入るタイルを要求する。

描画格子は16～256分割、総頂点目安120万。辺と250 m余白を別の標本列にし、本来のタイル境界の評価位置を正確に共有する。表示格子は保存しない。初回はSAU1から補助モデルを復元し再利用する。配布用のauxiliary=NULLでも読み込み時に構築可能。

異品質では各タイルの矩形から250 mの平滑な重みを作り、周囲の値を正規化して混合する。重みの微分も勾配へ含める。同品質では元曲面をそのまま使う。描画格子が異なる共有辺は、細かい側を粗い側の辺のポリラインへ合わせる。T字や一部境界でも実際の接触範囲だけを扱う。補正帯では細かい品質の誤差保証を維持しない。

読み込み中は以前の完成した描画セットを保持し、要求したセットが揃ってから更新する。サーバー評価2並列、クライアント取得4並列。操作中も計画更新は継続する。画面に元の全域モデルを一括ロードするものではない。

## 検証結果と範囲

- 生成: 全16,383タイル、約335秒。
- 全形状BLOBの親参照復元・SHA-256と全SAU1の復元を確認。
- 同品質の共有辺32,004本、各33点で高さと±1 m差分の傾きを照合。最大差は高さ4.69e-11 m以下、勾配3.98e-11以下。
- SQLite integrity_check成功。
- 部分接合の合成試験: 64/32/16/2 kmタイルで、147個の細かい側の辺頂点が粗い辺のポリラインへ一致。実地形の最大誤差保証とは別のロジック試験。
- 実ブラウザー: 富士山の全7固定品質、3地域の自動LOD、傾斜時の4品質混在、JavaScript/WebGLエラーなし。記録はbrowser-all-levels.jsonと画像。
- 未実施: 異なる地域モデル同士の接合、全品質ペアの実地形での網羅試験、混合帯と描画格子を含む元DEM全画素での誤差再監査。自然近傍のGPU画素評価はこのビューワーでは使わない。

## 再実行

作業場所はterrain。Pythonはbuild/terrain-venv/Scripts/python.exe。

1. tools/terrain/sibson_variable128.py: 保存済みタイルを保持して再開。専用SQLiteロックで二重実行を防ぐ。
2. tools/terrain/verify_sibson_variable128.py: 全復元と共有辺検証。
3. tools/terrain/export_sibson_variable128.py: 補助情報をNULLとした別配布DBを作成。既存出力は上書きしない。
4. tools/terrain/sibson_region128_server.py: 61485で/variableを配信。既存ページも維持。

テスト: test_sibson_variable128.py、test_sibson_variable_joins.cjs、test_sibson_variable_viewer.cjs、test_sibson_variable_all_levels.cjs。生成物をコミットしない。コミット・push・公開は今回行っていない。

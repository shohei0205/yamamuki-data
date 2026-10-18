# 地形データの生成と表示の試作

国土地理院の標高データから地形を生成し、圧縮方式・表示品質・データ量を比較するためのコード。アプリへの組み込みと Releases / Pages への公開処理は未実装。

## 現在の方式

- 全国分は L7〜L12、JPEG XL distance=0.2 のハイトマップ。法線マップは付けない。
- 生成済み SQLite は 11,304 タイル、208,162,816 バイト。データ本体の合計は 189,770,484 バイト。
- 海域と河川・湖沼は保存済み WAT1 / RIV1 ベクターを別に取得する。海域は三角形を直接描画し、地形の切り抜きには補助マスクを使う。河川・湖沼は補助マスクを通して描画する。
- 河川は水面ポリゴンの形状で表示し、川幅を持たない中心線は描かない。画面上で約 0.5 描画ピクセル未満になる水面を近似判定で除く。
- 海岸と河川・湖沼は、それぞれ自動／z10／z11／z12 を選択できる。河川は北海道などに未収録範囲がある。

検証経緯と制約は [調査メモ](tools/terrain/RESEARCH.md)、過去の実行手順は [試作の作業記録](tools/terrain/README.md) にまとめている。作業記録には元の作業環境のパスや、終了した方式の記述も含まれる。

## 配置

地形関連のコードと作業ファイルは `terrain/` 以下に置く。この文書のコマンドは、リポジトリ直下から `cd terrain` した後に実行する。`tools/` と `build/` はいずれも `terrain/` からの相対パス。

| 場所 | 内容 |
|---|---|
| `tools/terrain/` | 標高取得、変換、比較用 HTML / JavaScript、中継サーバー、テスト |
| `tools/terrain-rs/` | Rust の地形・LOD・JPEG XL 生成器と圧縮比較 |
| `build/` | ローカルの元データ・生成物・外部ツール。Git 対象外 |

生成済みデータと実行ファイル、Python 仮想環境はローカルの `terrain/build/` に移動済みで、このブランチには含めていない。元データの取り直しや全国生成を始める前に、必要な保存済みファイルを用意する。

## ローカルの準備と表示

Python 3 と、使う処理に応じた依存パッケージが必要。

```powershell
cd terrain
python -m venv build/terrain-venv
build/terrain-venv/Scripts/python.exe -m pip install -r tools/terrain/requirements.txt
build/terrain-venv/Scripts/python.exe -m pip install -r tools/terrain/requirements-overlays.txt
build/terrain-venv/Scripts/python.exe tools/terrain/heightmap_server.py --port 61440
```

比較用の PNG / JPEG XL、SQLite、海域・水域データを `build/` に用意した後、`http://127.0.0.1:61440/heightmap` と `http://127.0.0.1:61440/national-shading` で確認する。ファイルがない状態では比較画面は動かない。JPEG XL の動的圧縮には別途エンコーダー／デコーダーが必要で、導入は `tools/terrain/setup_codec_tools.py` を参照する。

Rust のビルドは `tools/terrain-rs/build.ps1`、実行は `tools/terrain-rs/run.ps1` を使う。GEOS を使うポリゴン生成と、JPEG XL の処理には対応する外部ライブラリが必要。利用可能なコマンドはビルド後に `tools/terrain-rs/run.ps1 --help` で確認できる。

## テスト

```powershell
build/terrain-venv/Scripts/python.exe -m unittest discover -s tools/terrain -p 'test_*.py'
```

ブラウザーのテストは Node.js、Playwright、Microsoft Edge と、生成済みの比較データおよび起動済みのローカルサーバーを必要とする。例：

```powershell
node tools/terrain/test_tokyo_water.cjs
```

過去の方式を対象とするテストは、その方式の生成物も必要。すべての比較データを再生成するテストではない。

## 出典

標高は国土地理院。海域・河川・湖沼は参照用に保存したデータパックから取得する。各データの由来と利用条件は調査メモに記載している。OpenStreetMap 由来の情報には © OpenStreetMap contributors（ODbL）が適用される。データを配布するときは、出典とライセンスを添付する。

## 移行時の検証（2026-10-18）

Python テストは 117 件中 110 件が成功。残る 7 件は、動的 JPEG XL の比較用法線データがない 3 件、既定の陰影画像がない 1 件、検証用 Ruby がない 2 件、サンドボックスのローカル通信制限による 1 件。必要なローカル資材を用意して再実行する必要がある。

東京湾のブラウザー確認は移行元の生成済みデータとサーバーで実施済み。L4 / L7、傾き 0° / 55°、海岸と河川の異なるレベル指定、自動への復帰、地域表示で WebGL エラーがないことを確認した。移行先には生成物を含めていないため、そのままではこのブラウザーテストを実行できない。

配置変更後も、スクリプトが求める作業ルートは `terrain/`。元の作業環境から生成済みデータを使う場合は `terrain/build/` に用意する。リポジトリ直下の `build/` は参照しない。

配置変更後、ローカル通信を許可して SQLite 中継のテスト 5 件を再実行し、すべて成功した。比較用データ・Ruby が必要な 6 件は未解消。

2026-10-18 に元のアプリ作業先から `terrain/build/` へローカルデータと実行環境を移動した。移動後は Python テスト 117 件がすべて成功し、主要 SQLite 3 件の quick_check と、移行先での東京湾・地域表示のブラウザー確認も成功した。ビューワーは引き続きポート 61440 で利用できる。移動記録と主要ファイルの SHA-256 はローカルの `build/data-migration.json` に保存している。

## 全国ハイトマップをブラウズする

全国 L7〜L12・JPEG XL d=0.2・法線なしの再生成済みデータから、ブラウザー表示用キャッシュを作成する。以下は `terrain/` 内で実行する。

```powershell
build/terrain-venv/Scripts/python.exe tools/terrain/prepare_height_browser.py
build/terrain-venv/Scripts/python.exe tools/terrain/heightmap_server.py --port 61440
```

`http://127.0.0.1:61440/national-height` を開く。初期表示は全国 L7。移動・拡大に応じて L12 まで切り替え、レベルの手動指定もできる。左ドラッグで移動、マウス位置を中心にホイールで拡大縮小、右ドラッグで方角と傾きを変える。槍ヶ岳・富士山・筑波山への移動ボタンもある。高レベルの広域指定は、表示範囲を拡大するまでタイル数を制限する。

JPEG XL の 16 bit 標高をブラウザーで直接復号する代わりに、`djxl` で一度だけ復号した値を PNG の R・G に分けて保存する。標高値を 8 bit へ落とさず、元の最小標高・スケールと有効範囲マスクも使用する。サーバーは保存済みファイルを中継し、投影・陰影・LOD・キャッシュはブラウザーで処理する。法線マップは使わない。海・河川・湖沼は別データの表示レイヤーとして切り替えられる。表示用 PNG の転送量と、配布用 JPEG XL の容量は画面で区別して表示する。

キャッシュは `build/terrain-height-browser/` に置く。生成途中から再開でき、別の元 DB と混在させないため SHA-256 を確認する。ブラウザーのタイルキャッシュには上限を設け、未取得部分は親レベルで補う。中心の緯度経度を URL で指定する例は `/national-height?lat=35.3606&lon=138.7274&span=45`（表示範囲の基準 45 km）。

検証は `python -m unittest discover -s tools/terrain -p test_prepare_height_browser.py` と `node tools/terrain/test_national_height.cjs`。後者は全国、富士山 L9〜L12、北海道、傾き、移動と元 DB の SHA-256 を確認する。

移動の座標変換は `node tools/terrain/test_national_navigation.cjs` で、回転 4 方向・傾き 3 段階のドラッグ方向とズーム中心の保持を確認する。

全国ハイトマップでも海・河川・湖沼を個別に表示できる。海域はベクター面を直接描画し、地形の切り抜きと河川・湖沼は既存の補助マスクを使う。海岸と河川・湖沼のレベルは独立した自動／z10／z11／z12。川幅のない中心線は表示せず、水面ポリゴンが約 0.5 描画ピクセル未満になる部分を非表示にする。水域の元データには既知の未収録範囲がある。水域転送量も地形とは別に表示する。確認は `node tools/terrain/test_national_height_water.cjs`。

タイルの境界は隣接する標高画素を線形補間し、両側の境界頂点を同じ高さにする。角は最大 4 タイルを参照する。未取得の詳細タイルは共通の親標高で補い、補助の垂直面は使わない。補正は表示時だけで、保存済み JPEG XL と PNG は変更しない。検証は `node tools/terrain/test_height_boundaries.cjs` と `node tools/terrain/test_height_tile_seams.cjs`。

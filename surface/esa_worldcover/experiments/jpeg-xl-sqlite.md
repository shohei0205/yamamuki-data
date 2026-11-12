# 全国JPEG XL・distance 4の比較用SQLite

## 概要

森林・水域・陰影の全レベルz5〜z12を、JPEG XLのVarDCT、distance 4、effort 7で圧縮する。各画像の符号化は1スレッド、全国変換は複数画像を並列処理する。

圧縮前PNGから変換する。ESAの分類・ノイズ除去・穴埋め・陰影は既存生成結果を再利用し、既存WebPの再圧縮や元データの再取得は行わない。

これは段階的復号と容量の比較用SQLite。正式な共通SQLite版1とは格納形式が異なり、ネイティブビューワは専用の読み込み経路で対応しているが、正式な共通形式の検査・Release公開処理は未対応。通常の配布SQLiteと間違えて読み込まれないよう、application_idを別にする。正式形式に採用する際の仕様は別途決める。

## グループとタイル

- z5〜z9は各XYZタイルを512×512のJPEG XLにする。
- z10だけ存在し、その子がない範囲は384×384のJPEG XLにする。
- z10・z11の4枚・z12の16枚がそろう範囲では、z12の16枚の圧縮前PNGを並べて1536×1536のJPEG XLにする。
- 同じJPEG XLをz10の1枚、z11の4枚、z12の16枚から参照する。低いレベルの別画像BLOBは保存しない。
- 完全な親子がそろわない範囲は、存在するタイルを個別に保存する。未生成範囲を新しく作らず、既存タイルを落とさない。

現在の全国入力は、各レイヤー22,350タイル、3レイヤー合計67,050タイル。1レイヤーあたり558個のz5〜z9タイル、540個の単独z10タイル、1,012個のz10〜z12共有グループがある。合計6,330個のJPEG XLになる。

## 読み込みと検証

application_idは1498237528（0x594D4A58、YMJX）、user_versionは1。正式な共通形式の版番号ではなく、この実験形式の版番号。

元のdatasets、surface_layers、surface_levelsの値を引き継ぐ。surface_tilesは画像BLOBを持たず、jxl_groupsへの参照と切り出し情報を持つ。生成設定と元ファイルの識別情報はexperiment_infoのconfigurationにJSONで保存する。

### jxl_groups

| 項目 | 意味 |
|---|---|
| group_id | 整数の識別子 |
| dataset_key、layer_key | 元のデータセット・レイヤー |
| z、x、y | グループの地理的範囲を表すXYZ |
| finest_z | 格納した原寸画像に相当する最も細かいレベル |
| pixel_size | 原寸画像の一辺。512、384または1536 |
| distance | 4固定 |
| mode | VarDCT固定 |
| source_tiles | グループが置き換える論理タイル数。1または21 |
| payload_hash | 保存BLOBの破損検査に使うFNV-1a 64ビット。配布時の暗号学的検証用ではない |
| payload | 完全なJPEG XL画像のBLOB |

### surface_tiles

主キーはdataset_key、layer_key、z、x、y。

| 項目 | 意味 |
|---|---|
| group_id | jxl_groupsへの参照 |
| downsample | 原寸から目的レベルへの縮小比。1、2または4 |
| actual_ratio | 復号を止めた時点の実際の精細度。目的段階が得られない場合は1 |
| prefix_bytes | その段階を復号するために必要な先頭からのバイト数 |
| left_px、top_px | 目的レベルの画像上で切り出す矩形の左上 |
| tile_pixels | 切り出す正方形の一辺 |

読み込みは次の順で行う。

1. 対象XYZの行を取得し、group_idのJPEG XLを参照する。
2. 必要に応じてBLOBの先頭prefix_bytesまでを読む。現状のWITHOUT ROWID構造で効率よく部分読み込みする方法は正式実装時に決める。
3. libjxlでactual_ratio相当の段階まで復号する。現在の出力APIでは、粗い段階でもpixel_size四方の出力バッファが必要。
4. 出力をpixel_size / downsample四方へTriangleフィルターで縮小する。
5. left_px、top_pxからtile_pixels四方を切り出す。

共有グループのz11は768×768に縮小してから384×384の4領域へ切る。z10は384×384全体、z12は1536×1536から16領域を切る。個別タイルは縮小・切り出し不要。

同じgroup_idを複数タイルで使う場合、復号結果を共有できる。グループの一部しか表示しない場合でも、現在の復号器ではグループ全体の処理が必要。

### 中間段階が得られない画像

単色などの画像では、目的とする中間段階の通知が得られないケースがある。途中入力だけでは再現できないケースもある。そうした段階は原寸まで復号して縮小し、actual_ratioを1、prefix_bytesを全体の長さとして保存する。画像を欠落させたり、未検証の途中入力長を保存したりしない。

summary.jsonのレベル別full_decode_groupsは、粗い表示でも原寸まで復号するグループ数。符号化自体は他と同じdistance 4の段階式JPEG XLであり、原寸で復号した結果を別画像として保存するものではない。

### 検査と誤差

各グループの全対象レベルで、保存予定の途中入力だけから同じ画像が出ることを確認する。原寸でも非可逆なので、元PNGとの画素一致は要求しない。

jxl_stage_errorsはグループ・レベルごとのRMSE、MAE、最大濃度誤差、評価画素数、復号器と出力バッファのピークを持つ。元PNGの同レベルと比較するため、低いレベルの誤差には、既存の粗画像とz12由来の画像の差も含まれる。

最後に全レベルのタイル数、SQLiteの構造・参照整合性、全BLOBの保存後ハッシュを検査する。summary.jsonには各レイヤーの実格納BLOB合計と、各レベルで全域を表示する場合の重複を除いた必要入力量を分けて記録する。レベルごとの入力量を足すと共有BLOBを重複計上するため、配布容量には使わない。

## 生成・再開

~~~powershell
cargo test --manifest-path surface/Cargo.toml -p yamamuki-data-sqlite --example generate_jxl --release --locked
cargo build --manifest-path surface/Cargo.toml -p yamamuki-data-sqlite --example generate_jxl --release --locked
& surface/target/release/examples/generate_jxl.exe `
    '全国生成済みのesa-worldcover.sqlite3' `
    '同じ生成から得たpacked/landcover-with-hillshade.sqlite3' `
    'libjxl/x64-windows/bin' `
    'build/surface/jxl-national-new' 8
~~~

出力はesa-worldcover-jxl-d4.sqlite3。途中はgeneration.sqlite3.partialを使い、32グループごとに確定する。失敗や中断後に同じ引数で再実行すると、確定済みのグループを飛ばす。元ファイルのパス・サイズ・更新時刻、エンコーダー版、設定が異なる場合は再開しない。完了済み出力への上書きも拒否する。

進捗ページをローカルで表示する場合は、別プロセスで次を起動する。

~~~powershell
& surface/target/release/examples/generate_jxl.exe --serve 'build/surface/jxl-national-new' 61496
~~~

http://127.0.0.1:61496/ で進捗、失敗、容量、残り時間の目安を確認できる。HTTPサーバーが返すのは生成先の進捗HTML・progress.json・summary.jsonだけ。

生成はWindows版公式libjxlを使用する。プログラムとグループ計画のテストはRustで実装している。生成物・入力データ・公式ツールはコミットしない。

## 2026-11-12の全国生成結果

全67,050タイル・6,330グループの生成と検査が完了した。SQLite全体は192,086,016バイト（183.19 MiB）。従来の408,723,968バイト（389.79 MiB）から53.00%減った。画像BLOBの合計は186,047,267バイト（177.43 MiB）。

| レイヤー | 従来のBLOB合計 | JPEG XLのBLOB合計 | 増減 |
|---|---:|---:|---:|
| forest | 90.06 MiB | 91.43 MiB | 1.52% |
| water | 15.13 MiB | 16.97 MiB | 12.20% |
| hillshade | 271.28 MiB | 69.02 MiB | -74.56% |

全国では森林・水域は増え、陰影の削減で全体が減った。5地点の比率とは異なるため、5地点の結果から全国容量を推定していない。

全XYZを元SQLiteと照合して追加0・欠落0、切り出し座標の不整合0。全BLOBの保存後ハッシュ、SQLite整合性、参照整合性、途中入力の再現を検査した。中間段階を原寸復号で補うz11グループは森林308、水域288、陰影289。

富士山・槍ヶ岳・筑波山について、保存済みSQLiteから3レイヤー×z10〜z12の計27タイルを復号・切り出してPNGに出力できることを確認した。抽出は次の形式で実行できる。

~~~powershell
& surface/target/release/examples/generate_jxl.exe --extract `
    'build/surface/jxl-national-new/esa-worldcover-jxl-d4.sqlite3' `
    'libjxl/x64-windows/bin' 'forest-fuji-z10.png' forest 10 906 404
~~~

上のコマンドは実際には一行で指定するか、PowerShellの改行継続記号を使う。出力PNGは新しいファイル名を指定する。

## ネイティブビューワの復号負荷の計測

2026-11-12、jxl-oxide 0.12.6で5地点・3レイヤー・z10〜z12の45段階をキャッシュなしで順に処理した。SQLite読み取り11.3 ms、圧縮データの読み込み・解析3.4 ms、画像レンダリング5,578.8 ms、原寸グレー画素への出力501.6 ms、Triangle縮小428.2 ms、合計6,523.3 msだった。GPU転送・合成・UI描画は含めない。粗い段階だけの値ではなく、原寸も含む45段階の合計。

高速化前は粗い段階でも1536×1536の出力を作ってから縮小していた。入力・細部の段階は制限するが、低解像度バッファへ直接出力はしていない。原寸画素出力と縮小で合計約14%、画像レンダリングが約86%だった。縮小処理だけを改善しても、今回の負荷の大部分は残る。

`YAMAMUKI_JXL_SQLITE`を指定し、`cargo test --locked --release --manifest-path viewer/Cargo.toml profiles_progressive_decode_steps -- --ignored --nocapture`で計測できる。

## jxl-rsへの切り替えと直接低解像度出力

2026-11-12、同じ5地点・3レイヤー・z10〜z12の45段階で、キャッシュなし・1スレッド・releaseビルドを比較した。SQLite読み取りは時間外、解析・復号・最終グレー画像への出力と必要な縮小を時間内とした。GPU転送・合成・UI描画は含めない。各行は15段階の合計で、下段は45段階の合計。

| 出力段階 | jxl-oxide 0.12.6＋縮小 | jxl-rs 0.8.0通常出力＋縮小 | 採用版：jxl-rs直接縮小 |
|---|---:|---:|---:|
| z10 | 2.172秒 | 0.753秒 | 0.675秒 |
| z11 | 2.369秒 | 0.957秒 | 0.739秒 |
| z12 | 2.091秒 | 0.562秒 | 0.557秒 |
| 合計 | 6.632秒 | 2.272秒 | 1.971秒 |

採用版は従来比約3.4倍。jxl-rsのSIMD復号への変更が主な改善であり、直接縮小だけで3.4倍になったという意味ではない。jxl-rs通常出力との比較では約13%短縮した。通常出力の比較にも同梱jxl-rsを使うが、行コールバックを使用しない元の出力経路を通す。計測はこのPCでの1回の45段階処理で、プラットフォーム全般の保証値ではない。

採用版はグループの1536×1536のu8出力ラスタを確保せず、グレースケール変換後の行をコールバックで受けてTriangle縮小する。z10は384×384、z11は768×768を作り、共有キャッシュから384×384のタイルを切り出す。入力全体を使う必要がある粗い段階も同じ低解像度出力を使う。z12は通常のSIMD出力を使用する。低解像度のfloat積算バッファと作業行は必要。内部の逆変換やフィルターをすべて低解像度で行う変更ではなく、内部処理には原寸の領域が残る。アプリ全体・デコーダー全体のピークメモリを測定した値ではない。

jxl-rs通常出力＋Triangle縮小とのRMSEはz10最大0.00261、z11最大0.00185／255。全国から別途分散抽出した86段階では最大画素差1／255だった。jxl-oxideとjxl-rsの出力差はRMSE最大0.50／255。富士山・槍ヶ岳・筑波山の27タイルは、公式libjxlの復号画像との比較でRMSE1未満を維持した。途中入力で単色画像のフレームが完了する場合も検査した。

jxl-oxideは比較テスト専用に移し、アプリではjxl-rsを使う。低解像度APIが上流にないため、jxl-rs 0.8.0をライセンス付きで同梱し、行出力に必要な4ファイルを変更した。libjxl（C++）は改修していない。変更範囲と更新手順は [同梱デコーダーの説明](../../../viewer/vendor/jxl/README.yamamuki.md) を参照。

~~~powershell
$env:YAMAMUKI_JXL_SQLITE = '全国JPEG XL SQLiteのパス'
$env:YAMAMUKI_JXL_PREVIEW = '公式libjxlの27枚のPNGを含むディレクトリ'
cargo test --locked --release --manifest-path viewer/Cargo.toml compares_decoders -- --ignored --nocapture
cargo test --locked --release --manifest-path viewer/Cargo.toml reads_national_groups -- --ignored
cargo test --locked --release --manifest-path viewer/Cargo.toml reduced_output_across_national_groups -- --ignored --nocapture
~~~

## 階層共有なしの独立JPEG XL・全国実測

2026-11-12、全国z5〜z12の67,050タイルをすべて独立画像として生成した。圧縮前PNGからlibjxl 0.13.0、VarDCT、distance 4、effort 7、1画像1スレッドで圧縮した。段階式の指定は使用せず、他タイル・他レベルと画像BLOBを共有しない。z5〜z9は512×512、z10〜z12は384×384。推定やサンプルからの外挿ではない。

| レイヤー | 従来WebPの画像合計 MiB | 独立JPEG XLの画像合計 MiB |
|---|---:|---:|
| forest | 90.06 | 133.52 |
| water | 15.13 | 26.82 |
| hillshade | 271.28 | 91.46 |
| 合計 | 376.47 | 251.80 |

画像合計264,028,161バイト、SQLite全体296,505,344バイト（282.77 MiB）。従来WebPのSQLite全体389.79 MiBより27.46%減少し、階層版183.19 MiBより54.36%増加した。画像以外に、1画像ごとの参照・誤差検査・ハッシュ・インデックスとSQLiteのページ空き領域を含む比較用DBの管理領域が約30.97 MiBある。正式配布用に最適化したDB容量ではない。

| レベル | 3レイヤーのタイル数 | 独立JPEG XL画像合計 MiB |
|---|---:|---:|
| z5 | 27 | 0.13 |
| z6 | 66 | 0.25 |
| z7 | 126 | 0.73 |
| z8 | 291 | 2.27 |
| z9 | 1,164 | 7.60 |
| z10 | 4,656 | 17.01 |
| z11 | 12,144 | 54.28 |
| z12 | 48,576 | 169.53 |

森林・水域はこのdistanceの独立JPEG XLより既存WebPの方が小さく、陰影の削減が全体の削減に寄与する。各方式は非可逆設定が異なり、容量だけで画質が同一とはみなさない。

5地点・3レイヤー・z10〜z12の45タイルについて、毎回ビューワの復号キャッシュを空にして読み込み、3回平均した。階層版1.947秒に対し独立版0.203秒、約9.6倍。z12だけでは15タイル合計0.581秒から0.065秒。どちらもjxl-rsを使用し、SQLite取得・復号・切り出し・RGBA化を含み、GPU転送とUI描画は含まない。OSのファイルキャッシュは制御していない。同じ階層グループ内の複数タイルをキャッシュ共有して読む速度の比較ではない。

全XYZを元SQLiteと照合して欠落0・追加0。全タイルで1画像1タイル、切り出し座標0、原寸とタイル寸法の一致、入力全体の使用を確認した。全BLOBの保存後ハッシュ・復号・SQLiteと参照の整合性を検査し、ビルド済みネイティブビューワで8レベルの読み込み・合成を確認した。GUI画面の目視検査はこの計測に含まない。

出力はbuild/surface/jxl-independent-d4-20261112/esa-worldcover-jxl-independent-d4.sqlite3。SHA-256: 60a9cf6f1bb69f1b45cb6935418ccf3615f8351cf1c3b53b786d8e92a775cb76。

比較用の専用application_idは階層版と同じで、configuration.formatはsurface-jxl-independent-experiment-v1。通常ビューワは両形式を識別できる。正式な共通SQLiteのRelease処理には未対応。

独立版はgenerate_jxlの最初に --independent を指定する。残りの引数は「配布SQLite、圧縮前packedSQLite、libjxlのbin、新しい出力先、並列数」。完了した出力先は上書きせず、中断した出力先は同じ入力・設定で再開できる。単色画像の重複圧縮は省くが、格納BLOBは各タイルに独立して保存する。

復号時間の比較はYAMAMUKI_JXL_SQLITEに階層版、YAMAMUKI_JXL_INDEPENDENTに独立版を指定して、cargo test --locked --release --manifest-path viewer/Cargo.toml compares_independent_tile_reads -- --ignored --nocapture で再実行できる。

## WebPと独立JPEG XLの復号速度比較

2026-11-12、現在のネイティブビューワが使用するRust実装で比較した。WebPはimage-webp 0.2.4、独立JPEG XLはjxl-rs 0.8.0。元データ・レベル構成が同じ全国SQLiteを使い、レイヤー×z5〜z12ごとに固定ハッシュ順で最大32タイルを抽出した。同一XYZ・同一画素数を検査し、計668タイルを5回測定。WebP版にあるPNG2枚は抽出候補から除外した。森林222、水域223、陰影223タイル。全67,050タイルを復号して測った全国総時間ではなく、全レベルに分散した標本の時間。

両デコーダーをタイマー外で暖機し、圧縮データをメモリに保持してから測る。形式を先に測る順序を回ごとに反転した。デコード単体は各実装の通常出力を作る時間で、WebPのRGB/RGBAとJPEG XLのグレーでは出力形式が異なるため、同じRGBA8画像を作る時間も別に測った。すべて単一の呼出スレッドで、GPU転送・合成・UI描画は含めない。

| レイヤー | 標本数 | WebPデコード ms/枚 | 独立JPEG XLデコード ms/枚 | JPEG XL / WebP |
|---|---:|---:|---:|---:|
| 森林 | 222 | 0.440 | 3.180 | 7.23倍 |
| 水域 | 223 | 0.370 | 2.848 | 7.69倍 |
| 陰影 | 223 | 1.740 | 2.808 | 1.61倍 |
| 全体 | 668 | 0.851 | 2.945 | 3.46倍 |

| 計測範囲 | WebP ms/枚 | 独立JPEG XL ms/枚 | JPEG XL / WebP |
|---|---:|---:|---:|
| デコード単体・通常出力 | 0.851 | 2.945 | 3.46倍 |
| デコード＋同じRGBA8出力 | 1.066 | 4.402 | 4.13倍 |
| SQLite取得〜RGBA8・復号キャッシュ空 | 1.321 | 4.714 | 3.57倍 |

最後の行は各タイルでアプリの復号キャッシュを作り直した読込経路。OSファイルキャッシュは暖まった状態で、ディスクの初回アクセス性能の比較ではない。WebPは通常のsurface_tile、JPEG XLは独立版のCache.tileを通す。SQLite込みのレイヤー別平均は、森林0.932対4.798 ms、水域0.738対4.693 ms、陰影2.291対4.650 ms。

z12だけのSQLite込み比較は、96タイル合計104.964対322.068 ms（1枚1.093対3.355 ms）で、独立JPEG XLが約3.07倍遅い。前節の「階層版より約9.6倍速い」は階層JPEG XLに対する改善であり、WebPより速いという意味ではない。

このPC・このビューワ実装では森林・水域はWebPが容量・速度の両方で有利。陰影は独立JPEG XLが容量を大きく削減できる一方、デコード単体で約1.61倍、読込全体で約2.03倍の時間が必要。異なる非可逆設定の現行データ同士の比較であり、画質が同じという前提ではない。他のWebP/JPEG XL実装や端末の速度を保証しない。

生のレイヤー・レベル別時間と5回分の総時間はbuild/surface/jxl-independent-d4-20261112/webp-decode-comparison.jsonに保存した。再現するにはYAMAMUKI_WEBP_SQLITEにWebP版、YAMAMUKI_JXL_INDEPENDENTに独立JPEG XL版、任意の出力先JSONをYAMAMUKI_CODEC_BENCH_REPORTに指定し、cargo test --locked --release --manifest-path viewer/Cargo.toml compares_webp_and_independent_jxl -- --ignored --nocapture を実行する。

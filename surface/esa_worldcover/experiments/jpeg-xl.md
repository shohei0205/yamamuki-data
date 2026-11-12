# JPEG XLによる地表圧縮の比較

2026-11-12に、配布形式を変えずに比較用Rustプログラムを実装し、実データを測定した。

## 条件

- libjxl 0.13.0、公式Windows x64配布物。CPUはIntel Core i7-12700。
- 富士山・槍ヶ岳・筑波山・東京都心・伊豆の5地点。各地点を含むz10区画を選び、z11の4枚とz12の16枚がそろっていることを確認する。
- 森林・水域・陰影の3レイヤー、合計315タイル。すべて384×384画素。
- 現行は森林・水域が可逆WebPまたはPNG、陰影がWebP品質89またはPNG。
- JPEG XLはeffort 7、1スレッド。森林・水域は可逆、陰影は可逆・distance 0.5・distance 1。
- 陰影は既存WebPを再圧縮せず、その生成で使った圧縮前のPNGから作る。原画に対する現行WebPとJPEG XLの誤差を両方測る。
- 復号はメモリ上の圧縮データから5回平均。プロセス起動・ファイルI/O・SQLite問い合わせ・GPU転送を含まない。現行はRustのimageクレート、JPEG XLは公式DLLを使うため、速度差にはデコーダー実装差も含まれる。
- 容量は画像BLOBの合計。SQLiteページ・索引・将来必要な階層索引の増減は含めない。全国全タイルの再圧縮、Android・iOS実機計測は未実施。

## 方式

1. 現行の21枚をそのまま使う。
2. 21枚をそれぞれ独立JPEG XLに変える。
3. z12の16枚を1536×1536にまとめ、JPEG XLの段階的復号で384相当・768相当・原寸を使う。

一括版では、段階式と通常の非段階式を両方測る。通常の一括版は容量比較の参考であり、z10・z11を少ない入力で読めることを意味しない。

## 結果

5地点を合計した画像BLOBの容量。KiBは1024バイト。

| レイヤー | 現行 | 独立JPEG XL | 一括・通常JPEG XL | 一括・段階的JPEG XL |
|---|---:|---:|---:|---:|
| 森林・可逆 | 1,097.1 KiB | 875.5 KiB（20.2%減） | 591.7 KiB | 1,753.3 KiB（59.8%増） |
| 水域・可逆 | 169.9 KiB | 122.4 KiB（28.0%減） | 83.0 KiB | 246.2 KiB（44.8%増） |
| 陰影・distance 1 | 1,945.1 KiB | 2,123.6 KiB（9.2%増） | 1,567.0 KiB | 1,617.6 KiB（16.8%減） |

陰影の原画に対するRMSEは現行WebPが1.63、独立JPEG XLのdistance 1が1.48。これは平均二乗誤差であり、同等の見た目を保証する基準ではない。distance 0.5の独立版はRMSE 0.96だが、容量は3,158.6 KiBまで増えた。可逆陰影はさらに大きくなる。

森林・水域を独立JPEG XL、陰影を段階式distance 1にする組み合わせは、3,289,182バイトから2,678,315バイトへ18.6%減った。ただし読み込み単位と復号負荷が変わる。5地点の比率から全国容量を断定しない。

### 可逆段階式の容量増加の切り分け

追加で富士山の同じ1536×1536画像について、通常設定にresponsive=1だけを指定して圧縮した。森林は143,773バイトから420,924バイト、水域は6,552バイトから16,842バイトへ増えた。完全な段階式指定ではそれぞれ422,797バイト・17,553バイトだった。

responsive設定だけでも増加の大部分が再現した。この設定は可逆ModularのSqueeze変換を有効にする。元の少数階調の画素を直接予測する方式と、粗い画像と段階別の差分に変換して圧縮する方式とでは、同じ情報でも圧縮モデルとの相性が異なる。今回のマスクでは後者が不利だったと考えられる。ただし内部の符号量を変換係数ごとには計測しておらず、値の種類の増加・予測効率などの寄与率までは分解していない。

JPEG XLの段階式が必ず増えるという結果ではない。また、低・中・高解像度の完全な画像を3枚そのまま重複保存したことによる増加でもない。

### 復号・メモリ

- 独立可逆森林105枚を復号する合計時間は、現行70.3 msに対してJPEG XLが620.3 ms。
- 水域105枚は現行52.7 msに対してJPEG XLが488.7 ms。
- 陰影をz10だけ表示する場合、5地点合計の現行入力は100,906バイト、段階式distance 1は433,362バイト。復号は14.4 msに対して130.1 ms。
- 陰影のz11だけでは入力397,198バイトに対して682,066バイト、復号51.2 msに対して145.8 ms。
- 陰影のz12を全16枚表示する場合は、現行1,493,646バイトに対して段階式1,656,457バイト。3レベル合計では減っても、あるレベルだけの読込量が減るとは限らない。
- 段階式陰影の復号器とグレー出力バッファの合計ピークは最大26.3 MiB。可逆一括マスクでは22.3 MiB。出力バッファ自体は粗い段階でも1536×1536＝2.25 MiB。
- メモリはlibjxlに渡したカスタムアロケーターで測定する。入力BLOB・リサイズ画像・DLL・GPUテクスチャ・プロセス全体の使用量は含めない。成功した各復号後に確保領域が全解放されることも検証する。

### 段階的復号の制約

非可逆陰影では、JXL_DEC_FRAME_PROGRESSIONイベントとJxlDecoderGetIntendedDownsamplingRatioで1/4・1/2の区切りを取得できた。その位置で入力を切っても同じ画像になることを全サンプルで検証した。

可逆のSqueezeを使った一括版は、今回のlibjxl/API設定では1/4・1/2のイベントを得られず、通常の測定では最終画像まで読んだ。入力を途中で切ってJxlDecoderFlushImageを呼ぶとプレビュー自体は得られる。富士山の森林では15通りの入力長のうち14通りで出力できたが、これを正確なz10・z11の境界とみなすことはできない。

比較画像ではAPIから得た1536画素の途中出力をTriangleフィルターで384・768画素へ縮小している。現在の粗レベルの面積平均と4ビット再量子化を完全再現する処理ではないため、粗レベルのマスク差はJPEG XLの可逆性が壊れたことを意味しない。原寸の可逆復号は全画素一致を検証している。

SQLiteのBLOBを単にJPEG XLへ置き換えるだけでは、レベル間の重複は減らない。一括版を採用するなら、地理的範囲・解像度・途中入力の区切りを表す索引、必要部分を読む格納方式、細タイルだけ要求する場合の扱いが必要になる。現行のsurface版1のencoding値へJPEG XLを追加する変更は今回行っていない。

## 判断

現行WebPを維持する。独立可逆JPEG XLの容量削減は確認できたが、今回の復号実装では約9倍遅い。一括段階式の森林・水域はむしろ大きい。陰影の段階式は合計容量を減らせる一方、粗レベルの読み込み量・復号時間・メモリの増加が大きい。

将来再検討する場合は、陰影だけの採用、別のデコーダー、少数の空間グループだけ復号する実装を候補にする。まずAndroid・iOS実機で容量と待ち時間の両方を検証し、利点が確認できてから配布形式を変更する。

## 再現方法

比較プログラムのコードはsurface/common/examplesに置く。正式な配布形式は変更しない。全国JPEG XLの比較用SQLiteは、ネイティブビューワの専用読み込み経路で表示できる。

Windowsでは[libjxl 0.13.0公式Release](https://github.com/libjxl/libjxl/releases/tag/v0.13.0)のjxl-x64-windows.zipを取得し、新しい作業ディレクトリへ展開する。binにcjxl.exeとjxl.dllがある動的リンク版を使う。ツールの配布物・入力SQLite・比較画像はコミットしない。

~~~powershell
cargo test --manifest-path surface/Cargo.toml -p yamamuki-data-sqlite --example compare_jxl --release --locked
cargo build --manifest-path surface/Cargo.toml -p yamamuki-data-sqlite --examples --release --locked

# 第4引数は、同じ全国生成から得たWebP圧縮前の中間SQLite。
# この中間形式ではhillshade_tilesのzが配布用z+1になっている。
& surface/target/release/examples/compare_jxl.exe `
    'build/surface/生成結果/esa-worldcover.sqlite3' `
    'build/tools/libjxl-0.13.0/x64-windows/bin' `
    'build/surface/jxl-comparison-new' `
    'build/surface/生成結果/packed/landcover-with-hillshade.sqlite3'

& surface/target/release/examples/report_jxl.exe 'build/surface/jxl-comparison-new'
~~~

第4引数を省くと、陰影も配布済み画像の復号結果が比較元になる。その場合は原画からの画質比較ではないことに注意する。出力先は必ず新しいディレクトリを指定する。入力SQLiteは読み取り専用で開く。

results.jsonに地点・XYZ・容量・速度・ピークメモリ・誤差・区切りを保存する。report_jxlがsummary.jsonと画像付きreport.htmlを作る。report.htmlはブラウザで直接開ける。

可逆途中表示の確認は、比較で作ったprogressive-d0.jxlを使って次のように実行する。

~~~powershell
& surface/target/release/examples/compare_jxl.exe --probe `
    'build/surface/jxl-comparison-new/fuji-forest/progressive-d0.jxl' `
    'build/tools/libjxl-0.13.0/x64-windows/bin' `
    'build/surface/jxl-prefix-probe-new'
~~~

## 森林・水域の非可逆段階式・追加比較

可逆だけでなく、森林・水域の非可逆圧縮も同じ5地点で測定した。通常のVarDCTをdistance 0.25・0.5・1・2で測定した後、VarDCTとModularの両方をdistance 0.5・1・2・4・8で追加測定した。すべてeffort 7・段階式・1スレッド。比較元のマスクは配布済み4ビット相当画像で、前節と同じ。5地点合計の現行容量は森林1,097.1 KiB、水域169.9 KiB。

| レイヤー | モード | distance | 合計容量 | 現行比 | z12 RMSE | z12の50%濃度境界の反転率 |
|---|---|---:|---:|---:|---:|---:|
| forest | VarDCT | 4 | 1009.1 KiB | -8.0% | 3.23 | 0.216% |
| forest | VarDCT | 8 | 698.0 KiB | -36.4% | 5.84 | 0.408% |
| forest | Modular | 4 | 717.8 KiB | -34.6% | 4.74 | 0.404% |
| forest | Modular | 8 | 479.0 KiB | -56.3% | 9.99 | 0.653% |
| water | VarDCT | 4 | 184.9 KiB | 8.8% | 0.90 | 0.018% |
| water | VarDCT | 8 | 133.3 KiB | -21.6% | 1.78 | 0.041% |
| water | Modular | 4 | 123.0 KiB | -27.6% | 1.50 | 0.034% |
| water | Modular | 8 | 88.6 KiB | -47.9% | 3.28 | 0.078% |

通常のVarDCTではdistance 0.25〜2の範囲で森林・水域とも現行より大きかった。distanceを4・8まで上げると削減が得られた。非可逆だから必ず可逆より小さくなるわけではない。VarDCTとModularで同じdistanceの画質も一致しないため、容量だけで選ばない。

VarDCTは全件で1/4・1/2の復号イベントを取得でき、入力をその位置で切って同じ出力になることも検証した。Modularは非可逆でも今回のAPI設定で同イベントが得られず、粗レベルも原寸まで復号した。Modularの数値を「粗い段階だけ読むことができる方式」として扱わない。

境界反転率は原画と復号後で濃度128以上の判定が変わった画素数を、画像全体の画素数で割った値。水域は画像全体に対する境界の割合が小さいので、画像全体の誤差だけで水際の品質を判断しない。原画ゼロの位置で濃度が17を超える画素率も記録した。VarDCT distance 8では森林0.623%、水域0.090%で、薄いにじみが加わっている。

原寸画像では東京湾の水際と富士山の森林を確認した。全体の形を大きく変えずに削減できる候補はあるが、拡大すると水際の軟化や細部の濃度変化がある。正式データを置き換えず、比較HTMLで許容する画質を確認する。

VarDCT distance 8でも、z10だけに必要な入力は森林225,716バイト、水域62,552バイト。現行の森林103,088バイト・水域21,742バイトより多い。全LOD合計の容量削減と、あるLODだけを見る場合の読み込み負荷は別に評価する必要がある。

### 追加比較の実行

compare_jxlは森林・水域もdistance 0.25〜2を比較するように拡張した。既存の可逆比較で作った原画をそのまま使い、非可逆モードだけ追加する場合は次を実行する。

~~~powershell
cargo build --manifest-path surface/Cargo.toml -p yamamuki-data-sqlite --examples --release --locked
& surface/target/release/examples/sweep_jxl.exe `
    'build/surface/jxl-comparison-new' `
    'build/tools/libjxl-0.13.0/x64-windows/bin' `
    'build/surface/jxl-mask-sweep-new'
& surface/target/release/examples/report_jxl.exe 'build/surface/jxl-mask-sweep-new'
~~~

sweep_jxlは保存済みのmosaic.pngとreference-z10〜z12.pngを入力として使う。原画を再生成せず、入力ファイルを変更しない。results.jsonには各モード・品質の実入力バイト数、実際の復号精細度、復号時間、メモリ、濃度誤差を保存する。report_jxlは結果を判別し、森林・水域の単独表示でVarDCTとModularを比べるHTMLを作る。

## 陰影の強い非可逆圧縮・追加比較

同じ5地点について、陰影もdistance 2・4・8へ圧縮を強めて比較した。既存WebPを再圧縮せず、その生成に使った圧縮前PNGからJPEG XLを生成する。effort 7・1スレッド・5回平均。現行WebPの3レベル合計は1,945.1 KiBで、原画に対するz12のRMSEは1.62。

| モード | distance | 3レベル相当の容量 | 現行比 | z12 RMSE | 段階的復号 |
|---|---:|---:|---:|---:|---|
| VarDCT | 2 | 939.0 KiB | -51.7% | 2.47 | 1/4・1/2で停止可能 |
| VarDCT | 4 | 509.7 KiB | -73.8% | 3.82 | 1/4・1/2で停止可能 |
| VarDCT | 8 | 266.2 KiB | -86.3% | 5.40 | 1/4・1/2で停止可能 |
| Modular | 2 | 796.8 KiB | -59.0% | 3.30 | 今回のAPIでは原寸まで復号 |
| Modular | 4 | 422.0 KiB | -78.3% | 4.80 | 今回のAPIでは原寸まで復号 |
| Modular | 8 | 223.7 KiB | -88.5% | 6.55 | 今回のAPIでは原寸まで復号 |

VarDCT distance 2では約52%、distance 4では約74%減った。陰影は森林・水域の分類マスクよりも圧縮を強めやすい候補だが、細かな起伏の明暗が弱まる。富士山の画像を現行と比較したところ、distance 4〜8でも大きな山体や尾根の形は見える一方、細かな陰影が滑らかになる。採用する画質は比較HTMLで確認する。陰影の濃度128をまたぐかどうかは分類を意味しないため、森林・水域用の境界反転率を品質の判断基準に使わない。

比較HTMLの現行欄は、配布済みWebPを復号してz12の16枚を並べた画像。JPEG XL側は圧縮前PNGを使う。粗い表示でも元の出力バッファは1536×1536で、メモリの制約は前節と同じ。例としてVarDCT distance 4のz10入力は5地点合計290,714バイトで、現行100,906バイトより多い。3レベル合計が減っても、粗レベルの読込量が減るとは限らない。

~~~powershell
& surface/target/release/examples/sweep_jxl.exe `
    'build/surface/jxl-comparison-new' `
    'build/tools/libjxl-0.13.0/x64-windows/bin' `
    'build/surface/jxl-shade-sweep-new' hillshade
& surface/target/release/examples/report_jxl.exe 'build/surface/jxl-shade-sweep-new'
~~~

省略可能な第4引数hillshadeで対象を陰影に限定する。指定しなければ森林・水域を比較する。レポートは対象レイヤーを結果から判別する。正式な配布SQLiteの形式は変更していない。全国JPEG XLの比較用SQLiteは、ネイティブビューワの専用読み込み経路で表示できる。

## 対応状況と参照資料

WebPはAndroidの標準画像形式で、iOS 14以降でも利用できる。JPEG XLはiOS 17以降のWebKitで対応しているが、段階的復号を使うAPIの可用性は別に確認する必要がある。Androidでは標準対応を前提にせず、アプリへのデコーダー組み込みを検討する。

- [Androidの標準画像形式](https://developer.android.com/media/platform/supported-formats)
- [GoogleのWebP対応説明](https://developers.google.com/speed/webp/faq)
- [AppleのSafari 17対応説明](https://webkit.org/blog/14445/webkit-features-in-safari-17-0/)
- [libjxlの復号API](https://libjxl.readthedocs.io/en/latest/api_decoder.html)
- [測定した版の復号ソース](https://github.com/libjxl/libjxl/blob/v0.13.0/lib/jxl/decode.cc)

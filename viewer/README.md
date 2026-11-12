# 山むき・データビューワ

[リポジトリの README に戻る](../README.md)

山むきの地点 JSON と、森林・水域・陰影などを収録した地表 SQLite を開いて重ねて表示できる、Windows と macOS（Apple Silicon）向けの Rust アプリ。地理院地図や OpenStreetMap 系のオンライン地図を下地に使える。地表データはオフラインでも表示できる。

## 起動

Rustのstableツールチェーンを用意する。WindowsではMSVCのビルドツール、macOSではXcode Command Line Toolsを用意する。macOSで未導入なら、ターミナルで `xcode-select --install` を実行する。

以下は両OSともリポジトリのルートから実行する。

```sh
cargo run --locked --release --manifest-path viewer/Cargo.toml
cargo run --locked --release --manifest-path viewer/Cargo.toml -- points/gsi-peaks/gsi-peaks-source.json
```

Windowsでは `viewer\run_win.bat` からも起動できる。起動のたびにCargoで変更を確認し、必要な部分をリリースビルドしてから実行ファイルを起動する。ファイルのパスを引数に渡すこともできる。macOSではターミナルから上記のコマンドで起動する。

「地点ファイルを開く」から `.json` または `.json.gz` を選ぶ。ファイルをウィンドウにドロップしても開ける。[地点データ版6](../points/schemas/data-v6.schema.json)と[版5の配列](../points/schemas/data-v5.schema.json)に対応する。左ドラッグで移動し、ホイールでカーソル位置を中心に拡大縮小する。右ドラッグの左右で地図を回転し、上下で最大80°まで傾ける。右上のコンパスは北を示し、クリックすると北が上の真上視点に戻る。地図をクリックすると、その位置にピンを置いて緯度・経度を表示する。ラベルの「コピー」で緯度・経度をカンマ区切りでクリップボードに入れられる。地点マーカーをクリックすると名称・座標・標高・IDを表示し、一覧の地点を選ぶ。近接する地点もそれぞれの座標に表示する。

左ペインの「レイヤー一覧」で、地理院地図の標準地図・淡色地図・白地図・写真・色別標高図・陰影起伏図・傾斜量図と、OpenStreetMap系の標準地図・地形図（OpenTopoMap）・自転車地図（CyclOSM）・人道支援地図（HOT）を切り替えられる。地図の提供ズームより拡大したときは、取得できる最も細かいタイルを拡大表示する。選んだ地図は次回起動時も使う。出典は [国土地理院（地理院タイル一覧）](https://maps.gsi.go.jp/development/ichiran.html) と [© OpenStreetMap contributors](https://www.openstreetmap.org/copyright)。

OpenStreetMap系では表示範囲のタイルを取得し、配信元のキャッシュ指定を使う。出典は各スタイルの表示中に画面下部に示す。[OSM標準地図の利用方針](https://operations.osmfoundation.org/policies/tiles/)、[OpenTopoMapのライセンス](https://opentopomap.org/about)、[CyclOSMとHOTの配信元の利用方針](https://www.openstreetmap.fr/usage/)を参照。

## 地表SQLite

「地表SQLiteを開く」から [山むきSQLiteコンテナ](../formats/sqlite.md) の地表データを選ぶ。ファイルのドロップや起動引数でも開ける。別の地表SQLiteを開くと一覧に追加され、既に開いている地表も表示したままになる。一覧にはデータセット名が出る。行のチェックでデータセット全体の表示を切り替える。`i` を押すとファイル名・パスと「全体」ボタンが出る。「閉じる」でそのファイルを一覧から外す。地表だけを表示する場合はタイルの取得にネット接続を使わない。一覧でオンライン地図を選ぶと、森林の地色を省いて、森林や水域がない場所にオンライン地図を表示する。地点データを同時に開いて重ねられる。

「レイヤー一覧」の「オンライン地図」にチェックを付け、配下の国土地理院またはOpenStreetMap系から下地を1種類選ぶ。地表のデータセットは開いた順に並び、後から開いたファイルを上に重ねる。各ファイルの中ではレイヤーの描画順に並べる。地表レイヤーは各行でON/OFFと自動または固定のLODを設定できる。固定LODの候補は各レイヤーの収録レベルから表示し、レベル名は実XYZズームに基づく `z10` などの表記に統一する。森林・水域・陰影の表示名は仕様に定めた `id` と `role` の組み合わせで判別し、汎用レイヤーは `id` を表示名に使う。オンライン地図と地表の自動LODは、256画素タイルに換算した画素密度を基準にする。タイルは表示に必要になった時点で読み込み、陰影を陸地に適用してから水域を合成する。オンライン地図に重ねると、下地の地図の色にも陰影を掛ける。地表単独表示で陰影だけを選ぶとグレースケールで表示する。ファイルに含まれる出典を画面下部に表示する。[地表画像のSQLite形式](../surface/formats/sqlite.md)

```powershell
viewer\run_win.bat 'build/surface/japan-0.1.0/esa-worldcover.sqlite3'
cargo run --locked --release --manifest-path viewer/Cargo.toml -- --check-surface 'build/surface/japan-0.1.0/esa-worldcover.sqlite3'
```

`--check-surface` はウィンドウを開かず、地表データセットと各レベルの代表タイルを読み込んで合成する。

## 地点一覧と地図

[Web版の地点ビューワ](../points/viewer.html)と同様に、地点名・読み・別名・IDで検索し、タグ、出典、取得元での分類、標高の出典はそれぞれ複数選択できる。同じタグ欄ではいずれかに一致し、検索語とタグ条件は両方に一致する地点を表示する。絞り込み対象外の地点は小さな灰色マーカーで残る。マーカーの大きさはズームと絞り込み後の件数で切り替わる。2万地点以上のファイルは、表示範囲に入る地点から最大1万件を個別表示する。検索や条件解除で地図の縮尺は変わらない。

一覧を選ぶと、その縮尺のまま地図の中心と詳細が切り替わる。一覧にフォーカスがあるときは上下キーでも移動できる。地図で選んだ地点が一覧にある場合はその行までスクロールする。一覧は番号、地点名、標高、所在地（参考）、緯度・経度、よみがな、別名、タグ、SVG、IDの10列。見えている行だけを描画し、列見出しから並び替えられる。横スクロールと列幅調整で右側の列を表示できる。

地点の詳細では出典、取得元での分類、標高の出典、関連ID、タグ、別名、SVG、元のJSONを表示する。「近くの山」は直線距離が近い20地点を表示し、選んだ地点へ移動できる。所在地は座標からの参考判定。行政区域は国土交通省の国土数値情報を共同通信社が加工した[行政区域データ](https://github.com/kyodo-official/japan-choropleth#行政区域)を取得する。

## 地点JSONの互換性

入力仕様は[地点データの形式](../points/formats/data.md)、[地点README](../points/README.md)、版5・版6のJSON Schemaに従う。ビューワは表示用にID・名称・緯度経度・標高・読み・別名・タグを取り出す。ほかの項目を含む配布ファイルも開ける。標高は数値・null・省略を扱う。読みは版5で文字列、版6で文字列または文字列の配列を扱い、両版ともnull・省略を扱う。複数の読みは入力順に表示する。別名は版5の文字列と版6の名前・読みを持つオブジェクトを扱う。

表示項目を変更するときは、スキーマの `$ref` が指す共通定義と地点READMEも確認する。Cargoテストはスキーマから表示項目の型の候補を組み立て、実際の読み込み処理に通す。リポジトリにある国土地理院の地点JSONも読む。共通CIでWindowsとmacOS（Apple Silicon）のリリースビルドとテストを実行する。Windowsではビルドした実行ファイルを起動し、国土地理院の地点JSONを読み込めることも確認する。

## 自動LOD

ツールバーの「自動LOD」でオンライン地図と地表に共通の上限と選択基準を設定できる。上限は実XYZの `z` をスライダーで指定する。数値をクリックすると直接入力できる。スライダーの最大値は、表示中のオンライン地図と地表レイヤーで利用できる最も細かいレベル。初期値はその最大値。上限が地表レイヤーの収録範囲より低い場合は、そのレイヤーの最も粗い収録レベルを表示する。選択基準のズーム補正は表示ズームに加える値で、+1で目標の画素密度が2倍、−1で半分になる。各レイヤーで選んだ固定LODは個別設定として優先する。自動LODの設定は終了時に保存する。LODの切り替えでは、新しいレベルの可視タイルを読み込んでから、約0.22秒でクロスフェードする。大きくズームアウトして旧レベルのタイルが画面内に密集する場合は、読み込み後に切り替える。

### JPEG XLの比較用SQLite

森林・水域・陰影をdistance 4で格納した[全国JPEG XLの比較用SQLite](../surface/esa_worldcover/experiments/jpeg-xl-sqlite.md)も、「地表SQLiteを開く」、ファイルのドロップ、起動引数から開ける。通常のWebP／PNGのSQLiteを同時に開いて比較できる。データセット・各レイヤーの表示、個別の固定レベル、自動LOD、クロスフェードは同じ操作で使う。

RustのJPEG XLデコーダーをアプリへ組み込んでいるため、Windows・macOSとも外部DLLやlibjxlのインストールは不要。jxl-rs 0.8.0のSIMD対応デコーダーを使い、目的レベルに必要な途中入力を復号して共有画像からタイルを切り出す。粗いレベルは行コールバックでTriangle縮小し、1536×1536の出力ラスタを確保せず384×384／768×768の画像を作る。中間段階が存在しない画像も、入力全体を読みながら同じ縮小出力を使う。デコーダー内部の逆変換・フィルターには原寸で処理する部分が残る。

z10〜z12で同じグループを使うタイルは、グループ・段階ごとの復号済み画像を64 MiBを目安とするキャッシュで共有する。2つの背景ワーカーが同じグループを要求しても重複して復号せず、異なるグループは並列に処理する。JPEG XLは1画像16,777,216サンプルを上限として受け付ける。これは内部バッファのバイト数を制限するものではない。これらはGPUテクスチャやアプリ全体のメモリ上限ではない。

タイルをすべて独立圧縮したJPEG XL版も同じ操作で開ける。独立版はz10〜z12それぞれ384×384の画像を復号し、周囲16枚の復号を待たない。レベル間のBLOB共有はない。

レイヤーの情報ボタンでは、レベルごとに必要な画像の入力量を重複なしで表示する。段階式はレベル間で同じBLOBを使うため、各レベルの数値を合計しても配布容量にはならない。独立版は各レベルの画像容量を合計できる。この比較用形式は正式な共通SQLiteの検査・Release公開処理には対応していない。

```powershell
viewer\run_win.bat build/surface/jxl-national-d4-20261112/esa-worldcover-jxl-d4.sqlite3
```

## 確認

```sh
cargo test --locked --manifest-path viewer/Cargo.toml
cargo run --locked --manifest-path viewer/Cargo.toml -- --check-points points/gsi-peaks/gsi-peaks-source.json
cargo run --locked --manifest-path viewer/Cargo.toml -- --check-map
```

`--check-map` は確認用の地理院タイルを1枚取得するため、ネット接続が必要。

## 配布版の公開

GitHub Actions の「データビューワ：下書き Release 生成・検査」を手動で実行する。Windows x64 と macOS Apple Silicon の ZIP を作り、それぞれの実行ファイルで地点 JSON を読み込んで確認する。

1. `main` で実行すると、生成日時から配布名とタグを自動で決め、Draft Release を作る。配布名は `YamamukiDataViewer-<UTC の生成日時>-<実行 ID>-<試行番号>`（例: `YamamukiDataViewer-20261112T000032Z-43123965055-1`）とし、日時以降の形式を地点データとそろえる。タグと ZIP 名の先頭には `viewer-` を付ける。バージョン番号の指定や更新は不要。作業ブランチでは Release を作らず、2 環境の ZIP を Actions の実行結果から 7 日間ダウンロードできる。
2. Draft の内容を確認し、「共通：Draft-release を試用公開」にそのタグまたは Release URL を指定する。2 環境の ZIP と `SHA256SUMS.txt` を検証し、同じ Release を Pre-release にする。
3. 試用して確認が済んだら、「共通：Pre-release を正式公開」に同じタグを指定する。添付ファイルを再検証して正式 Release にする。タグ・添付ファイル・取得 URL は変えない。ビューワの公開では Pages やデータカタログを更新しない。

配布先を選ぶ `channel` 入力は廃止し、生成時は常に下書きにする。次の配布も同じ Action を実行するだけで作れる。既存のタグや公開済み Release は上書きしない。

共通 Action の `sha256` は任意で、ビューワでは `SHA256SUMS.txt` 自体の SHA-256 を指定する。空欄でも、チェックサム一覧と両 ZIP の一致、ZIP 内の実行ファイルと破損の有無を検証する。公開時に ZIP の再生成・差し替えは行わない。

生成した Draft には整理用マーカーを付け、7 日以上経過した未公開の下書きは共通の整理 Action の対象になる。試用公開するとマーカーを外し、公開済みの Pre-release・正式版を保持する。長く保持する Draft は本文からマーカーを取り除く。

GitHub Releasesでは、ZIPと `SHA256SUMS.txt` を配布する。Windows版はZIPを展開して `yamamuki-viewer.exe` を起動する。macOS版はApple Silicon向けのZIPを展開して `YamamukiViewer.app` を開く。macOS版はアドホック署名で、Appleによる公証は行っていない。

Cargo.toml の `version` は Rust の内部情報として固定し、配布の識別には使わない。macOS の内部の版は UTC の生成日から自動設定する。既存の `viewer-v<版>` の下書きも、引き続き試用公開・正式公開できる。

JPEG XLの実データ検査は、`YAMAMUKI_JXL_SQLITE` に全国SQLite、`YAMAMUKI_JXL_PREVIEW` に公式libjxlで出力した27枚のPNGを含むディレクトリ、`YAMAMUKI_WEBP_SQLITE` に比較元WebP版、`YAMAMUKI_JXL_INDEPENDENT` に独立JPEG XL版を指定し、`cargo test --locked --release --manifest-path viewer/Cargo.toml jxl -- --include-ignored` で実行する。通常のテストでは実データを必要とする検査を除外し、参照範囲・共有キャッシュ・キャッシュ容量と行縮小の検査を実行する。

復号速度の比較は、同じSQLiteを指定して `cargo test --locked --release --manifest-path viewer/Cargo.toml compares_decoders -- --ignored --nocapture` で実行する。jxl-oxide、jxl-rs通常出力＋縮小、jxl-rs直接縮小の順に、5地点・3レイヤー・z10〜z12の45段階を比較する。jxl-oxideは比較テスト専用の依存で、アプリには組み込まない。行出力の変更箇所は [同梱デコーダーの説明](vendor/jxl/README.yamamuki.md) を参照。

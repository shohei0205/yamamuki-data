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

## 確認

```sh
cargo test --locked --manifest-path viewer/Cargo.toml
cargo run --locked --manifest-path viewer/Cargo.toml -- --check-points points/gsi-peaks/gsi-peaks-source.json
cargo run --locked --manifest-path viewer/Cargo.toml -- --check-map
```

`--check-map` は確認用の地理院タイルを1枚取得するため、ネット接続が必要。

## 配布版の公開

GitHub Actions の「データビューワ：Release」を手動で実行する。Windows x64とmacOS Apple SiliconのZIPを作り、それぞれの実行ファイルで地点JSONを読み込んで確認する。

- main では、Cargo.toml の版を使った `viewer-v<版>` のタグで公開する。次の正式版を作るときは Cargo.toml の版と Cargo.lock を更新する。
- dev では、viewer-v<版>-dev.<実行ID> のタグでプレリリースを公開する。
- 作業ブランチでは、2環境のZIPをActionsの実行結果から7日間ダウンロードできる。

GitHub Releasesでは、ZIPと `SHA256SUMS.txt` を配布する。Windows版はZIPを展開して `yamamuki-viewer.exe` を起動する。macOS版はApple Silicon向けのZIPを展開して `YamamukiViewer.app` を開く。macOS版はアドホック署名で、Appleによる公証は行っていない。

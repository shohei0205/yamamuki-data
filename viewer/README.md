# ネイティブ地点ビューワ

国土地理院の標準地図に、山むきの地点JSONを重ねる。WindowsとmacOS（Apple Silicon）を対応対象とするRustアプリ。地図は現在の表示範囲のタイルをオンラインで取得する。地点ファイルはローカルで読む。

## 起動

Rustのstableツールチェーンを用意する。WindowsではMSVCのビルドツール、macOSではXcode Command Line Toolsを用意する。macOSで未導入なら、ターミナルで `xcode-select --install` を実行する。

以下は両OSともリポジトリのルートから実行する。

```sh
cargo run --locked --release --manifest-path viewer/Cargo.toml
cargo run --locked --release --manifest-path viewer/Cargo.toml -- points/gsi-peaks/gsi-peaks-source.json
```

Windowsでは `viewer\run.bat` からも起動できる。作成済みの `viewer\target\release\yamamuki-viewer.exe` があれば起動し、実行ファイルがなければCargoでビルドしてから起動する。ファイルのパスを引数に渡すこともできる。macOSではターミナルから上記のコマンドで起動する。

「地点ファイルを開く」から `.json` または `.json.gz` を選ぶ。ファイルをウィンドウにドロップしても開ける。[地点データ版6](../points/schemas/data-v6.schema.json)と[版5の配列](../points/schemas/data-v5.schema.json)に対応する。左ドラッグで移動し、ホイールでカーソル位置を中心に拡大縮小する。マーカーをクリックすると名称・読み・座標・標高・IDを表示する。近接する地点もそれぞれの座標に表示する。

地図の出典は [国土地理院（地理院タイル一覧）](https://maps.gsi.go.jp/development/ichiran.html)。

## 地点一覧と地図

[Web版の地点ビューワ](../points/viewer.html)と同様に、地点名・読み・別名・IDで検索し、タグ、出典、取得元での分類、標高の出典はそれぞれ複数選択できる。同じタグ欄ではいずれかに一致し、検索語とタグ条件は両方に一致する地点を表示する。絞り込み対象外の地点は小さな灰色マーカーで残る。マーカーの大きさはズームと絞り込み後の件数で切り替わる。2万地点以上のファイルは、表示範囲に入る地点から最大1万件を個別表示する。検索や条件解除で地図の縮尺は変わらない。

一覧を選ぶと、その縮尺のまま地図の中心と詳細が切り替わる。一覧にフォーカスがあるときは上下キーでも移動できる。地図で選んだ地点が一覧にある場合はその行までスクロールする。一覧は番号、地点名、標高、所在地（参考）、緯度・経度、よみがな、別名、タグ、SVG、IDの10列。見えている行だけを描画し、列見出しから並び替えられる。横スクロールと列幅調整で右側の列を表示できる。

地点の詳細では出典、取得元での分類、標高の出典、関連ID、タグ、別名、SVG、元のJSONを表示する。「近くの山」は直線距離が近い20地点を表示し、選んだ地点へ移動できる。所在地は座標からの参考判定。行政区域は国土交通省の国土数値情報を共同通信社が加工した[行政区域データ](https://github.com/kyodo-official/japan-choropleth#行政区域)を取得する。

## 地点JSONの互換性

入力仕様は[地点データの形式](../points/formats/data.md)、[地点README](../points/README.md)、版5・版6のJSON Schemaに従う。ビューワは表示用にID・名称・緯度経度・標高・読み・別名・タグを取り出す。ほかの項目を含む配布ファイルも開ける。標高は数値・null・省略を扱う。読みは版5で文字列、版6で文字列または文字列の配列を扱い、両版ともnull・省略を扱う。複数の読みは入力順に表示する。別名は版5の文字列と版6の名前・読みを持つオブジェクトを扱う。

表示項目を変更するときは、スキーマの `$ref` が指す共通定義と地点READMEも確認する。Cargoテストはスキーマから表示項目の型の候補を組み立て、実際の読み込み処理に通す。リポジトリにある国土地理院の地点JSONも読む。共通CIでWindowsとmacOS（Apple Silicon）のリリースビルドとテストを実行する。Windowsではビルドした実行ファイルを起動し、国土地理院の地点JSONを読み込めることも確認する。

## 確認

```sh
cargo test --locked --manifest-path viewer/Cargo.toml
cargo run --locked --manifest-path viewer/Cargo.toml -- --check-points points/gsi-peaks/gsi-peaks-source.json
cargo run --locked --manifest-path viewer/Cargo.toml -- --check-map
```

`--check-map` は確認用の地理院タイルを1枚取得するため、ネット接続が必要。

## 配布版の公開

GitHub Actions の「地点 / ネイティブビューワ：Release」を手動で実行する。Windows x64とmacOS Apple SiliconのZIPを作り、それぞれの実行ファイルで地点JSONを読み込んで確認する。

- main では、Cargo.toml の版を使った `viewer-v<版>` のタグで公開する。次の正式版を作るときは Cargo.toml の版と Cargo.lock を更新する。
- dev では、viewer-v<版>-dev.<実行ID> のタグでプレリリースを公開する。
- 作業ブランチでは、2環境のZIPをActionsの実行結果から7日間ダウンロードできる。

GitHub Releasesでは、ZIPと `SHA256SUMS.txt` を配布する。Windows版はZIPを展開して `yamamuki-viewer.exe` を起動する。macOS版はApple Silicon向けのZIPを展開して `YamamukiViewer.app` を開く。macOS版はアドホック署名で、Appleによる公証は行っていない。

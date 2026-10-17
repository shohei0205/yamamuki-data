# 国土地理院の山頂データ（gsi-peaks）

[地点データの共通仕様に戻る](../README.md)

国土地理院の公式データから、山名・読み・位置・標高を取得し、地点データの版6で出力する。データセット名は `gsi-peaks`。

この文書の構成：[配るもの](#配るもの)・[置き場所](#置き場所)・[作り方](#作り方)・[地点の補足対応表](#地点の補足対応表)・[出典とライセンス](#出典とライセンス)・[テスト](#テスト)

## 配るもの

全国の山頂データを `gsi-peaks.json.gz` 一つにまとめる。山の一覧・表示名・座標は最適化ベクトルタイルを第一ソースとし、山岳一覧・自然地名から未収録の山と不足する情報を補う。収録する地点の `type` は `peak` とする。全国の全山頂を網羅した一覧ではない。

地点 JSON の項目・型・共通の制約と版の履歴は [地点データフォーマット](../formats/data.md) を参照する。配列の並び順を前提にせず、ID で地点を見分ける。

山名・よみがな・別名の照合と、山頂座標・標高の優先順位は「[名前・位置・ID の照合](#名前位置id-の照合)」に記載する。読みを推測で補わず、確認できない場合は `null` とする。オリジナルの Wikipedia・Wikidata のリンクは `null` とする。配布時には GSI 用の `supplements.json` により、`日本百名山` タグと山名・読み・別名の補正、配布対象からの除外を適用する。補足の出典と加工内容は補足 JSON の `metadata.sources` に記録する。注記位置・自然地名の代表点で取得した DEM の地表標高は、公称の山頂標高と区別する。

### 名前・位置・ID の照合

ズーム 16 の `Anno` レイヤーから分類コード `312`（山、岳、峰等）、`313`（尖峰、丘、塚等）、`810`（ズーム 17 用の山）を採用する。ズーム 17 の情報もズーム 16 に含まれる。山の総称 `311` は山頂として追加しない。タイル外の複製注記を除き、同名で 10 m 以内の通常表示用・拡大表示用の注記をまとめる。単一点ではない注記は report に残す。

公式 CSV の山名に実際の `?` が含まれる 4 件は、索引番号と補正前の名前が一致する場合だけ、公式ベクトルタイルで確認した表記へ補正する。対象は `199`（薬萊山）、`204-3`（屛風岳）、`226`（朳差岳）、`662`（屛風山）。元 CSV は保持し、未確認の文字を推測で置き換えない。

照合時は空白・全半角と、槇／槙・峯／峰・嶽／岳・ヶ／ケ・舘／館・寶／宝をそろえる。表示名はベクトルタイルの表記を使う。

山岳一覧は、名前・別名が一致する 1 km 以内の地点を探し、注記と山岳一覧が一対一に対応する場合だけ、山頂座標・公称標高と `gsi-mountain-list:<索引番号>` を優先する。地図注記と実際の山頂は数百 m 離れる例があるため、自然地名より広い範囲で照合する。表示名と山岳一覧の名前が同じ場合は、山岳一覧の読みを優先する。別名だけが一致した場合に、元の表示名の読みを別名へ流用しない。

それ以外の地点は、ベクトルタイルの名称と注記座標を使い、ID は `gsi-vector:` に名称と座標のハッシュを付ける。自然地名が一致してもその代表点へ座標を戻さない。自然地名の読み・別名は、名前・別名と 100 m 以内の位置で照合する。「岩・洞窟」など山以外の種別も読みの照合対象とするが、それ自体を新しい山として追加しない。100 m 以内に読みがない場合は、1 km 以内で同名の自然地名が一つに決まり、その自然地名から見ても該当する山名注記が一つだけの場合に限り読みを採用する。この広い照合で座標を自然地名へ変更したり、別名を追加したりはしない。資料間の読みの相違や対応のあいまいさは `vector-comparison.json` に残す。推測した読みは使わず、確認できない読みは `null`。

ベクトルタイルにない山を追加するときも、同名・別名の近接地点があれば二重登録を避ける。100 m 以内に一つだけ対応する地点があれば既存の地点を使い、複数候補や 100～1000 m の同名地点は確認候補に回す。自然地名だけから追加した地点は `gsi-natural:` の ID を使う。ID の継続利用は、次の維持方針に従う。

元データと補足資料の出典は地点 JSON の `metadata.sources` と manifest の `metadata.sources` に保持する。`gsi-peaks` は主資料、`gsi-mountain-list` は山岳一覧、`gsi-natural` は自然地名、`gsi-dem` は標高タイルを表す。取得ファイルごとの記録は生成時 manifest の `sourceFiles` に分けて保存する。

### 変換の規則

- 山岳一覧の `＜山頂名＞` がある行は、その山頂名と読みを使う。山全体の名前を山頂の別名にはしない。
- `（別称）` は表示名と分け、別名へ入れる。地図に山名が表示されていないことを示す角括弧は表示名から外す。
- 自然地名にコンマで複数の読みが併記されている場合、先頭を表示用に使う。全国取得の report の `readingVariants` と元タイルには全候補を残す。
- 読みはひらがなへそろえる。漢字やローマ字から推定しない。資料の括弧の対応が変わった場合はエラーにする。
- 山岳一覧の ID は `gsi-mountain-list:<索引番号>`。自然地名の ID は山名と座標から作る。OSM ID は出力しない。OSM 版からの ID の引き継ぎや、補足・お気に入りなどの移行は含まない。山名・座標の更新で自然地名の ID は変わる。
- 自然地名と山岳一覧の表示名が完全一致し、100 m 以内で一意に対応する場合は、一つの地点にまとめて山岳一覧の位置・標高・読みを優先する。読みの相違はレポートに記録する。同名でも位置が離れていれば別の地点として残す。
- 自然地名の外字は復元を推測せず、その地点を除外してレポートに記録する。
- 自然地名には標高がない。範囲指定の生成では `--dem` を付けた場合、全国全取得では常に、その代表点の地表標高を標高タイルから補う。これは公称の山頂標高とは区別し、補った件数をレポートに記録する。タイルや値が欠ける場合は標高を `null` にする。
- オリジナルの Wikipedia・Wikidata リンクは `null`。`日本百名山` タグと名前・読み・別名の補正、配布除外は GSI 用の `supplements.json` で適用する。

### 地点ごとの出典情報

`provenance.dataSource` は `mountainList`（山岳一覧）、`vector`（ベクトルタイル）、`natural`（自然地名）。山岳一覧と一意に対応する地点は `mountainList` とする。

`classification` は対応するベクトルタイルの注記分類コードを文字列で記録する。対応する注記がない山岳一覧の地点は `peak`、自然地名は `山（自然地名）` とする。コードは [国土地理院の注記分類コード](https://maps.gsi.go.jp/help/pdf/vector/optbv_featurecodes.pdf)を参照する。

`elevationSource` は `mountainList`、`triangulation`（三角点）、`spotHeight`（標高点）、`dem1a_png`、`dem5a_png`、`dem5b_png`、`dem5c_png`、`dem_png`（DEM10B）のいずれか。標高が取得できない地点では `elevationSource` を省略する。補足で標高を補正した場合は `supplement`（補足による補正）にし、削除した場合は省略する。

<a id="id-の維持方針"></a>

### ID の作り方と維持方針

[地点データの共通仕様](../formats/data.md#id-とデータセット)に従い、`gsi-peaks` でも ID の維持は努力義務とし、永続性を保証しない。

`gsi-vector:` と `gsi-natural:` は名称と小数 7 桁にそろえた座標のハッシュから作る。同じ名称・座標ならハッシュは同じになるが、国土地理院が名称や座標を修正すると、同じ山でも ID が変わり得る。前回の生成結果との照合による ID の引き継ぎは行わない。山岳一覧の `gsi-mountain-list:` は索引番号を使うため、索引番号が同じなら名称・座標の変更では ID を変えない。

利用側でお気に入りや補足を ID にひも付ける場合は、データ更新後も参照先があるか確認する。以前の ID に対応する地点がなくなった場合の再関連付けは、利用側で必要に応じて行う。

### コミット済み全国版の内容

2026 年 10 月 14 日の補足適用前の生成結果を `gsi-peaks-source.json` と `snapshot.json` に保存している。補足適用前の件数は次のとおり。

| 項目 | 件数 |
|---|---:|
| 地点 | 17,248 |
| よみがなあり | 17,030 |
| 標高あり | 17,248 |
| 山岳一覧の標高 | 1,059 |
| 三角点・標高点の標高 | 14,931 |
| DEM1A の地表標高 | 986 |
| DEM5A/B/C の地表標高 | 216 |
| DEM10B の地表標高 | 56 |
| 山岳一覧の索引 ID を使う地点 | 1,059 |
| ベクトルタイルの山名注記（複製を統合済み） | 17,199 |
| 山岳一覧・自然地名から追加した未収録の山 | 49 |

この生成結果では、山の周囲の自然地名は 23,896 タイルを対象とする。全国の保存済みタイルを含めた対象は 67,243 タイルで、200 応答が 60,735 件、404 応答が 6,508 件。保存済み応答を使っているため、保存済みデータからの再生成では、自然地名への通信は 0 件。元資料の取得日時とハッシュは `snapshot.json`、照合の確認候補と読みの相違はローカルの `vector-comparison.json` に記録する。

宝篋山は山名注記から収録し、周囲の三角点の標高 460.7 m を採用する。よみがなと別名「小田山」は参照した資料に含まれないため未設定。長七ノ頭は山名注記の位置を使い、自然地名の「岩・洞窟」に記載された「ちょうしちのあたま」を照合して読みを補っている。

## 置き場所

[GitHub Releases](https://github.com/shohei0205/yamamuki-data/releases) に、次のファイルを添付する。

| ファイル | 内容 |
|---|---|
| `gsi-peaks.json.gz` | アプリが取得する地点データ。manifest の `downloadUrl` から取得する |
| `manifest.json` | 地点データの取得先・サイズ・ハッシュ・件数・出典 |
| `gsi-peaks.json` | 同じ地点配列の非圧縮 JSON。確認用 |
| `snapshot.json` | コミット済みオリジナル JSON のハッシュと、元資料の取得日時・出典の要約 |

正式版は `gsi-peaks-<version>`、開発版は `gsi-peaks-dev-<version>` の Release に置く。利用する版を選び、その Release の `manifest.json` の URL を固定して取得する。リポジトリ全体の `releases/latest` は使わない。GSI の公開 Action は Pages の manifest・履歴・地点カタログを更新しない。

manifest の項目・型・取得と検証の共通手順は [manifest フォーマット](../../formats/manifest.md) を参照する。GSI 固有の元資料の取得・照合と加工方法は、この文書に記載する。

### アプリ用データと確認・編集用の添付

補足適用前の `gsi-peaks-source.json` をコミットして保持する。マージ済みの `gsi-peaks.json` は Action で補足を適用して生成する。Release には、マージ済みの `gsi-peaks.json.gz`、オリジナルの `gsi-peaks-source.json.gz`、補足の `supplements.json.gz` と、それぞれの `manifest.json`・`gsi-peaks-source-manifest.json`・`supplements-manifest.json` を配布する。各 manifest は共通形式を使う。公開前にオリジナルへ補足を適用し、配布地点を再現できることを確認する。

## 作り方

配布データには GSI 用の `supplements.json` を適用する。

通常の全国版はベクトルタイルを第一ソースに生成する。山岳一覧・自然地名だけの試行と、自然地名の全取得は用途に応じて使う。

### ベクトルタイルを第一ソースに全国版を生成する

全国版の山の一覧と表示名・座標は、[国土地理院最適化ベクトルタイル](https://github.com/gsi-cyberjapan/optimal_bvmap)を第一ソースとする。よみがな・別名は山岳一覧と自然地名、公称の山頂座標・標高は山岳一覧で補う。標高は山岳一覧を優先し、未設定の地点は周囲 25 m の三角点・標高点の注記を照合する。候補が一つで、他の山と候補を共有しない場合だけ採用する。それでも標高がない地点は、その出力座標で DEM1A → DEM5A → DEM5B → DEM5C → DEM10B の順に PNG 標高タイルを参照する。404 や欠測値の場合は次の資料を使い、すべて欠ける場合は `null` とする。ベクトルタイルにない山は山岳一覧と取得済み自然地名から追加する。

PMTiles は全国の地図を一つのファイルで取得し、山名と山の周囲の標高注記を抽出する。保存先は Git の対象外の `build/gsi-vector-cache/` とし、取得後も元ファイルを保持する。標高注記の抽出結果は出力先の `height-annotations.sqlite3` に保存する。DEM の元 PNG と 404 の応答記録は `build/gsi-cache/` に保持し、取得済みタイルを再利用する。同時取得数は最大 4 とする。`--offline` を指定すると通信せず、必要なキャッシュが欠けていれば停止する。

```bash
python -m pip install -r points/gsi-peaks/requirements-vector.txt
python points/gsi-peaks/download_vector.py --cache build/gsi-vector-cache
python points/gsi-peaks/vector.py \
  --archive build/gsi-vector-cache/optimal_bvmap-v1.pmtiles \
  --cache build/gsi-cache --output build/gsi-national-primary
```

全国の自然地名を保存した SQLite を利用する場合は、次の指定も加える。元の SQLite は読み取り専用で開き、存在確認済みのタイルを全件再利用する。保存した自然地名に含まれる、ベクトルタイル未収録の山も補完対象になる。

```bash
--natural-database build/gsi-national-all/natural-tiles.sqlite3
```

抽出済みのベクトルタイルがある場合は、`--extraction-cache build/gsi-national-vector/vector-annotations.sqlite3` も指定すると山名の抽出を再利用できる。この指定がない場合は出力先の `vector-annotations.sqlite3` を使う。

### 標高の補完

全国版では DEM1A はズーム 17、DEM5A/B/C はズーム 15、DEM10B はズーム 14 のPNGタイルを使う。採用した標高の出典は `elevation-sources.json` に記録する。

三角点・標高点の標高も山の最高地点の標高とは限らず、DEMは指定座標の地表標高を表す。

範囲指定の `generate.py --dem` と `fetch_all.py` は、2024 年 10 月以降更新されていない `dem` のテキスト形式を使う。

### 自然地名の取得範囲とキャッシュ

自然地名は、ベクトルタイルの山と山岳一覧の各地点から、周囲 100 m にかかる自然地名のズーム 15 のタイルだけを選ぶ。タイル境界をまたぐ場合は隣接タイルも含め、同じタイルは一度だけ取得する。

100 m 以内で読みが見つからない地点だけ、周囲 1 km のタイルを追加で探す。完全な全国 SQLite の存在確認済みタイルを読み込んでいる場合は、追加の通信は不要。

全国の自然地名を保存した SQLite を指定すると、山の周囲に加えて、存在確認済みのタイルも取得対象に含める。保存済みの 200・404 応答を再利用し、取得記録のないタイルだけを問い合わせる。

選択した元 GeoJSON の圧縮バイト列と、404 を含む URL・取得日時・ハッシュ・応答情報は、出力先の `natural-selected.sqlite3` に保存する。全取得データを取り込んだ記録と、存在するタイルの一覧も保持するため、取り込み後は `--natural-database` を外してもその選択 DB だけで同じ範囲を再利用できる。取り込み元の全国 DB はバックアップとして残す。取得済み応答は次回も使い、中断後は未取得分だけを取得する。403 や通信失敗を 404 として保存しない。誤った広範囲取得を避けるため、選択数は既定で 100,000 タイルまでとし、`--max-natural-tiles` で変更できる。

全国の自然地名の SQLite を指定せず、山の周囲だけを取得した場合、自然地名だけにありベクトルタイルから離れた山まで網羅したとは扱わない。補完する山の範囲は、取得した自然地名と山岳一覧の範囲になる。report の `naturalAcquisition` に選択数、通信数、404 数、全取得データを再利用したかを記録する。自然地名を指定範囲で全取得する場合は、`fetch_all.py` を使う。

### オフライン再生成と配布

初回の取得と標高補完が完了したら、同じコマンドに `--offline` を付けて通信せず再生成できる。再生成には PMTiles・取得記録、抽出 DB、選択自然地名 DB、山岳一覧・標高の `gsi-cache/` を保管する。別の版の元ファイルを使う場合は、版ごとに元ファイルと抽出 DB の保存先を分ける。

`vector-only-points.json` はベクトルタイルだけから抽出した比較用の地点配列で、よみがなと標高は未補完の `null`。配布には補完後の `gsi-peaks.json` を使う。

```bash
python -m release_tools.gsi_peaks snapshot --source build/gsi-national-primary
```

Release の Action はコミット済みの地点 JSON と出典要約を使う。PMTiles や SQLite はローカルに保管する。

### 元データの保管と再生成

元資料の資料日や取得日時は `snapshot.json` と元データの取得記録を参照する。自然地名タイルは実験提供のため、取得したファイルをキャッシュとハッシュで固定する。生成日時や取得日時を元データの編集日時として記録しない。

取得した元データは生成後も削除しない。自然地名の GeoJSON は `build/gsi-national-all/natural-tiles.sqlite3` の `tiles.body` に gzip で可逆圧縮して保存する。解凍すると取得時の元のバイト列に戻り、SHA-256 で一致を確認できる。山岳一覧の CSV と標高タイルの元データは `build/gsi-cache/` の `.data` に保存する。両方の保存先を保管すると、取得時と同じ入力から再生成できる。これらは Git の管理対象外。

404 は URL を区画番号から復元し、同じ取得日時・応答ヘッダーなどを `missing_metadata` テーブルに一度だけ保存する。`tiles.metadata` は、200 では取得記録の JSON、404 では共通記録の番号を格納する。`fetch_all.restore_metadata` は、共通記録の番号とタイルごとの JSON の両方を読み、URL を含む取得記録を復元する。GeoJSON 本体は変更しない。

404 の取得記録をタイルごとに JSON で保存した SQLite も読み込める。その記録を共通化して容量を減らす場合は、取得・生成を停止し、次のコマンドで別ファイルへ変換する。

```bash
python points/gsi-peaks/compact_cache.py build/gsi-national-all/natural-tiles.sqlite3 build/gsi-national-all/natural-tiles.compact.sqlite3
```

変換処理は全区画の GeoJSON の圧縮バイト列と、URL・取得日時などの記録が元と一致することを確認する。成功したあと、変換後のファイルを元の `natural-tiles.sqlite3` と入れ替える。元ファイルは変換処理では変更しない。変換が失敗した場合は出力先の途中ファイルを削除してから再実行する。

全取得と最初の生成が完了したあと、次のコマンドで通信せずに再生成できる。必要な元データが欠けている場合は停止し、オンラインで別のデータを混ぜない。

```bash
python points/gsi-peaks/fetch_all.py --regenerate-only
```

取得中にバックアップする場合は、SQLite のバックアップ機能を使う。稼働中の `.sqlite3` だけをコピーすると WAL に残っている取得結果が抜けるため、通常のファイルコピーは取得プロセスの停止後に行う。`build/gsi-cache/` も一緒に保管する。

### 生成済み JSON をコミットして Release に公開する

全国の元データ取得・地点生成は手元で行う。Git には補足適用前の `points/gsi-peaks/gsi-peaks-source.json`、補足の `supplements.json`、出典・取得日時の範囲・オリジナル JSON の SHA-256 を記録した `snapshot.json` を保存する。マージ済みの `gsi-peaks.json` は配布時に生成し、Git に含めない。SQLite、`gsi-cache/`、gzip、全タイルの明細を含む manifest・report はコミットしない。

生成後、次のコマンドで地点 JSON と小さな出典要約を取り込む。

```bash
python -m release_tools.gsi_peaks snapshot --source build/gsi-national-primary
git add points/gsi-peaks/gsi-peaks-source.json points/gsi-peaks/supplements.json points/gsi-peaks/snapshot.json
```

オリジナルの地点 JSON と `snapshot.json` を同じコミットで更新する。地点 JSON だけを編集すると SHA-256 が一致せず、公開処理は停止する。元データは手元で保管し、JSON の取り込み後も削除しない。

取り込んだ変更を `dev` または `main` にマージしたあと、Actions の「地点 / 国土地理院山頂：JSON を Release に公開」から「Run workflow」で実行元ブランチを選ぶ。元データの取得・山頂の再抽出・標高補完は行わず、コミット済みの JSON を検証して gzip を作る。実行元が `dev` なら `gsi-peaks-dev-<version>` のプレリリース、`main` なら `gsi-peaks-<version>` の Release を作る。版には UTC 日時・実行番号・再実行番号を使う。

Release には「[アプリ用データと確認・編集用の添付](#アプリ用データと確認編集用の添付)」のファイルを添付する。下書きにアップロードしたあと、全ファイルを取り直してバイト列の一致を確認してから公開する。既存の Release は差し替えず、リポジトリ全体の Latest にも設定しない。この Action では Pages の manifest や地点カタログは更新しない。

公開用ファイルの準備だけを手元で確認する場合は、次を実行する。外部通信は行わない。

```bash
python -m release_tools.gsi_peaks prepare --channel dev --version local-test
```

既定の出力先は `build/gsi-release/`。公開処理のテストは共通 CI でも実行する。

```bash
python -m unittest discover -s release_tools/tests -v
```

### 山岳一覧・自然地名だけを試す

通常の全国版は、後述の「ベクトルタイルを第一ソースに全国版を生成する」の手順を使う。ここでは、山岳一覧だけの出力や小範囲での自然地名の取得を試せる。

既定では「日本の主な山岳標高一覧（1003山）」の 2026 年 3 月 31 日版 CSV を使う。資料は別山頂の行などを含む 1,059 行で、全行を出力する。日本全国の主な山を収録するが、小さな山も含む全国の全山頂を網羅した一覧ではない。

`--natural --bbox 西 南 東 北` を指定すると、範囲内の自然地名タイルを国土地理院から直接取得し、`type=山` の地点を追加する。指定範囲に山岳一覧の地点がある場合は、山岳一覧も同じ範囲に絞る。自然地名は山の代表点であり、すべての座標が最高地点や山頂標識の位置を表すとは限らない。

全国の自然地名を一括取得する目録は、この生成処理では利用していない。OSM や第三者の地点一覧を取得対象の選択に流用せず、緯度経度の範囲からタイルを列挙する。誤って大量取得しないよう、1 回あたりの自然地名タイルは既定で 10,000 枚まで。取得は逐次行い、取得済みの応答は再利用する。

Python 3.12 以降を使い、リポジトリ直下で実行する。山岳一覧と自然地名の生成には追加の Python パッケージは不要。ベクトルタイルを使う全国生成には、後述のパッケージを使う。

```bash
# 主な山岳一覧の全国版
python points/gsi-peaks/generate.py --output build/gsi-national

# 富士山周辺で自然地名・標高タイルも試す
python points/gsi-peaks/generate.py --natural --dem \
  --bbox 138.70 35.34 138.75 35.38 \
  --output build/gsi-fuji --version local-fuji
```

`--bbox` の順は西端の経度・南端の緯度・東端の経度・北端の緯度。`--natural` には `--bbox` が必須。自然地名を使わない場合も、山岳一覧を範囲で絞るために `--bbox` を指定できる。`--max-tiles` で自然地名タイルの取得上限を変更できる。

取得ファイルと応答情報は `build/gsi-cache/` に保存する。`--cache` で置き場所を変えられる。URL・取得日時・SHA-256・サイズ・取得できた場合の Last-Modified と ETag を保存し、再利用時はハッシュを検証する。欠けているタイルの 404 も記録し、同じ範囲を取り直さない。403 や通信失敗を空のタイルとして扱わない。`--refresh` を指定すると再取得する。

Windows で公式 CSV の TLS を Python が扱えない場合は、Windows 標準の `curl.exe` を使う。証明書の検証は無効にしない。

| 出力 | 内容 |
|---|---|
| `gsi-peaks.json.gz` | 地点データ版6の gzip。比較・読み込みの試験用 |
| `gsi-peaks.json` | 同じ地点配列の非圧縮 JSON。共通ビューアでも開ける |
| `manifest.json` | 件数、サイズ、ハッシュ、出典、取得した各ファイルの情報 |
| `report.json` | 収録範囲、読み・標高の件数、資料間の相違、外字による除外 |

生成コマンドはローカルにファイルを出力する。Release への公開は、次の JSON 取り込みと公開 Action で行う。元データと途中生成物はコミットしない。公開用の地点 JSON と出典要約 JSON だけを、次の手順で取り込む。

### 参考：自然地名の全取得（通常の全国生成では不要）

```bash
python points/gsi-peaks/fetch_all.py
```

北方領土と離島を含む日本全域を囲む、西端の経度 122 度・南端の緯度 20 度・東端の経度 154 度・北端の緯度 46 度の範囲を、ズーム 15 の全 8,360,266 タイルで走査する。海上も含めて取得し、別のデータの収録範囲を根拠にタイルを省略しない。山だけを地点 JSON に出力するが、取得した自然地名タイルの全内容を保存する。

既定の同時接続数は 8。`--workers` で 1～16 に変更できる。長時間かかるため、途中の取得結果を `build/gsi-national-all/natural-tiles.sqlite3` に保存する。同じコマンドを再実行すると、取得済みの 200 と 404 を飛ばして再開する。通信障害・429・一時的なサーバー障害は間隔をあけて再試行し、403 などは失敗として停止する。通信の失敗を空のタイルとして扱わない。

`build/gsi-national-all/progress.json` の `phase` が `fetching` の間はタイル取得中、`elevation` は標高補完中、`complete` は JSON 生成完了。`failed` はエラーを記録して停止した状態。完了後、同じフォルダへ `gsi-peaks.json`・gzip 版・manifest・report を出力する。`--output` で出力先を変更できる。

全タイルの HTTP 状態・取得日時・200 の元ファイルとハッシュは SQLite に保存する。manifest の `sourceFiles` と report の `sources` には取得できた自然地名タイル、山岳一覧、標高タイルの情報を記載する。404 の明細は SQLite に残し、report の `scan` に取得範囲・全件数・404 件数・SQLite の SHA-256 を記録する。国土地理院がこの範囲で配信していない地点は補わず、全国の全山頂を網羅したとは扱わない。

### 全部入り JSON の生成

山以外も含む確認用 JSON の生成方法は、[全部入り JSON の生成手順](../README.md#山以外も含む全部入り-json-を作る) を参照する。

## 地点の補足対応表

[supplements.json](supplements.json) 一つでタグと補正を管理する。`id` で地点を指定し、補足適用前のオリジナルへタグの追加・削除、表示名・読みの補正、別名の追加・削除、配布対象からの除外を適用する。全国版の生成スクリプトでは `--supplements` で別の補足 JSON を指定できる。不正値・未知の ID・重複 ID・想定値の不一致はエラーとし、別の地点へ自動で付け替えない。

補足 JSON の配列は著書順を表さない。`note` は任意の管理用メモで、ビューアーで保存すると補足適用前の地点名を自動設定する。修正理由と管理用メモは補足 JSON に保持し、マージ済み地点 JSON には含めない。

共通形式は [補足データフォーマット](../formats/supplements-data.md)、編集操作は [ビューアの編集手順](../README.md#補足を編集する) を参照する。

### 出典の管理

補足の出典・ライセンス・帰属表示・加工内容は補足 JSON の `metadata.sources` に記録する。マージ済み地点 JSON には原本と補足の出典をまとめ、その `metadata` を配布 manifest にコピーする。出典は `note` に記載しない。項目と制約は [metadata フォーマット](../../formats/metadata.md) を参照する。

`snapshot.json` は補足適用前の原本の出典と取得記録を保持する。 日時の項目と制約は [metadata フォーマット](../../formats/metadata.md) と [manifest フォーマット](../../formats/manifest.md) を参照する。

### タグごとの記録

タグごとの記録は `notes/<タグ名>.md` に置く。新しいタグを追加するときも同じ命名にする。

以下のタグを補足 JSON に収録する。

| タグ | 読み | 記録 |
|---|---|---|
| 日本百名山 | にほんひゃくめいざん | [日本百名山の作成手順と採用地点](notes/日本百名山.md) |

出典・利用条件は「[出典とライセンス](#出典とライセンス)」を参照する。


## 出典とライセンス

| 用途 | 出典 |
|---|---|
| 山名・読み・最高地点・標高 | [日本の主な山岳標高一覧](https://www.gsi.go.jp/kihonjohochousa/kihonjohochousa41139.html)の [2026 年 3 月 31 日版 CSV](https://www.gsi.go.jp/KOKUJYOHO/MOUNTAIN/1003zan20260331.csv) |
| 範囲内の山名・読み・代表点・別称 | [自然地名の GeoJSON タイル提供実験](https://github.com/gsi-cyberjapan/experimental_pni) |
| 全国版の山の一覧・表示名・注記座標 | [最適化ベクトルタイル](https://github.com/gsi-cyberjapan/optimal_bvmap)（PMTiles） |
| 三角点・標高点の標高 | 最適化ベクトルタイルの `Anno` 分類コード `7102`・`7201` |
| 任意の地表標高の補完 | [標高タイル](https://maps.gsi.go.jp/development/demtile.html)（DEM1A・DEM5A/B/C・DEM10B の PNG） |
| 日本百名山の山名一覧 | [Wikipedia「日本百名山」](https://ja.wikipedia.org/w/index.php?title=日本百名山#著書順の山の一覧)（CC BY-SA 4.0）。帰属表示・加工内容は補足 JSON の `metadata.sources` を参照 |

国土地理院の資料は、[公共データ利用規約（第1.0版）](https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html)に従って利用・加工する。

出典・利用条件の一覧は、[ルート README のライセンス](../../README.md#ライセンス)を参照する。

## テスト

```bash
python -m pip install -r points/gsi-peaks/requirements-vector.txt
python -m unittest discover -s points/gsi-peaks/tests -v
python -m unittest release_tools.tests.test_gsi_peaks -v
```

テストは外部通信を行わず、山頂名と別称、GSI 専用 ID、重複、位置の範囲、地表標高、キャッシュ、取得失敗、共通の地点形式を確かめる。CI の「共通：データ処理のテスト」の「地点 / 国土地理院山頂」でも実行する。

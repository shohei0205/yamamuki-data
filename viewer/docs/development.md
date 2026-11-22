# ビューワーの開発・検証・公開

[ビューワーの使い方に戻る](../README.md) / [開発・検証・公開](development.md) / [描画とデータ処理](rendering.md) / [スナップショット](snapshots.md)

コマンドは、特に記載がない限りリポジトリのルートから実行する。

## 開発環境と起動

Rustのstableツールチェーンを用意する。WindowsではMSVCのビルドツール、macOSではXcode Command Line Toolsを用意する。macOSで未導入なら、ターミナルで `xcode-select --install` を実行する。

以下は両OSともリポジトリのルートから実行する。

```sh
cargo run --locked --release --manifest-path viewer/Cargo.toml
cargo run --locked --release --manifest-path viewer/Cargo.toml -- points/gsi-peaks/gsi-peaks-source.json
cargo run --locked --release --manifest-path viewer/Cargo.toml -- --terrain build/terrain/gsi-terrain.sqlite3
```

Windowsでは `viewer\run_win.bat` からも起動できる。起動のたびにCargoで変更を確認し、必要な部分をリリースビルドしてから実行ファイルを起動する。ファイルのパスを引数に渡すこともできる。macOSではターミナルから上記のコマンドで起動する。

## 通常CIとローカルGPUテスト

```sh
cargo test --locked --release --workspace --manifest-path viewer/Cargo.toml
cargo run --locked --manifest-path viewer/Cargo.toml -- --check-points points/gsi-peaks/gsi-peaks-source.json
cargo run --locked --manifest-path viewer/Cargo.toml -- --check-map
```

`--check-map` は確認用の地理院タイルを1枚取得するため、ネット接続が必要。

通常のテストは実機GPUを必要とせず、データ形式・復号・LOD選択・座標変換・キャッシュ方針・シェーダーの構文と型を検査する。GitHub ActionsのWindows／macOSの通常CIでは、この範囲のテスト、実行ファイルのビルド、地点JSON・地形SQLiteの読み込みを確認する。配布ビルドでも同じ通常テストと地点JSONの読み込みを確認する。

実際の描画画素・GPUメッシュ・接合・LOD変形・GPUメモリ・シェーダーのバックグラウンドコンパイルの検査は、`gpu-tests`機能を指定してローカルの実機で行う。GPUドライバー・アダプターに依存するため、共有のActionsランナーでは実行しない。複数のGPUテストが同時にコンパイルしないよう逐次実行する。

```sh
cargo test --locked --release --workspace --manifest-path viewer/Cargo.toml --features gpu-tests -- --test-threads=1
```

`gpu-tests`を指定しなくてもGPUテストのコードはコンパイルするが、テスト実行は明示的な無視として表示する。省略した実行を、GPU描画の検証済みとは扱わない。シェーダーのNagaによる構文検査は通常CIに残す。実データのスナップショット再生・全国データ比較・処理時間の計測は、従来どおり対象ファイルを用意し、該当テスト名を指定して`--ignored`で実行する。すべての無視テストを一括実行しない。

## 入力形式との互換性

入力仕様は[地点データの形式](../../points/formats/data.md)、[地点README](../../points/README.md)、版5・版6のJSON Schemaに従う。ビューワは表示用にID・名称・緯度経度・標高・読み・別名・タグを取り出す。ほかの項目を含む配布ファイルも開ける。標高は数値・null・省略を扱う。読みは版5で文字列、版6で文字列または文字列の配列を扱い、両版ともnull・省略を扱う。複数の読みは入力順に表示する。別名は版5の文字列と版6の名前・読みを持つオブジェクトを扱う。

表示項目を変更するときは、スキーマの `$ref` が指す共通定義と地点READMEも確認する。Cargoテストはスキーマから表示項目の型の候補を組み立て、実際の読み込み処理に通す。リポジトリにある国土地理院の地点JSONも読む。共通CIでWindowsとmacOS（Apple Silicon）のリリースビルドとテストを実行する。Windowsではビルドした実行ファイルを起動し、国土地理院の地点JSONを読み込めることも確認する。

## ファイルの読み込みと描画を検査する

```powershell
viewer\run_win.bat 'build/surface/japan-0.1.0/esa-worldcover.sqlite3'
cargo run --locked --release --manifest-path viewer/Cargo.toml -- --check-surface 'build/surface/japan-0.1.0/esa-worldcover.sqlite3'
```

`--check-surface` はウィンドウを開かず、地表データセットと各レベルの代表タイルを読み込んで合成する。

```sh
cargo run --locked --release --manifest-path viewer/Cargo.toml -- --terrain build/terrain/gsi-terrain.sqlite3
cargo run --locked --release --manifest-path viewer/Cargo.toml -- --terrain build/terrain/gsi-terrain.sqlite3 build/surface/esa-worldcover-z12.sqlite3
cargo run --locked --release --manifest-path viewer/Cargo.toml -- --check-terrain build/terrain/gsi-terrain.sqlite3
```

`--check-terrain-render <地形SQLite> <出力PNG> [地表SQLite]` は富士山の実画面を保存して終了する、手元の描画検査用コマンド。地形と地表を統合したSQLiteでは、収録された地表も自動的に重ねる。

`--check-terrain` は各レベルの代表タイルを復号し、形式に応じた標高データを準備して点数・容量を報告する。配布前の全タイル監査は `terrain-tool validate` を使う。地表のON/OFF・固定/自動LODは既存のレイヤー一覧で操作する。読み込みは背景スレッドで行う。

## 配布版の公開

GitHub Actions の「データビューワ：下書き Release 生成・検査」を手動で実行する。Windows x64 と macOS Apple Silicon の ZIP を作り、それぞれの実行ファイルで地点 JSON を読み込んで確認する。

1. `main` で実行すると、生成日時から配布名とタグを自動で決め、Draft Release を作る。Release名は[共通の命名規則](../../README.md#release名の規則)に従う。タグとZIP名の先頭には `viewer-` を付ける。バージョン番号の指定や更新は不要。作業ブランチでは Release を作らず、2 環境の ZIP を Actions の実行結果から 7 日間ダウンロードできる。
2. Draft の内容を確認し、「共通：Draft-release を試用公開」にそのタグまたは Release URL を指定する。2 環境の ZIP と `SHA256SUMS.txt` を検証し、同じ Release を Pre-release にする。
3. 試用して確認が済んだら、「共通：Pre-release を正式公開」に同じタグを指定する。添付ファイルを再検証して正式 Release にする。タグ・添付ファイル・取得 URL は変えない。ビューワの公開では Pages やデータカタログを更新しない。

配布先を選ぶ `channel` 入力は廃止し、生成時は常に下書きにする。次の配布も同じ Action を実行するだけで作れる。既存のタグや公開済み Release は上書きしない。

共通 Action の `sha256` は任意で、ビューワでは `SHA256SUMS.txt` 自体の SHA-256 を指定する。空欄でも、チェックサム一覧と両 ZIP の一致、ZIP 内の実行ファイルと破損の有無を検証する。公開時に ZIP の再生成・差し替えは行わない。

生成した Draft には整理用マーカーを付け、7 日以上経過した未公開の下書きは共通の整理 Action の対象になる。試用公開するとマーカーを外し、公開済みの Pre-release・正式版を保持する。長く保持する Draft は本文からマーカーを取り除く。

GitHub Releasesでは、ZIPと `SHA256SUMS.txt` を配布する。Windows版はZIPを展開して `yamamuki-viewer.exe` を起動する。macOS版はApple Silicon向けのZIPを展開して `YamamukiViewer.app` を開く。macOS版はアドホック署名で、Appleによる公証は行っていない。

Cargo.toml の `version` は Rust の内部情報として固定し、配布の識別には使わない。macOS の内部の版は UTC の生成日から自動設定する。既存の `viewer-v<版>` の下書きも、引き続き試用公開・正式公開できる。

Release名とGitタグの扱いは、[共通のRelease名の規則](../../README.md#release名の規則)に従う。

## エラーログと性能診断

異常終了時のRustエラーとスタックは、アプリの保存フォルダーの `logs/viewer.log` に追記する。起動時と通常の終了エラーも記録する。メインスレッドのエラーでは保存先をダイアログに表示する。OSやGPUドライバーによる強制終了など、Rustのエラー処理を通らない終了は記録できない場合がある。

GPU資源の連続描画検証は `cargo test --release --locked --manifest-path viewer/Cargo.toml -p yamamuki-terrain-view continuous_drawing_reuses_gpu_descriptors_and_uniform_storage -- --ignored --nocapture` で実行できる。`YAMAMUKI_GPU_STRESS_FRAMES` でフレーム数を指定でき、同じ192タイルを描き続けてもバインドグループ生成数と共通バッファ数が一定で、使用しなくなったタイルを解放することを検査する。

実データの地形スナップショット再現試験では `YAMAMUKI_SNAPSHOT_RELOAD=1` を指定すると、保存時のカメラ位置でタイルを選び直し、SQLiteから不足データを補ってGPU曲面が完成するまで検査できる。指定しない場合は保存時の表示集合を固定再生する。読み込みスレッドが終了していた場合は、次の要求時に表示済みメッシュを保ったまま読み込み処理だけを再開する。

起動・ファイル追加の速度検査では `YAMAMUKI_STARTUP_CHECK_OUTPUT` に結果JSONのパスを指定すると、自動終了する診断用起動になる。さらに `YAMAMUKI_STARTUP_CHECK_TERRAIN` に地形SQLiteのパスを指定すると、初回GPU準備が終わってからそのファイルを追加し、追加開始から最初の地形メッシュを描画するまでを `first_terrain_ms` に記録する。ウィンドウ表示、初回GPU準備、最初の地形表示、詳細LODの精細化完了は別の時間として扱う。

`YAMAMUKI_GPU_PROFILE` に出力ファイルのパスを指定すると、シェーダーモジュールの変換と各パイプラインの作成時間をJSON Linesで追記する。通常の地形、制御点ワイヤー、地球・空、自然近傍評価、地点投影、画像合成の各処理を個別に計測できる。起動全体の計測は前述の `YAMAMUKI_STARTUP_CHECK_OUTPUT` と併用する。計測用アプリは自動終了するため、普段の利用時はこの起動検査用環境変数を外す。

使用中のwgpu 24の永続 `PipelineCache` APIはVulkanのみ対応している。現在優先しているWindowsのDirect3D 12とmacOSのMetalでは、このAPIで再起動後に再利用できるキャッシュを保存できない。同じGPUデバイス内でのプログラム共用は継続する。永続キャッシュの導入には、Direct3D 12側のコンパイル済みコードを保存・復元できるバックエンド対応、または対応バックエンドの採用が必要になる。今回の高速化はDirect3D 12のまま再起動後も効く。

GPU描画の検査は`--features gpu-tests -- --test-threads=1`を指定する。実データの計測には入力ファイルと対象テスト名を指定し、必要な場合だけ`--ignored`を付ける。通常CIの範囲と区別して結果を記録する。

操作記録の途中画面を調べる場合は、`YAMAMUKI_OPERATION_TEST_INPUT`に展開した操作記録JSON、`YAMAMUKI_OPERATION_IMAGE_DIR`に出力先を指定し、`external_recorded_camera_frames`を個別に`--ignored --nocapture --test-threads=1`で実行する。5フレームごと（記録終了後は20フレームごと）の縮小画像を保存する。`YAMAMUKI_OPERATION_CAPTURE_FRAME`にフレーム番号を指定すると、そのフレームの位置バッファと地形スナップショットも保存する。

スナップショットの地形情報には `diagnostics`、F8操作記録の各フレームには `terrain_diagnostics` を保存する。実際に描画したタイルのキー・評価区画・分割数・描画パッチの分割数別件数に加え、要求中のタイル、選択世代、読み込み・補間・シーン準備の進行状態、キャッシュ件数とGPU元データ容量を記録する。入力の再生で症状が再現しない場合も、記録時のメッシュ切り替えを比較できる。診断情報は再生を強制するものではなく、非同期処理の完了時刻は再現しない。旧スナップショットも読み込める。記録には画像やメッシュの頂点本体を追加しないが、フレーム数とタイル数に応じて保存容量が増える。

GPUカリング比較は、地形スナップショットJSONを `YAMAMUKI_SNAPSHOT_TEST_INPUT` に指定し、`benchmark_external_gpu_culling` を `--ignored --nocapture --test-threads=1` で実行する。同じ視点・LODの地理座標、標高、陰影の差と欠落画素を検査し、実際の間接描画命令の可視パッチ数とフレーム時間を報告する。GPUからの命令読み戻しは、この手動ベンチマークだけで行う。


CPU計画を置き換えるGPU分割計画の比較は、同じ入力変数を指定して `compare_external_gpu_planner` を `--ignored --nocapture --test-threads=1` で実行する。従来方式と分割数・接合座標を照合し、統合した間接描画経路で欠落画素、XY位置・標高5cm超、陰影0.001超の差がないことを検査する。計画は各方式8回の中央値、描画は各32フレームの平均を出す。GPU計画は8回の反復でも区画・密度・接合情報がビット単位で一致することを検査する。初回GPUプログラム準備を含む切り替え待ちは別に報告する。GPU計画経路では通常動作でも必要な範囲の計画結果をCPUへ読み戻し、テストでは読み戻したバイト数も報告する。

外部データ不要のGPU計画検査は `cargo test --release --locked --manifest-path viewer/Cargo.toml -p yamamuki-terrain-view --features gpu-tests gpu_planner::tests -- --test-threads=1` で実行する。平面・球面、俯瞰・景色、視点方向の変更、背景被覆、隣接辺、再計画、三角形予算超過、パッチ数急増時の追加読み戻し、空入力、不正な格子のエラーを検査する。WindowsはDirect3D 12、macOSはMetalを対象とする。通常CIではGPU実行を行わず、WGSLの構文・型を検査する。

`YAMAMUKI_GPU_PLAN_PROFILE=1` はGPU計画を区間ごとに同期実行し、処理区間の時間をログへ出す診断用設定。CPUの投入・同期待ちも含むためGPU演算単独の時間ではない。通常の並列実行を区切るため、速度の比較時には外す。

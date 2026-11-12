# 地表データ

[リポジトリの README に戻る](../README.md)

地図上に配置する画像タイルを、レイヤーごとに収録する地表データの共通形式をまとめる。元データの取得・生成処理・配布手順は、取得元と対象ごとのフォルダに分ける。

| 対象・取得元 | 状態 | 詳細 |
|---|---|---|
| ESA WorldCover による日本全国の地表 | 森林・水域・陰影を収録した全国 SQLite をローカルで生成・検査し、Release で配布できる | [esa_worldcover](esa_worldcover/README.md) |

## データを取得して利用する

公開済みの地表データは、次の手順で取得して利用できる。

1. [Releases 一覧](https://github.com/shohei0205/yamamuki-data/releases)で利用するデータセットと版を選び、Assets から、利用するSQLiteに対応する `<SQLite の拡張子を除いた名前>.manifest.json` を取得する。タグと配布ファイル名は、各データセットの README を参照する。
2. manifest の `schemaVersion` とデータ本体の `dataSchemaVersion` に対応していることを確認する。
3. `downloadUrl` から SQLite を取得し、ファイルの `sizeBytes` と `sha256` を照合する。
4. [地表 SQLite の形式](#地表-sqlite-の形式)に従ってデータを読む。地図上で確認する場合は、[データビューワ](../viewer/README.md)の「地表SQLiteを開く」でファイルを選ぶ。
5. データセットの README と manifest の `metadata.sources` で、出典表示と利用条件を確認する。

## フォルダ構成

```text
surface/
  README.md         地表データの一覧・共通形式
  formats/          地表画像の SQLite 形式
  common/           SQLite の読み書き・画像の復号・検査・統合の共通コード
  tools/            検査・manifest 作成・SQLite 統合を行う surface-tool
  scripts/          共通の配布準備・公開と元データの取得スクリプト
  esa_worldcover/   ESA WorldCover を使う地表データ
```

元データの取得方法・収録範囲・生成方法・出典は、データセットごとの README を参照する。元データと生成物は `build/` などの Git 管理外に置く。

## 地表データの manifest

共通項目は [manifest フォーマットの「項目」](../formats/manifest.md#項目)、地表データの形式識別子とレイヤー別配布に使う項目は [山むき SQLite コンテナ仕様の「配布manifest」](../formats/sqlite.md#配布manifest)を参照する。

## 変更の確かめ方

- [共通の確認](../README.md#変更の確かめ方)
- [ESA WorldCover の確認](esa_worldcover/README.md#検査)
- [データビューワの確認](../viewer/README.md#確認)

### 検査と統合

リポジトリのルートで実行する。生成したデータと元データは `build/` などのGit管理外に置く。生成手順と出典・利用条件はデータセットごとのREADMEを参照する。

```powershell
cargo test --locked --manifest-path surface/Cargo.toml -p surface-tool -p yamamuki-data-sqlite
cargo run --locked --release --manifest-path surface/Cargo.toml --bin surface-tool -- validate 入力.sqlite3
cargo run --locked --release --manifest-path surface/Cargo.toml --bin surface-tool -- manifest 入力.sqlite3 https://github.com/shohei0205/yamamuki-data/releases/download 2026-11-09T00:00:00Z
cargo run --locked --release --manifest-path surface/Cargo.toml --bin surface-tool -- merge 出力.sqlite3 入力1.sqlite3 入力2.sqlite3
```

`validate` はSQLiteの整合性、参照、地表画像を検査する。`manifest` は配布用のJSONを標準出力に書く。`merge` は出力に新規ファイルを指定し、データセットIDが重複するとエラーにする。

## 地表データの公開

Release名とGitタグの扱いは、[共通のRelease名の規則](../README.md#release名の規則)に従う。

公開対象は1データセットを収録したSQLite。`surface/<データセットのフォルダー>/release.json` に、台帳の `datasetId`、`versionPattern`、配布単位 `packages` を記載する。各配布単位で `releaseId`、`fileName`、収録するレイヤーIDの配列 `layers` を指定する。タグは `<releaseId>-<データ版>`。全レイヤー入りのSQLiteは `all` で指定する。配布単位に `levels` を指定すると、入力SQLiteのレイヤー別収録ズームをその定義と照合する。レイヤー別配布を行う場合は、同じ配布単位に `splitPackages` を記載する。

```powershell
pwsh -NoProfile -File surface/scripts/prepare_release.ps1 `
  -Dataset '<データセットのフォルダー>' -Package all `
  -Database '入力.sqlite3' -OutputDirectory 'build/surface/配布準備' `
  -TargetCommit '<生成に使ったコミットの40桁のSHA>' -ProcessDraft
```

`-ProcessDraft` は受け渡し用Draftの作成・添付と「地表：Release 加工・検査」の起動を行う。GitHub CLIでログインし、生成に使ったコミットを先にpushして、Actionを `main` に配置する。`-CreateDraft` は受け渡し用Draftの作成まで行う。どちらも省くと配布ファイルの準備だけを行う。出力先は新規ディレクトリを指定する。生成日時は同じ場所のmanifestから引き継ぐ。記録がないファイルや抽出したSQLiteには、元の生成日時を `-DataTimestamp` にUTCで指定する。

Actionは添付したSQLite、manifest、ライセンスを取得し直し、収録画像、ハッシュ、レイヤーとレベル、タグと対象コミットを照合する。構成別の添付を作成し、検査記録を本文に残して同じReleaseを配布用Draftに整える。失敗時はDraftが残る。起動だけが失敗した場合は、表示された `gh workflow run` コマンドで再実行できる。

受け渡し用と配布用のDraftには整理マーカー `<!-- yamamuki-data:temporary-draft:v1 -->` を付ける。作成から7日以上経ったマーカー付きDraftは、共通の週次整理の対象になる。

加工・検査が完了した後は、次の順に `main` から手動実行する。

1. 「共通：Draft-release を試用公開」でDraftのURLまたはタグを指定する。検査記録と全添付を再検証し、同じReleaseをPre-releaseにする。このとき整理マーカーを外す。
2. アプリで確認した後、「共通：Pre-release を正式公開」で同じタグを指定する。全添付を再検証し、Pre-releaseの指定を解除する。

試用公開から正式公開まで、添付ファイル・タグ・取得URLを固定する。地表manifestはReleaseのAssetsから取得する。

### レイヤー別配布の構造

入力には全レイヤーを 1 つの SQLite に収録する。レイヤー別に配布する場合も、**同じデータセット・データ版のファイルを 1 つの GitHub Release にまとめる**。Release タグは `<releaseId>-<データ版>` を使う。

| 単位 | 構造 |
|---|---|
| SQLite | ファイルごとに、選んだレイヤーの設定・レベル・画像タイルを収録する。元のデータセット ID とデータ版を引き継ぎ、タイル画像の BLOB をそのままコピーする |
| manifest | SQLite ごとに作る。全レイヤー版・分割版とも `<SQLite の拡張子を除いた名前>.manifest.json` とする。`downloadUrl`・サイズ・SHA-256 に対応する SQLite の情報を記録する |
| GitHub Release | 同じタグの Assets に、すべての SQLite と対応する manifest を添付する。共通のライセンス文書は Release に 1 点添付する |

分割ファイルを同じデータセットとして扱う場合は、台帳のデータセット ID とデータ版を照合する。レイヤーは文字列の `id` で区別する。`dataset_key` と `layer_key` はファイル内の参照に使う。

#### 加工・検査Actionでの分割手順

ローカルでは全レイヤー入りの SQLite、そのファイル名に対応する `.manifest.json`、`LICENSE.txt` を下書き Release に添付する。

`release.json` の `splitPackages` に、配布ファイルごとの `fileName` と `layers` を指定する。任意の `levels` はレイヤー ID から実 XYZ ズームの整数配列への対応表。省略したレイヤーは全レベルを収録する。ファイル名は重複させない。同じレイヤーを別ファイルにも収録でき、各ファイルで組み合わせやレベルを変えられる。

```json
{
  "fileName": "esa-worldcover-overview.sqlite3",
  "layers": ["forest", "water"],
  "levels": {
    "forest": [5, 6, 7],
    "water": [5, 6, 7, 8]
  }
}
```

設定がない場合は一式だけを配布用Draftにする。設定がある場合、加工・検査Actionは次の順に処理する。

1. 下書きから全レイヤー版を取得し、SQLite、manifest、ライセンス、タグと対象コミットを検証する。
2. 検証済みの SQLite から `surface-tool select-surface` で指定したレイヤーとレベルを抽出する。分割版ごとに同じタグを指す manifest を作り、収録レイヤー、レベル、サイズ、SHA-256 を検証する。
3. 全レイヤー版と分割版を同じ下書きに揃え、添付ファイルの名前、サイズ、SHA-256を再確認し、検査記録を付けて配布用Draftにする。

途中で失敗した場合は下書きを保持する。再実行時は、検証済みの全レイヤー版から分割版を作り直し、下書きにある分割版の添付を照合する。検証結果と異なる分割版の添付は下書きの間に差し替える。

## 地表 SQLite の形式

ファイルの識別、データセット台帳、メタデータ、統合の決まりは [山むき SQLite コンテナ仕様](../formats/sqlite.md)にまとめている。地表画像のタイル座標・レベル・復号・合成方法は [地表画像の SQLite 形式](formats/sqlite.md)を参照する。

## データを追加するとき

`surface/<取得元と対象>/` に、README、生成処理、配布設定をまとめる。実装時にこの一覧を更新し、README に元データの取得方法・生成手順・出典と利用条件を書く。SQLite の検査や配布 manifest の作成には共通の `surface-tool` を使う。

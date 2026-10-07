# peaks正式版の復元用ブランチ

このブランチは地点共通形式への変更前のmain（`50fcf9e`）を元に、リリース済みアプリのテスト向けに版4の山頂データを再生成する。

Actionsの既存の山頂生成ワークフロー（`publish-data.yml`）で、実行ブランチに `codex/restore-peaks-stable` を選ぶ。手動実行だけを許可し、生成・検査後に正式版Releaseを公開してPagesを更新する。初回でもこの復元操作を手動確認として扱う。配布元の取得日を指定しない場合はlatestを使う。当時と同一のファイルの復元ではなく、当時の形式での再生成となる。

- manifest: `https://shohei0205.github.io/yamamuki-data/peaks/manifest.json`
- Releaseタグ: `peaks-<version>`
- 本体: `japan-mountains.json.gz`
- データ形式: 版4（整数osmId、mountainCountなど）

テストは `peaks/` で `python -m unittest discover -s tests -v`、ルートで `python -m unittest discover -s release_tools/tests -v` を実行する。復元ブランチの配布先制限と地点カタログ・履歴の保持も確認する。

現在のosm-peaksのmanifest・履歴と地点カタログは引き継ぐ。新しい地点カタログには互換用peaksを追加しない。公開処理は通常の公開と同じ `publish-data-pages` グループで直列化する。mainやdevにはこのブランチをマージしない。

現在のmain/devの公開処理はpeaksパスを除外するため、その処理を再実行すると復元パスは削除される。テスト中に継続して併存させる場合は、main/dev側でも保持する変更が必要。

---

# yamamuki-data

山むきアプリ（[shohei0205/yamamuki](https://github.com/shohei0205/yamamuki)）が使うデータを作って配るためのリポジトリ。

## データ一覧

| データ | 状態 | 詳細 |
|---|---|---|
| 山頂 | 生成・検査・公開の処理を実装 | [山頂データの仕様と運用](peaks/README.md) |
| 地形 | 将来追加予定・未実装 | 実装時に専用の文書を追加する |

データごとの収録対象、JSON の形式、ダウンロード先、生成・公開手順、検査基準は、それぞれの文書にまとめる。

## フォルダ構成

```text
peaks/
  README.md       山頂データの仕様・生成・公開手順
  scripts/        山頂データの取得・生成・検査・公開
  tests/          山頂データのテスト
.github/workflows/ GitHub Actions の定義
```

地形データの実装時は `terrain/` を追加する。Actions の定義は `.github/workflows/` に置き、データごとのフォルダを作業ディレクトリにして処理を実行する。

## 配布の方針

生成したデータは [GitHub Releases](https://github.com/shohei0205/yamamuki-data/releases) に置く。元データとデータ本体は git に含めない。最新版を示す小さな `manifest.json` は、Actions の生成物として GitHub Pages に公開する。

データの種類ごとに公開時期と最新版の参照先を分ける。正式版と開発版も独立して扱い、リポジトリ全体の `releases/latest` はアプリの参照先に使わない。具体的な URL とタグは各データの文書を参照する。山頂データの manifest には、実際に取得・検証した日付付き URL を `sourceUrl` として記録する。

| ブランチ | 配布先 |
|---|---|
| `main` | 正式版 |
| `dev` | 開発版（Pre-release） |

開発中の処理は `dev` で確認し、正式版に採用するときは変更を `main` に取り込む。実行タイミングと確認方法はデータごとに定める。山頂データは手動実行時に取得対象日も指定できる（[取得対象日の指定](peaks/README.md#取得対象日を指定する)）。

正式版も開発版も、このリポジトリの同じ [Releases 一覧](https://github.com/shohei0205/yamamuki-data/releases) に公開する。`main`・`dev` は生成処理の実行元で、配布ファイルは各 Release の **Assets** に添付する。データの種類と正式版・開発版の違いは、Release のタグで区別する。

山頂データ本体は、正式版の `peaks-<version>` と開発版の `peaks-dev-<version>` の Release に置く。各版の Assets は `manifest.json` と `japan-mountains.json.gz` で、公開後は差し替えない。

最新版を示す manifest は GitHub Pages に置く。`main`・`dev` のコードでサイトのファイルを生成して直接配置するため、配布専用ブランチは作らない。生成物も git に入れない。

データごとの manifest と公開履歴の URL、データ本体の取得方法は、各データの文書にまとめる。山頂データは [置き場所](peaks/README.md#置き場所)と[公開履歴](peaks/README.md#公開履歴)を参照する。

## 確認済みデータの公開

Actions の「確認済みのデータを公開」を共通の入口とする。確認した Release の URL（またはタグ）を入力すると、タグからデータ種別を判定し、その種別の検証・公開処理を実行する。正式版は `main`、開発版は `dev` を選ぶ。確認理由は任意。SHA-256 は検査結果から自動取得できる。

共通の振り分け処理は `release_tools/`、データ固有の検証・公開処理は各データのフォルダに置く。現時点で対応するのは山頂（`peaks`）のみ。将来は地形用の検証・公開処理を実装し、`release_tools/publish_reviewed.py` の登録表に追加する。地形専用の手動公開アクションを増やす必要はない。地形の生成アクションや公開時期は別に設定できる。

山頂データの生成は配布先ごとに直列化する。自動公開と手動公開のジョブは共通の `publish-data-pages` グループで直列化し、既存のサイトを引き継いでから配置する。正式版・開発版は別のパスに保ち、片方の公開で他方の参照先を消さない。未対応の種別や最新版参照タグを公開対象に指定すると停止する。

最新版 manifest の固定 URL と Pages の配置結果は公開ジョブの Summary に表示する。下書きで保留された場合は参照先を更新しない。

両方の Actions（生成・検査と確認済みデータの公開）は、処理の冒頭に Summary へ実行パラメーターを表示する。未入力の場合の扱いも明記する。生成した Release へのリンクも Summary に表示し、下書きの確認へ移動できる。

全国 PBF は Actions のキャッシュに保存し、同じ配布日・MD5 のデータを再利用する。配布元の情報と復元ファイルは毎回照合し、キャッシュがない場合や一致しない場合は再取得する。

## ブランチのマージ方針

開発版の `dev` と正式版の `main` は継続して使い、変更は PR を通して取り込む。取り込み先に応じて、次のマージ方法を選ぶ。

| 取り込み元 → 取り込み先 | マージ方法 |
|---|---|
| 作業ブランチ → `dev` | スカッシュマージ（Squash and merge） |
| `dev` → `main` | 通常のマージコミット（Create a merge commit） |
| `main` → `dev`（正式版だけに入れた修正の反映） | 通常のマージコミット（Create a merge commit） |

作業ブランチの変更は、1つの目的を1つのコミットにまとめて `dev` に取り込む。`dev` で確認した変更を正式版に採用するときは、取り込み先を `main`、取り込み元を `dev` とした PR を作る。

`dev` → `main` は履歴を共有したまま取り込む。スカッシュすると、同じ変更が `main` では別のコミットになり、次の正式版への PR に取り込み済みのコミットが並んだり、競合が起きやすくなったりするため、この方向ではスカッシュやリベースでのマージを使わない。

正式版だけに修正を入れた場合は、`main` → `dev` の PR を作り、マージコミットで取り込む。これにより、次の開発・正式版公開にも修正と履歴を引き継ぐ。`dev` → `main` のマージ後も `dev` ブランチは削除しない。

GitHub のリポジトリ設定では、マージコミットとスカッシュマージの両方を許可する。マージはユーザーが行い、エージェントは行わない。

## 開発ルール

作業の進め方は [AGENTS.md](AGENTS.md) を参照する。データを追加するときは種類ごとのフォルダにスクリプト・テスト・README をまとめ、この README のデータ一覧からリンクする。

## ライセンス

データの出典は **© OpenStreetMap contributors**。配布データは [Open Database License（ODbL）1.0](https://opendatacommons.org/licenses/odbl/1-0/) に従って利用・再配布する。[OpenStreetMap の著作権とライセンス](https://www.openstreetmap.org/copyright)を参照。

元の日本全国データは [Geofabrik](https://download.geofabrik.de/asia/japan.html) が提供する。アプリなどで配布データを使う場合も、出典とライセンスを表示する。

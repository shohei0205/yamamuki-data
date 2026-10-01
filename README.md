# yamamuki-data

山むきアプリ（[shohei0205/yamamuki](https://github.com/shohei0205/yamamuki)）が使う山データを作って配るためのリポジトリ。

## 配るもの

<!-- データの種類、中身、状態 -->

## 置き場所

<!-- Releases の URL、manifest.json の形式 -->

## 作り方

### 開発版を手動実行する

`main` には Actions の手動実行一覧に表示するための入口を置く。データの生成・公開処理は `dev` にあり、選択したブランチのワークフローを実行する。正式版の生成・公開と月次実行は、正式導入時に追加する。

1. [Actions](https://github.com/shohei0205/yamamuki-data/actions) を開く。
2. 「全国の山頂データを生成・検査」を選び、「Run workflow」を開く。
3. **Branch を `dev` に変更**する。取得対象日を固定する場合は `source_date` に配布済みの日付を `YYYY-MM-DD` で入力する（例: `2026-09-29`）。空欄は `latest` を使う。実行当日分は未配布の場合があるため、実行日を自動入力しない。その後実行する。検査に合格した場合は開発版（Pre-release）を公開する。初回や要確認の場合は下書きで保留する。
4. 下書きを確認して公開するときは「確認済みの山頂データを公開」を選び、同じく **Branch を `dev` に変更**する。対象タグ・SHA-256・確認理由を入力して実行する。

日付指定を使う前に、`dev` 側にも `source_date` を受け取る生成処理を取り込む。日付指定時は `japan-YYMMDD.osm.pbf` を直接取得し、`latest` の転送障害を避けられる。

`main` のまま実行すると、`dev` を選び直す案内を出して失敗で終了する。データの取得・生成・Release の作成は行わない。

検査条件と詳しい操作は [dev の山頂データの説明](https://github.com/shohei0205/yamamuki-data/blob/dev/peaks/README.md) を参照する。

GitHub の手動実行は既定ブランチにもワークフローが必要なため、入口のファイル名と入力項目を `dev` 側に合わせている。[GitHub の手動実行の説明](https://docs.github.com/ja/actions/how-tos/manage-workflow-runs/manually-run-a-workflow)を参照。

正式導入時は、この入口を `dev` 側の実際のワークフローと生成処理に置き換える。入口の追加後に `main` の変更を `dev` へ取り込む場合は、`dev` 側の生成・公開処理を残す。

## ライセンス

<!-- コードのライセンス（MIT）と、データのライセンス（ODbL）・出典表示 -->

# manifest フォーマット

[リポジトリの README に戻る](../README.md)

## 概要

このリポジトリで配布する各データセットに共通する、UTF-8（BOM なし）の JSON オブジェクト。データ本体の取得先・大きさ・ハッシュ・出典を記載する。データ本体の構造は種類ごとの仕様に従い、地点データは [points/README.md](../points/README.md#地点-json-の形式) に記載する。過去版との差分と仕様は「変更履歴」にまとめる。

共通スキーマはデータ種別ごとの追加項目を許可する。

`format` の検査を有効にした検証ツールを使う。日時の前後関係・本体のサイズ・ハッシュ・件数・取得 URL と Release の一致は JSON Schema では照合できないため、公開処理で別途検証する。

## 版について

manifest 自体の版は `schemaVersion` で確認する。現行版は `6`。データ本体の形式は `dataSchemaFormat`、版は `dataSchemaVersion` に記載し、manifest の版とは独立して管理する。

版内の変更は `版1.1` のように小数点以下で区別する。小数点以下は文書上の番号で、JSON の版の値には含めない。

項目表の「版の履歴」は、フィールドの追加や変更があった版を示す。スキーマは版ごとに分けて管理する。

| 形式の版 | 機械検証用スキーマ | 規格 |
|---|---|---|
| 6 | [manifest-v6.schema.json](../schemas/manifest-v6.schema.json) | JSON Schema Draft 2020-12 |
| 5 | [manifest-v5.schema.json](../schemas/manifest-v5.schema.json) | JSON Schema Draft 2020-12 |
| 1〜4 | スキーマなし。変更履歴を参照 | — |

## 項目

| 項目 | JSON の型 | 必須 | 内容・制約 | 版の履歴 |
|---|---|---|---|---|
| `schemaFormat` | string | 必須 | JSON の形式の識別子。`yamamuki-manifest` 固定。他の形式と区別する | [版6] 追加 |
| `schemaVersion` | integer | 必須 | JSON の形式の版。現行版は `6` | [版1] 追加 |
| `dataSchemaFormat` | string | 必須 | データ本体の `schemaFormat` と同じ値。地点は `yamamuki-points`、補足は `yamamuki-point-supplements`。manifest 自身の `schemaFormat` とは独立 | [版6] 追加 |
| `dataSchemaVersion` | integer | 必須 | データ本体のスキーマの版。正の整数。manifest の `schemaVersion` と独立して管理する | [版5] 追加<br>[版6] 必須化 |
| `name` | string | 任意 | データセットの表示名 | [版5] 追加 |
| `version` | string | 必須 | データセットの配布版。英数字で始まり、英数字・ピリオド・ハイフン・下線で構成する | [版1] 追加 |
| `downloadUrl` | string | 必須 | この版の gzip データ本体を取得する HTTPS URL | [版4] 追加 |
| `fileName` | string | 必須 | gzip データ本体のファイル名。データセットごとに定める | [版1] 追加 |
| `sha256` | string | 必須 | gzip ファイルそのものの SHA-256。小文字の16進数64文字 | [版1] 追加 |
| `sizeBytes` | integer | 必須 | gzip ファイルのバイト数。正の整数 | [版1] 追加 |
| `uncompressedSizeBytes` | integer | 必須 | 展開後の JSON のバイト数。正の整数 | [版1] 追加 |
| `dataTimestamp` | string | 必須 | manifest が指す配布データを生成した日時。UTC の ISO 8601（例: `2026-10-14T03:00:00Z`）。元データの編集日時・取得日時・Release の公開日時ではない | [版6] 追加 |
| `metadata` | object | 必須 | 指し示すデータ JSON の `metadata` をそのままコピーする。型・構造・制約は [metadata フォーマット](metadata.md) を参照。 | [版6] 追加 |
| `pointCount` | integer | 廃止 | 版5の地点件数（必須）。版6では `metadata.pointCount` に移動する | [版1] 元の項目を追加<br>[版5] `mountainCount` から改名<br>[版6] 直下の項目を廃止 |
| `latestPointTimestamp` | string または null | 廃止 | 版5の収録地点の最終編集日時の最大値（任意）。版6では `metadata.sources[].lastestTimeStamp` に移動する | [版3] 元の項目を追加<br>[版5] `latestMountainTimestamp` から改名し任意化<br>[版6] 直下の項目を廃止 |
| `sourceTimestamp` | string | 廃止 | 版5以前の元データ全体の基準日時。版6では廃止。配布データの生成日時は `dataTimestamp` に記載する | [版1] 追加<br>[版5] 必須から任意に変更<br>[版6] 直下の項目を廃止 |
| `sourceUrl` | string | 廃止 | 版5以前の元データの取得・参照 URL。版6では `metadata.sources[].source` に記載する | [版1] 追加<br>[版3.1] 日付付き URL に変更<br>[版5] 必須から任意に変更<br>[版6] 廃止 |
| `license` | string | 廃止 | 版5以前のデータセットの利用条件（必須）。版6では `metadata.sources[].license` に記載する | [版1] 追加<br>[版6] 廃止 |
| `attribution` | string | 廃止 | 版5以前の出典表示（必須）。版6では `metadata.sources[].attribution` に記載する | [版1] 追加<br>[版6] 廃止 |
| `supplementSources` | object | 廃止 | 版5の補足の出典一覧（任意）。版6では元データの出典とともに `metadata.sources` に統合する | [版5] 追加<br>[版6] 廃止 |

地点データ版6では、地点 JSON の `metadata` を manifest の `metadata` にそのままコピーする。出典・タグ・件数・更新日時もこの object 内で保持する。地点文書の構造は [地点データフォーマット](../points/formats/data.md) を参照する。

### 補足ファイル単体の配布

補足ファイルを単体で配布するときも、この文書の共通 manifest 形式を使う。`dataSchemaVersion` は補足 JSON 本体の版とし、取得先・サイズ・ハッシュは配布する補足ファイルの値を記載する。`metadata` は補足 JSON の `metadata` をそのままコピーする。

## 読み込みと検証

現行形式では、各 object に未知の項目を許可し、読み手はそれを解釈しない。既知の項目の型・必須条件・重複などの制約は維持する。

利用するアプリは対応する `schemaVersion` を確認し、`downloadUrl` から本体を取得する。展開前に `sizeBytes` と `sha256`、展開後に `uncompressedSizeBytes` を照合する。データ種別ごとの件数なども各仕様に従って照合する。地点データの件数は、manifest 版4では `mountainCount`、版5では `pointCount`、版6では `metadata.pointCount` を使う。通信や検証に失敗した場合は保存済みデータを維持する。

データセットと正式版・開発版ごとに最新版の manifest の URL を分ける。Release に添付する manifest と最新版として配置する manifest は同じ内容を使い、公開済みの版の本体は差し替えない。公開先の URL・Release タグ・ファイル名は各データセットの資料に記載する。

スキーマファイルは種類ごとに保存する。データ種別に共通する manifest は `schemas/manifest-v<版>.schema.json` に置き、データ本体などのスキーマはデータ種別のフォルダ（地点データは `points/schemas/`）に置く。それぞれ必要なときに独立して版を上げ、公開済みの版は原則変更しない。

## JSON の例

版5・版4は公開済み Release を基に、任意項目を補った説明用の例。版1〜3.1は版4の例を基に、履歴で確認できる項目の差を示した説明用の例で、実在する Release の manifest ではない。

### 版6

構造を示す例。サイズ・ハッシュは説明用の値。

```json
{
  "schemaFormat": "yamamuki-manifest",
  "schemaVersion": 6,
  "dataSchemaFormat": "yamamuki-points",
  "dataSchemaVersion": 6,
  "name": "見本の地点データ",
  "version": "example-v6",
  "fileName": "sample-points.json.gz",
  "downloadUrl": "https://example.com/releases/example-v6/sample-points.json.gz",
  "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
  "sizeBytes": 1000,
  "uncompressedSizeBytes": 5000,
  "dataTimestamp": "2026-10-15T00:00:00Z",
  "metadata": {
    "sources": [
      {
        "source": "見本の地名資料 https://example.com/source",
        "license": "CC BY 4.0 https://creativecommons.org/licenses/by/4.0/",
        "attribution": "見本資料の著作者",
        "changes": "地点名・座標・標高を抽出",
        "lastestTimeStamp": "2026-10-14T00:00:00Z",
        "name": "元資料"
      },
      {
        "source": "見本の山名一覧 https://example.com/supplement",
        "license": "CC0-1.0",
        "attribution": "見本の補足資料の作成者",
        "changes": "読み・別名・タグを追加",
        "lastestTimeStamp": "2026-10-14T01:00:00Z",
        "name": "補足資料"
      }
    ],
    "tags": [
      {
        "name": "日本百名山",
        "nameReading": "にほんひゃくめいざん"
      }
    ],
    "pointCount": 1
  }
}
```

### 版5

```json
{
  "schemaVersion": 5,
  "dataSchemaVersion": 5,
  "name": "山頂",
  "version": "20261007T071401Z-37585754546-1",
  "fileName": "osm-peaks.json.gz",
  "downloadUrl": "https://github.com/shohei0205/yamamuki-data/releases/download/osm-peaks-20261007T071401Z-37585754546-1/osm-peaks.json.gz",
  "sha256": "01c732e53edd96840111cd17361f2a4d5484e1c2887606663c554377047ee7ee",
  "sizeBytes": 505256,
  "uncompressedSizeBytes": 3203003,
  "pointCount": 14035,
  "sourceTimestamp": "2026-10-06T20:21:06Z",
  "latestPointTimestamp": "2026-10-06T19:13:49Z",
  "sourceUrl": "https://download.geofabrik.de/asia/japan-261006.osm.pbf",
  "license": "ODbL-1.0",
  "attribution": "© OpenStreetMap contributors",
  "supplementSources": {
    "補足資料": {
      "source": "見本の資料 https://example.com/supplement",
      "license": "CC0-1.0",
      "attribution": "見本資料の作成者",
      "changes": "山名・読み・別名を抽出"
    }
  }
}
```

### 版4

```json
{
  "schemaVersion": 4,
  "version": "20261007T084736Z-37595744056-1",
  "fileName": "japan-mountains.json.gz",
  "downloadUrl": "https://github.com/shohei0205/yamamuki-data/releases/download/peaks-20261007T084736Z-37595744056-1/japan-mountains.json.gz",
  "sha256": "7c327218a36e23da35c19cc6b032723d6e036f0a343ed319e37de06f70017b8c",
  "sizeBytes": 451248,
  "uncompressedSizeBytes": 2751355,
  "mountainCount": 14035,
  "sourceTimestamp": "2026-10-06T20:21:06Z",
  "latestMountainTimestamp": "2026-10-06T19:13:49Z",
  "sourceUrl": "https://download.geofabrik.de/asia/japan-261006.osm.pbf",
  "license": "ODbL-1.0",
  "attribution": "© OpenStreetMap contributors"
}
```

### 版3.1

```json
{
  "schemaVersion": 3,
  "version": "20261007T084736Z-37595744056-1",
  "fileName": "japan-mountains.json.gz",
  "sha256": "7c327218a36e23da35c19cc6b032723d6e036f0a343ed319e37de06f70017b8c",
  "sizeBytes": 451248,
  "uncompressedSizeBytes": 2751355,
  "mountainCount": 14035,
  "sourceTimestamp": "2026-10-06T20:21:06Z",
  "latestMountainTimestamp": "2026-10-06T19:13:49Z",
  "sourceUrl": "https://download.geofabrik.de/asia/japan-261006.osm.pbf",
  "license": "ODbL-1.0",
  "attribution": "© OpenStreetMap contributors"
}
```

### 版3

```json
{
  "schemaVersion": 3,
  "version": "20261007T084736Z-37595744056-1",
  "fileName": "japan-mountains.json.gz",
  "sha256": "7c327218a36e23da35c19cc6b032723d6e036f0a343ed319e37de06f70017b8c",
  "sizeBytes": 451248,
  "uncompressedSizeBytes": 2751355,
  "mountainCount": 14035,
  "sourceTimestamp": "2026-10-06T20:21:06Z",
  "latestMountainTimestamp": "2026-10-06T19:13:49Z",
  "sourceUrl": "https://download.geofabrik.de/asia/japan-latest.osm.pbf",
  "license": "ODbL-1.0",
  "attribution": "© OpenStreetMap contributors"
}
```

### 版2

```json
{
  "schemaVersion": 2,
  "version": "20261007T084736Z-37595744056-1",
  "fileName": "japan-mountains.json.gz",
  "sha256": "7c327218a36e23da35c19cc6b032723d6e036f0a343ed319e37de06f70017b8c",
  "sizeBytes": 451248,
  "uncompressedSizeBytes": 2751355,
  "mountainCount": 14035,
  "sourceTimestamp": "2026-10-06T20:21:06Z",
  "sourceUrl": "https://download.geofabrik.de/asia/japan-latest.osm.pbf",
  "license": "ODbL-1.0",
  "attribution": "© OpenStreetMap contributors"
}
```

### 版1

```json
{
  "schemaVersion": 1,
  "version": "20261007T084736Z-37595744056-1",
  "fileName": "japan-mountains.json.gz",
  "sha256": "7c327218a36e23da35c19cc6b032723d6e036f0a343ed319e37de06f70017b8c",
  "sizeBytes": 451248,
  "uncompressedSizeBytes": 2751355,
  "mountainCount": 14035,
  "sourceTimestamp": "2026-10-06T20:21:06Z",
  "sourceUrl": "https://download.geofabrik.de/asia/japan-latest.osm.pbf",
  "license": "ODbL-1.0",
  "attribution": "© OpenStreetMap contributors"
}
```

## 変更履歴

### 版6（2026-10-14）

- `dataSchemaVersion` を必須にする。データ本体の版を manifest の版から推測しない。

- 必須の `dataSchemaFormat` を追加し、データ本体の `schemaFormat` と一致させる。

- 必須の `schemaFormat: "yamamuki-manifest"` を追加し、形式を識別する。未公開の版6に統合する。

- 地点全体の `latestPointTimestamp` を廃止し、出典ごとの `metadata.sources[].lastestTimeStamp` に最終更新日時を保持する。

- 補足ファイル単体の配布にも共通の manifest 形式を使う。地点データは補足適用後・補足適用前・補足の 3 種類を、それぞれ共通形式の manifest 付きで配布する。

- `sourceTimestamp` を廃止し、配布データの生成日時を表す `dataTimestamp` を直下に追加する。
- 元データと補足の出典を `metadata.sources` に統合する。
- `sourceUrl`・`license`・`attribution`・`supplementSources` を廃止する。URL は各出典の `source`、利用条件と帰属表示は各出典の `license`・`attribution` に記載する。
- 版5以前の manifest は従来のスキーマで検証する。

### 版5（2026-10-07、#36）

#### 前の版との差分

| 項目 | 版4 | 版5 |
|---|---|---|
| データ本体の版 | manifest と同じ番号で管理 | `dataSchemaVersion` で独立して管理 |
| 件数 | `mountainCount` | `pointCount` |
| 最終編集日時 | `latestMountainTimestamp` | `latestPointTimestamp` |
| `sourceTimestamp` | 必須 | 任意 |
| 追加項目 | | `dataSchemaVersion`・`name`・`supplementSources` |

版3で `latestMountainTimestamp`、版4で `downloadUrl` を追加した。各フィールドの追加・変更は項目表の「版の履歴」を参照する。

件数と最終編集日時の項目名が変わるため、版4の項目名を前提とする読み手は対応が必要。地点 JSON の変更は [地点データフォーマットの変更履歴](../points/formats/data.md#変更履歴) を参照する。

版1〜3 は、最初の生成・公開処理（#6、2026-10-01 マージ）の中で順に作られた。版1〜3 の Release は今は残っていない。

### 版4（2026-10-03、#26）

- manifest に `downloadUrl` を足した。版3 までは、読み手が `https://github.com/shohei0205/yamamuki-data/releases/download/peaks-<version>/<fileName>` を自分で組み立てる必要があった。
- 新しく生成する版4 では `downloadUrl` を必須にし、公開前にリポジトリ・版・配布先・ファイル名と合っているかを確かめる。

### 版3.1（2026-10-01、#12）

- `sourceUrl` の値を `japan-latest.osm.pbf` から実際に取得した日付付きの URL に変更した。JSON の `schemaVersion` は `3` のまま。

### 版3（2026-10-01、#6）

- manifest に `latestMountainTimestamp`（収録した山頂ノードの OSM 上の最終編集日時の最大値）を足した。
- 収録する山頂ノードに日時が無い、形式が正しくない、元データの基準日時より新しい場合は、生成を止める。

### 版2（2026-10-01、#6）

- manifest の項目は版1 と同じ。地点 JSON の項目の追加は [地点データフォーマットの変更履歴](../points/formats/data.md#変更履歴) を参照する。

### 版1（2026-10-01、#6）

- manifest: `schemaVersion`・`version`・`fileName`・`sha256`・`sizeBytes`・`uncompressedSizeBytes`・`mountainCount`・`sourceTimestamp`、それに出典の `sourceUrl`・`license`・`attribution`。
- 版1 の仕様を書いた資料は残っていない。上の項目は、#6 の検査処理（`peaks/scripts/check_release.py` の `validate`）が版1 に求める項目から読み取ったもの。

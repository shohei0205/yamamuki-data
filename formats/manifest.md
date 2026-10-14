# manifest フォーマット

[リポジトリの README に戻る](../README.md)

## 概要

このリポジトリで配布する各データセットに共通する、UTF-8（BOM なし）の JSON オブジェクト。データ本体の取得先・大きさ・ハッシュ・出典を記載する。データ本体の構造は種類ごとの仕様に従い、地点データは [points/README.md](../points/README.md#地点-json-の形式) に記載する。過去版との差分と仕様は「変更履歴」にまとめる。

共通スキーマはデータ種別ごとの追加項目を許可する。

`format` の検査を有効にした検証ツールを使う。日時の前後関係・本体のサイズ・ハッシュ・件数・取得 URL と Release の一致は JSON Schema では照合できないため、公開処理で別途検証する。

## 版について

manifest 自体の版は `schemaVersion` で確認する。現行版は `5`。データ本体の版は `dataSchemaVersion` に記載し、manifest の版とは独立して管理する。

版内の変更は `版1.1` のように小数点以下で区別する。小数点以下は文書上の番号で、JSON の版の値には含めない。

項目表の「版の履歴」は、フィールドの追加や変更があった版を示す。スキーマは版ごとに分けて管理する。

| 形式の版 | 機械検証用スキーマ | 規格 |
|---|---|---|
| 5 | [manifest-v5.schema.json](../schemas/manifest-v5.schema.json) | JSON Schema Draft 2020-12 |
| 1〜4 | スキーマなし。変更履歴を参照 | — |

## 項目

| 項目 | JSON の型 | 必須 | 内容・制約 | 版の履歴 |
|---|---|---|---|---|
| `schemaVersion` | integer | 必須 | 形式の版。新規生成は `5` | [版1] 追加 |
| `dataSchemaVersion` | integer | 新規生成では必須 | データ本体のスキーマの版。正の整数。manifest の `schemaVersion` と独立して管理する | [版5] 追加 |
| `name` | string | 任意 | データセットの表示名 | [版5] 追加 |
| `version` | string | 必須 | データセットの配布版。英数字で始まり、英数字・ピリオド・ハイフン・下線で構成する | [版1] 追加 |
| `downloadUrl` | string | 必須 | この版の gzip データ本体を取得する HTTPS URL | [版4] 追加 |
| `fileName` | string | 必須 | gzip データ本体のファイル名。データセットごとに定める | [版1] 追加 |
| `sha256` | string | 必須 | gzip ファイルそのものの SHA-256。小文字の16進数64文字 | [版1] 追加 |
| `sizeBytes` | integer | 必須 | gzip ファイルのバイト数。正の整数 | [版1] 追加 |
| `uncompressedSizeBytes` | integer | 必須 | 展開後の JSON のバイト数。正の整数 | [版1] 追加 |
| `sourceTimestamp` | string | 任意 | 元データ全体の基準日時。取得日時とは区別する。不明なら省略 | [版1] 追加<br>[版5] 必須から任意に変更 |
| `sourceUrl` | string | 任意 | 元データの取得・参照 URL。特定できない場合は省略 | [版1] 追加<br>[版3.1] 日付付き URL に変更<br>[版5] 必須から任意に変更 |
| `license` | string | 必須 | データセットの利用条件。OSM 由来では `ODbL-1.0` | [版1] 追加 |
| `attribution` | string | 必須 | 表示すべき出典。OSM 由来では `© OpenStreetMap contributors` | [版1] 追加 |
| `tags` | object | 任意 | 地点 JSON の `metadata.tags` と同じタグ共通の読み情報 | [版5] 任意の追加情報として使用 |
| `supplementSources` | object | 任意 | 補足 JSON の `metadata.sources` を複製した出典一覧。構造と各属性の説明は [補足データフォーマットの出典ごとの情報](../points/formats/supplements-data.md#出典ごとの情報) を参照。出典情報がない場合は省略する | [版5] 追加 |

地点データ版6では、`sourceUrl`・`license`・`attribution`・`supplementSources` を地点 JSON にも記載し、manifest と一致させる。地点 JSON の `metadata` の各子項目を manifest の直下に複製する。タグ共通の情報も `tags` に保持する。地点文書の構造は [地点データフォーマット](../points/formats/data.md) を参照する。

### 地点データ用の項目

地点データの manifest では、共通項目に加えて次の項目をトップレベルに置く。`supplementSources` の子には入れない。ほかのデータ種別の追加項目と制約は、そのデータの仕様に記載する。日時は UTC の ISO 8601 形式（例: `2026-09-30T20:21:22Z`）を使い、記録していない日時を推測して埋めない。

| 項目 | 型 | 必須 | 内容 | 版の履歴 |
|---|---|---|---|---|
| `pointCount` | integer | 必須 | 地点の件数。正の整数。地点データ版6では `points` 配列の件数、版5以前ではトップレベルの配列の件数と同じ | [版1] 元の項目を追加<br>[版5] `mountainCount` から改名 |
| `latestPointTimestamp` | string または null | 任意 | 収録地点の元データ上の最終編集日時の最大値。現地調査日や生成日時ではない。`sourceTimestamp` があるときは、それより後にしない | [版3] 元の項目を追加<br>[版5] `latestMountainTimestamp` から改名し任意化 |

これらの項目には、ほかの種類の件数や日時を入れない。共通スキーマ内の地点データ用の定義（`pointManifest`）で検証する。日時の前後関係や、本体の大きさ・ハッシュ・件数はスキーマでは確かめられないので、別に確かめる。

## 読み込みと検証

利用するアプリは対応する `schemaVersion` を確認し、`downloadUrl` から本体を取得する。展開前に `sizeBytes` と `sha256`、展開後に `uncompressedSizeBytes` を照合する。データ種別ごとの件数なども各仕様に従って照合する。地点データの件数は、manifest 版4では `mountainCount`、版5では `pointCount` を使う。通信や検証に失敗した場合は保存済みデータを維持する。

データセットと正式版・開発版ごとに最新版の manifest の URL を分ける。Release に添付する manifest と最新版として配置する manifest は同じ内容を使い、公開済みの版の本体は差し替えない。公開先の URL・Release タグ・ファイル名は各データセットの資料に記載する。

スキーマファイルは種類ごとに保存する。データ種別に共通する manifest は `schemas/manifest-v<版>.schema.json` に置き、データ本体などのスキーマはデータ種別のフォルダ（地点データは `points/schemas/`）に置く。それぞれ必要なときに独立して版を上げ、公開済みの版は原則変更しない。

## JSON の例

版5・版4は公開済み Release の例。版1〜3.1は版4の例を基に、履歴で確認できる項目の差を示した説明用の例で、実在する Release の manifest ではない。

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
  "attribution": "© OpenStreetMap contributors"
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

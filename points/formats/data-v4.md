# 地点データフォーマット (版4)

manifest の `schemaVersion` が `4` の版の形式を説明する。地点の形式には別の版の番号が無く、manifest の版4 に含まれる。この版は 2026-10-03 から配り始めた。版4 は山頂データだけが使った形式なので、配り方もこの文書に書く。

新しい形式は [版5](data-v5.md) を参照する。

## 概要

| 項目 | 内容 |
|---|---|
| 最新版の manifest | `https://shohei0205.github.io/yamamuki-data/peaks/manifest.json`（正式版）、`https://shohei0205.github.io/yamamuki-data/peaks-dev/manifest.json`（開発版） |
| Release のタグ | `peaks-<version>`（正式版）、`peaks-dev-<version>`（開発版） |
| データ本体のファイル名 | `japan-mountains.json.gz` |
| 最後に作った正式版の Release | `peaks-20261007T084736Z-37595744056-1` |
| 件数 | 14,035 |
| 元データ | Geofabrik の日本全国 PBF（2026-10-06 版） |
| ライセンス | ODbL-1.0、© OpenStreetMap contributors |

版4 は新しく生成しない。公開済みの Release は残すが、最新版の manifest（`peaks/manifest.json`）は `{"schemaVersion": 999}` に置き換えて公開を終える予定（Issue #48）。今の置き場所は [リリース済みアプリ向けの互換データ](../README.md#リリース済みアプリ向けの互換データ) を参照する。

## 読み方

1. 最新版の manifest を取る。
2. `schemaVersion` が `4` であることを確かめる。
3. `downloadUrl` からデータ本体を取り、`sizeBytes` と `sha256` を照合する。
4. gzip を展開し、展開後の大きさを `uncompressedSizeBytes` と、配列の件数を `mountainCount` と照合する。

## manifest

UTF-8 の JSON オブジェクト。次は、Release `peaks-20261007T084736Z-37595744056-1` に添付した `manifest.json` の実物。

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

すべての項目を必ず書く。

| 項目 | 型 | 内容 |
|---|---|---|
| `schemaVersion` | integer | 形式の版。`4` |
| `version` | string | データの版。生成時の UTC 日時・Actions の実行 ID・再実行番号をつないだもの |
| `fileName` | string | `japan-mountains.json.gz` |
| `downloadUrl` | string | データ本体の HTTPS URL。正式版は `peaks-<version>`、開発版は `peaks-dev-<version>` の Release を指す |
| `sha256` | string | gzip ファイルそのものの SHA-256（小文字の 16 進数 64 文字） |
| `sizeBytes` | integer | gzip ファイルのバイト数 |
| `uncompressedSizeBytes` | integer | 展開後の JSON のバイト数 |
| `mountainCount` | integer | 山の件数 |
| `sourceTimestamp` | string | 元の PBF の基準日時（ヘッダーの `osmosis_replication_timestamp`、UTC、ISO 8601） |
| `latestMountainTimestamp` | string | 収録した山頂ノードの OSM 上の最終編集日時の最大値。現地調査日や生成日時ではない |
| `sourceUrl` | string | 実際に取得した日付付きの PBF の URL |
| `license` | string | `ODbL-1.0` |
| `attribution` | string | `© OpenStreetMap contributors` |

## データ本体

gzip で圧縮した UTF-8 の JSON 配列で、1 要素が 1 つの山頂。gzip のヘッダーには日時とファイル名を入れない。並びは OSM ノード ID の昇順。すべての項目をすべての要素に書く（省略しない）。

次は、実際のデータの先頭の 1 件。

```json
{"osmId":165028676,"name":"奥穂高岳","latitude":36.289206,"longitude":137.6479807,"elevationM":3190.0,"nameReading":"おくほたかだけ","aliases":[],"wikipediaUrl":"https://ja.wikipedia.org/wiki/%E7%A9%82%E9%AB%98%E5%B2%B3","wikidataUrl":"https://www.wikidata.org/wiki/Q31702740"}
```

| 項目 | 型 | 内容 |
|---|---|---|
| `osmId` | integer | OSM ノード ID。データ内で重複しない |
| `name` | string | 表示名。`name:ja`、`name` の順に使い、前後の空白を除く。空にしない |
| `latitude` | number | WGS 84 の緯度（度） |
| `longitude` | number | WGS 84 の経度（度） |
| `elevationM` | number または null | 標高（m）。OSM の `ele` から読む。カンマ、m・ft などの単位、セミコロン区切りの先頭の値に対応し、解釈できなければ `null` |
| `nameReading` | string または null | 読み（`name:ja-Hira`）。推測で補わない |
| `aliases` | string の配列 | 別名（`alt_name:ja`、`alt_name` をセミコロンで分けたもの）。表示名と同じ名前・重複は除く。無ければ `[]` |
| `wikipediaUrl` | string または null | OSM の `wikipedia`（言語:記事名）から作った HTTPS URL |
| `wikidataUrl` | string または null | OSM の `wikidata`（例: `Q39231`）から作った HTTPS URL |

収録するのは、OSM の `natural=peak` または `natural=volcano` の、名前の付いたノード。way・relation と名前の無いノードは入れない。

## 出典とライセンス

OpenStreetMap のデータから作り、ODbL 1.0 で配っている。使うときは「© OpenStreetMap contributors」を表示する。

## 変更履歴

版1〜3 は、最初の生成・公開処理（#6、2026-10-01 マージ）の中で順に作られた。版1〜3 の Release は今は残っていない。

### 版4（2026-10-03、#26）

- manifest に `downloadUrl` を足した。版3 までは、読み手が `https://github.com/shohei0205/yamamuki-data/releases/download/peaks-<version>/<fileName>` を自分で組み立てる必要があった。
- 新しく生成する版4 では `downloadUrl` を必須にし、公開前にリポジトリ・版・配布先・ファイル名と合っているかを確かめる。
- 地点の項目は版3 と同じ。

### 版3

- manifest に `latestMountainTimestamp`（収録した山頂ノードの OSM 上の最終編集日時の最大値）を足した。
- 収録する山頂ノードに日時が無い、形式が正しくない、元データの基準日時より新しい場合は、生成を止める。
- 地点の項目は版2 と同じ。
- 版3 のまま、#12（2026-10-01）で `sourceUrl` の値を `japan-latest.osm.pbf` から実際に取得した日付付きの URL（例: `japan-261006.osm.pbf`）に変えた。項目は変わっていないので、版は上げていない。

### 版2

- 地点に `nameReading`・`aliases`・`wikipediaUrl`・`wikidataUrl` を足した。名前・位置・標高はそのまま。
- リンク先の記事の本文は入れない（見るには通信が要る）。

### 版1

- manifest: `schemaVersion`・`version`・`fileName`・`sha256`・`sizeBytes`・`uncompressedSizeBytes`・`mountainCount`・`sourceTimestamp`、それに出典の `sourceUrl`・`license`・`attribution`。
- 地点: `osmId`・`name`・`latitude`・`longitude`・`elevationM` の 5 つ。
- 版1 の仕様を書いた資料は残っていない。上の項目は、#6 の検査処理（`peaks/scripts/check_release.py` の `validate`）が版1 に求める項目から読み取ったもの。


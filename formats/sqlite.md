# 山むきSQLiteコンテナ 版1

一つのファイルに地表、地形、地点のデータセットを任意の組み合わせで収録する。ここではコンテナの識別、データセット台帳、共通メタデータ、配布manifest、統合時の規則を定める。

## ファイルを開く

1. SQLiteを読み取り専用で開く。配布物は単体のSQLiteファイルで、別ファイルのWALは必要ない。SQLiteのページサイズはライブラリに判定させる。
2. `PRAGMA application_id` が `1498235970`（`0x594D4442`、YMDB）、`PRAGMA user_version` が `1` であることを確かめる。未知のコンテナ版は対応外として扱う。
3. 次のクエリで一覧を取得する。複数の種別が入っていても、対応しているデータセットだけ選択できるようにする。

```sql
SELECT dataset_key, id, kind, title, data_version, format_version, metadata_json
FROM datasets
ORDER BY id;
```

| 列 | SQLite型 | 読み手での使い方 |
| --- | --- | --- |
| `dataset_key` | INTEGER | ファイル内だけで有効な主キー。全データ行の検索に使う |
| `id` | TEXT | コンテナ内で一意のデータセットID |
| `kind` | TEXT | `surface`、`terrain`、`points` |
| `title` | TEXT | 選択画面に表示する名前 |
| `data_version` | TEXT | 収録内容の版 |
| `format_version` | INTEGER | 種別ごとの格納形式の版 |
| `metadata_json` | TEXT | 共通メタデータを格納したJSONオブジェクト |

形式の対応可否は `(kind, format_version)` で判定する。コンテナの版、データセットの格納形式の版、データ版は独立している。統合時に `dataset_key` は振り直される。別ファイルへ引き継ぐ選択設定は `id` で保存し、キャッシュにはファイルの識別子も含める。

## 出典と作成者

`metadata_json` は地点JSONと同じ [共通metadata](metadata.md) のオブジェクト。表示に必要な項目は次の通り。

- `authors[]` の `name`、`role`、`url` は、この配布データを編集・加工した主体。
- `sources[]` の `name` はデータセット内で一意の出典名。`source`、`license`、`attribution`、`changes`、`latestTimestamp` は任意。`latestTimestamp` は元資料に由来する収録内容の更新日時。
- 地点の `tags[]` はタグの説明、`pointCount` は収録地点数。地表・地形には `pointCount` を記録しない。未知のキーは読み飛ばせる。

## 配布manifest

[共通manifest版6](manifest.md) に従う。 `dataSchemaFormat` は `yamamuki-sqlite`、`dataSchemaVersion` はコンテナの `PRAGMA user_version`。SQLiteをそのまま配布するので `sizeBytes` と `uncompressedSizeBytes` は同じ値。 `sha256` はSQLite全体のハッシュ。

ESA地表の `data_version` は、ローカル生成時は `20261110T032217Z` のようなUTC日時の仮識別子、加工・検査Action完了後は `20261113T010203Z-12345-1` のようなUTC日時・Run ID・試行番号の確定識別子とし、セマンティックバージョン番号は使わない。形式の版は `format_version` と `user_version` で別に管理する。

単一データセットの配布では `version` に台帳の `data_version`、`name` に `title`、`metadata` に `metadata_json` を使う。追加項目の `datasets[]` に台帳の `id`・`kind`・`title`・`dataVersion`・`formatVersion`・`metadata` を載せられる。地表の収録レイヤーとズームはSQLiteの `surface_layers` と `surface_levels` を参照する。複数データセットを統合したファイルの公開用メタデータと配布版の決め方は今後定める。

## 地表の収録形式

`kind = 'surface'` のテーブル、タイル座標、LOD、復号と合成は [地表画像のSQLite形式 版1](../surface/formats/sqlite.md) を参照する。

## 地点の収録枠

> **設計案**。この節は検討中の収録形式を示す。正式仕様の確定時に、テーブル構造や読み込み規則が変わる可能性がある。

`point_records(dataset_key, ordinal, point_id, payload_json)` の主キーは `(dataset_key, ordinal)`。`ordinal` は元の順序を表す0以上の整数。`point_id` はデータセット内で一意で、JSON内の `id` と一致する。

```sql
SELECT point_id, payload_json
FROM point_records
WHERE dataset_key = :dataset
ORDER BY ordinal;
```

地点JSON版6を格納するとき、`payload_json` は `points[]` の1件と同じJSONオブジェクト。表示には少なくとも `id`、`name`、WGS 84の度単位の `latitude`・`longitude` を使う。`elevationM` はメートル、`nameReading` は文字列または文字列配列。その他の項目は [地点JSON仕様](../points/formats/data.md) に従う。ビューワーはこの設計案の地点JSON版6の収録枠を読み取り、地点ファイルの情報から台帳とメタデータを表示できる。コンテナへ地点JSONを取り込むコマンドは今後の工程。

## 地形の収録形式

`terrain_levels(dataset_key,level_key,z,settings_json)` と `terrain_tiles(dataset_key,level_key,x,y,encoding,payload)` を使う。形式版1の座標・制御点・差分参照・復号・表示は [地形のSQLite形式](../terrain/formats/sqlite.md) を参照する。地形BLOBはコンテナのdataset_keyに依存しないため、地表・地点と同じSQLiteへ統合できる。

## 統合時の互換性

統合先は新規ファイルとし、重複するデータセットIDはエラー。`dataset_key` を振り直し、レイヤー・レベル・地点の順序と各データセットのメタデータを保つ。画像・地形の圧縮BLOBと地点のJSON本文をコピーするため、コーデックの内部にコンテナ固有の整数キーを埋め込まない。

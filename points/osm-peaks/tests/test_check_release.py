"""件数の境界・地域の欠落・ファイル破損の判定を確かめる。"""

import copy
import gzip
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from scripts.build_data import FILE_NAME, write_distribution
from scripts.check_release import assess, validate, report
from document_metadata import metadata_maps


def dataset(count=10000, latitude=35):
    return ({"version": "test", "schemaVersion": 2, "sourceTimestamp": "2026-09-29T20:22:51Z"},
            [{"id": f"osm:{i + 1}", "name": "山", "latitude": latitude, "longitude": 139,
              "elevationM": None, "nameReading": None, "aliases": [], "wikipediaUrl": None,
              "wikidataUrl": None} for i in range(count)])


class CheckReleaseTests(unittest.TestCase):
    def test_report_separates_sources_from_checks_and_regions(self):
        current = dataset(1)
        current[0].update(sizeBytes=100, sha256='abc')
        contents = report(current, None, [])
        self.assertIn('\n## 出典\n\n© OpenStreetMap contributors', contents)
        self.assertNotIn('元データ: [Geofabrik]', contents)
        self.assertIn('\n## 追加範囲の山頂\n', contents)
        self.assertLess(contents.index('### 確認事項'), contents.index('## 出典'))

    def test_report_includes_all_metadata_sources(self):
        current = dataset(1)
        sources = [
            {"name": "osm-peaks", "attribution": "© OpenStreetMap contributors", "license": "ODbL-1.0"},
            {"name": "国土地理院", "attribution": "国土地理院のデータを加工", "license": "PDL-1.0"},
            {"name": "日本百名山", "attribution": "Wikipedia の投稿者", "license": "CC BY-SA 4.0"},
        ]
        for version in (5, 6):
            with self.subTest(version=version):
                current[0].update(schemaVersion=version, sizeBytes=100, sha256='abc')
                if version == 6:
                    current[0].pop('sources', None)
                    current[0]['metadata'] = {'sources': sources}
                else:
                    current[0]['sources'] = metadata_maps({'sources': sources})['sources']
                contents = report(current, None, [])
                section = contents.split('## 出典\n\n', 1)[1].split('## 追加範囲の山頂', 1)[0]
                for source in sources:
                    self.assertIn(f"- {source['name']}: {source['attribution']} / {source['license']}", section)
                self.assertEqual(section.count('- osm-peaks:'), 1)

    def test_latest_timestamp_in_array_sources(self):
        before, after = dataset(), dataset()
        for manifest, _ in (before, after):
            manifest.update(schemaVersion=6, dataTimestamp="2026-09-30T20:00:00Z",
                            metadata={"sources": [{"name": "osm-peaks", "latestTimestamp": "2026-09-29T08:00:00Z"}]})
        self.assertTrue(any("最新編集日時が前回公開版と同じ" in w for w in assess(after, before)))
        after[0]["metadata"]["sources"][0]["latestTimestamp"] = "2026-09-29T08:00:01Z"
        self.assertEqual([], assess(after, before))

    def test_normal_and_minimum_boundary(self):
        self.assertEqual([], assess(dataset(), dataset()))
        self.assertTrue(any("最低目安" in w for w in assess(dataset(9999), dataset())))

    def test_first_release_requires_review(self):
        self.assertTrue(any("初回" in w for w in assess(dataset())))

    def test_same_latest_mountain_timestamp_requires_review(self):
        before, after = dataset(), dataset()
        before[0].update(schemaVersion=3, latestMountainTimestamp="2026-09-29T08:00:00Z")
        after[0].update(schemaVersion=3, latestMountainTimestamp="2026-09-29T17:00:00+09:00",
                        sourceTimestamp="2026-09-30T20:00:00Z", version="new")
        self.assertTrue(any("最新編集日時が前回公開版と同じ" in w for w in assess(after, before)))
        after[0]["latestMountainTimestamp"] = "2026-09-29T08:00:01Z"
        self.assertEqual([], assess(after, before))

    def test_latest_timestamp_comparison_across_manifest_versions(self):
        before, after = dataset(), dataset()
        before[0].update(schemaVersion=4, latestMountainTimestamp="2026-09-29T08:00:00Z")
        after[0].update(schemaVersion=5, latestPointTimestamp="2026-09-29T08:00:00Z")
        self.assertTrue(any("最新編集日時が前回公開版と同じ" in warning for warning in assess(after, before)))
        after[0]["latestPointTimestamp"] = "2026-09-29T08:00:01Z"
        self.assertFalse(any("最新編集日時が前回公開版と同じ" in warning for warning in assess(after, before)))

    def test_missing_old_timestamp_is_not_equal(self):
        before, after = dataset(), dataset()
        after[0].update(schemaVersion=3, latestMountainTimestamp="2026-09-29T08:00:00Z")
        warnings = assess(after, before)
        self.assertFalse(any("最新編集日時が前回公開版と同じ" in w for w in warnings))
        self.assertTrue(any("schemaVersion" in w for w in warnings))

    def test_twenty_percent_boundary(self):
        self.assertEqual([], assess(dataset(10001), dataset(12500)))
        self.assertTrue(any("全国の件数が20%" in w for w in assess(dataset(10000), dataset(12500))))

    def test_regional_loss_despite_unchanged_total(self):
        before = dataset()
        after = copy.deepcopy(before)
        for row in before[1][:20]:
            row["latitude"] = 43
        self.assertTrue(any("北緯43" in w for w in assess(after, before)))
        before[1][19]["latitude"] = 35
        self.assertEqual([], assess(after, before))

    def test_old_source_and_schema_change(self):
        before, after = dataset(), dataset()
        after[0]["sourceTimestamp"] = "2026-09-28T20:22:51Z"
        after[0]["schemaVersion"] = 1
        warnings = assess(after, before)
        self.assertTrue(any("古く" in w for w in warnings))
        self.assertTrue(any("schemaVersion" in w for w in warnings))

    def test_integrity_checks_cannot_be_bypassed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rows = dataset(2)[1]
            write_distribution(rows, root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
            self.assertEqual(rows, validate(root, tag="osm-peaks-test")[1])
            write_distribution(rows, root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z", channel="dev")
            self.assertEqual(rows, validate(root, tag="osm-peaks-dev-test", channel="dev")[1])
            write_distribution(rows, root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
            with self.assertRaises(ValueError):
                validate(root, tag="osm-peaks-dev-test", channel="stable")
            with self.assertRaises(ValueError):
                validate(root, tag="osm-peaks-other")
            manifest_path = root / "manifest.json"
            original = manifest_path.read_text(encoding="utf-8")
            for key, value in [("sha256", "0" * 64), ("pointCount", 10000),
                               ("sizeBytes", 1), ("uncompressedSizeBytes", 1), ("schemaVersion", 99)]:
                with self.subTest(key=key):
                    manifest = json.loads(original)
                    manifest[key] = value
                    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
                    with self.assertRaises(ValueError):
                        validate(root)
            manifest_path.write_text(original, encoding="utf-8")
            (root / FILE_NAME).write_bytes(b"broken")
            with self.assertRaises(ValueError):
                validate(root)

    def test_latest_timestamp_validation_and_older_schema(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_distribution(dataset(1)[1], root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
            path = root / "manifest.json"
            original = json.loads(path.read_text(encoding="utf-8"))
            for value in (None, "bad", "2026-09-29T12:00:00", "2026-09-30T00:00:00Z"):
                with self.subTest(value=value):
                    manifest = {**original, "metadata": {**original["metadata"], "sources": {**metadata_maps(original["metadata"])["sources"], "osm-peaks": {**metadata_maps(original["metadata"])["sources"]["osm-peaks"], "latestTimestamp": value}}}}
                    path.write_text(json.dumps(manifest), encoding="utf-8")
                    with self.assertRaises(ValueError):
                        validate(root)
            # 旧版のデータ本体も旧形式で作り直して検査する。
            import gzip
            import hashlib
            rows = dataset(1)[1]
            rows[0]["osmId"] = 1
            del rows[0]["id"]
            raw = json.dumps(rows).encode("utf-8")
            archive = gzip.compress(raw)
            (root / FILE_NAME).write_bytes(archive)
            original.update(sizeBytes=len(archive), uncompressedSizeBytes=len(raw),
                            sha256=hashlib.sha256(archive).hexdigest())
            original.update(original.pop("metadata"))
            original["sourceTimestamp"] = "2026-09-29T20:22:51Z"
            original["schemaVersion"] = 2
            original["dataSchemaVersion"] = 2
            original.pop("latestPointTimestamp", None)
            original["mountainCount"] = original.pop("pointCount")
            path.write_text(json.dumps(original), encoding="utf-8")
            self.assertEqual(2, validate(root)[0]["schemaVersion"])

    def test_manifest_data_schema_format_is_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            manifest=write_distribution(dataset(1)[1],root,'test','2026-09-29T20:22:51Z','2026-09-29T12:00:00Z')
            self.assertEqual(manifest['dataSchemaFormat'],'yamamuki-points')
            for value in (None,'yamamuki-manifest','yamamuki-point-supplements',123):
                changed=dict(manifest)
                if value is None:changed.pop('dataSchemaFormat')
                else:changed['dataSchemaFormat']=value
                (root/'manifest.json').write_text(json.dumps(changed),encoding='utf-8')
                with self.subTest(value=value),self.assertRaises(ValueError):validate(root)

    def test_manifest_schema_format_is_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            manifest=write_distribution(dataset(1)[1],root,'test','2026-09-29T20:22:51Z','2026-09-29T12:00:00Z')
            self.assertEqual(manifest['schemaFormat'],'yamamuki-manifest')
            for value in (None,'yamamuki-points','yamamuki-point-supplements',123):
                changed=dict(manifest)
                if value is None:changed.pop('schemaFormat')
                else:changed['schemaFormat']=value
                (root/'manifest.json').write_text(json.dumps(changed),encoding='utf-8')
                with self.subTest(value=value),self.assertRaises(ValueError):validate(root)

    def test_data_timestamp_is_required_and_rejects_invalid_dates(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            manifest=write_distribution(dataset(1)[1],root,'test','2026-09-29T20:22:51Z','2026-09-29T12:00:00Z')
            for value in (None,123,'not-a-date','2026-02-30T00:00:00Z','2026-10-14T00:00:00','2026-10-14T00:00:00+09:00'):
                changed={**manifest,'dataTimestamp':value}
                (root/'manifest.json').write_text(json.dumps(changed),encoding='utf-8')
                with self.subTest(value=value),self.assertRaises(ValueError):validate(root)

    def test_data_schema_version_is_checked_independently(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rows = dataset(1)[1]
            manifest = write_distribution(rows, root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
            self.assertEqual(6, manifest["dataSchemaVersion"])
            missing = {key: value for key, value in manifest.items() if key != "dataSchemaVersion"}
            (root / "manifest.json").write_text(json.dumps(missing), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "dataSchemaVersion"):
                validate(root)
            for value in (None, True, "5", 0, 99):
                (root / "manifest.json").write_text(json.dumps(dict(manifest, dataSchemaVersion=value)), encoding="utf-8")
                with self.subTest(value=value), self.assertRaises(ValueError):
                    validate(root)
            # manifest の版が同じでも、本体の版が旧形式なら文字列 id を受け付けない。
            (root / "manifest.json").write_text(json.dumps(dict(manifest, dataSchemaVersion=2)), encoding="utf-8")
            with self.assertRaises(ValueError):
                validate(root)
            old_rows = dataset(1)[1]
            old_rows[0]["osmId"] = 1
            del old_rows[0]["id"]
            old_manifest = write_distribution(old_rows, root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
            old_manifest["dataSchemaVersion"] = 2
            old_raw = (json.dumps(old_rows,ensure_ascii=False,separators=(',',':'))+'\n').encode()
            old_gzip = gzip.compress(old_raw,mtime=0)
            (root / FILE_NAME).write_bytes(old_gzip)
            old_manifest.update(sha256=hashlib.sha256(old_gzip).hexdigest(),sizeBytes=len(old_gzip),uncompressedSizeBytes=len(old_raw))
            (root / "manifest.json").write_text(json.dumps(old_manifest), encoding="utf-8")
            self.assertEqual(old_rows, validate(root)[1])
            write_distribution(rows, root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
            legacy = dict(manifest)
            legacy.update(legacy.pop("metadata"))
            legacy["latestPointTimestamp"] = metadata_maps(legacy)["sources"]["osm-peaks"]["latestTimestamp"]
            legacy["sourceTimestamp"] = "2026-09-29T20:22:51Z"
            legacy["schemaVersion"] = 5
            del legacy["dataSchemaVersion"]
            legacy_rows = [{**row,"id":row["id"].removeprefix("osm:")} for row in rows]
            legacy_raw = (json.dumps(legacy_rows,ensure_ascii=False,separators=(',',':'))+'\n').encode()
            legacy_gzip = gzip.compress(legacy_raw,mtime=0)
            (root / FILE_NAME).write_bytes(legacy_gzip)
            legacy.update(sha256=hashlib.sha256(legacy_gzip).hexdigest(),sizeBytes=len(legacy_gzip),uncompressedSizeBytes=len(legacy_raw))
            (root / "manifest.json").write_text(json.dumps(legacy), encoding="utf-8")
            self.assertEqual(legacy_rows, validate(root)[1])
        before, after = dataset(), dataset()
        before[0]["dataSchemaVersion"] = 2
        after[0]["dataSchemaVersion"] = 5
        self.assertTrue(any("dataSchemaVersion" in warning for warning in assess(after, before)))

    def test_optional_point_type_and_future_types(self):
        with tempfile.TemporaryDirectory() as directory:
            for value in ("peak", "parking", "trailhead", "landmark", "mountain_hut", "campsite", "water_source", "toilet", "viewpoint", "pass", "junction", "future_type", None, "", "Peak", " peak", 1, []):
                rows = dataset(1)[1]
                rows[0]["type"] = value
                write_distribution(rows, directory, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
                with self.subTest(value=value):
                    if value in ("peak", "parking", "trailhead", "landmark", "mountain_hut", "campsite", "water_source", "toilet", "viewpoint", "pass", "junction", "future_type", None):
                        self.assertEqual(rows, validate(directory)[1])
                    else:
                        with self.assertRaises(ValueError):
                            validate(directory)

    def test_ids_and_related_ids(self):
        with tempfile.TemporaryDirectory() as directory:
            valid = [{}, {"relatedIds": []}, {"relatedIds": ["gsi-vector:sample", "custom:sample"]}]
            invalid = [{"id": value} for value in (None, 1, "", "0", "1", "01", "osm:0", "osm:01", "osm:way:1", "curated:landmark:test")]
            invalid += [{"relatedIds": value} for value in (None, "osm:2", [1], [""], [" osm:2"], ["osm:2","osm:2"], ["osm:1"])]
            for index, changes in enumerate(valid + invalid):
                with self.subTest(changes=changes):
                    rows = dataset(1)[1]
                    rows[0].update(changes)
                    if index < len(valid):
                        write_distribution(rows, directory, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
                        self.assertEqual(rows, validate(directory)[1])
                    else:
                        with self.assertRaises(ValueError):
                            write_distribution(rows, directory, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
                            validate(directory)
            rows = dataset(2)[1]
            rows[1]["id"] = rows[0]["id"]
            write_distribution(rows, directory, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
            with self.assertRaises(ValueError):
                validate(directory)

    def test_optional_details_can_be_omitted_but_invalid_values_fail(self):
        fields = ("elevationM", "nameReading", "aliases", "wikipediaUrl", "wikidataUrl")
        with tempfile.TemporaryDirectory() as directory:
            for omitted in [(field,) for field in fields] + [fields]:
                with self.subTest(omitted=omitted):
                    rows = dataset(1)[1]
                    for field in omitted:
                        del rows[0][field]
                    write_distribution(rows, directory, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
                    self.assertEqual(rows, validate(directory)[1])
            for field, value in (("elevationM", "high"), ("nameReading", 1), ("aliases", "別名"),
                                 ("wikipediaUrl", []), ("wikidataUrl", False)):
                with self.subTest(field=field):
                    rows = dataset(1)[1]
                    rows[0][field] = value
                    with self.assertRaises(ValueError):
                        write_distribution(rows, directory, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
                        validate(directory)

    def test_bad_rows_rejected_even_with_valid_checksum(self):
        with tempfile.TemporaryDirectory() as directory:
            for changes in ({"id": "0"}, {"name": ""}, {"latitude": 91}, {"aliases": "別名"}):
                with self.subTest(changes=changes):
                    rows = dataset(1)[1]
                    rows[0].update(changes)
                    with self.assertRaises(ValueError):
                        write_distribution(rows, directory, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
                        validate(directory)


class DownloadUrlTests(unittest.TestCase):
    def test_urls_match_channel_and_reject_invalid_targets(self):
        for channel, tag in (("stable", "osm-peaks-test"), ("dev", "osm-peaks-dev-test")):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                manifest = write_distribution(dataset(2)[1], root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z", channel=channel)
                expected = f"https://github.com/shohei0205/yamamuki-data/releases/download/{tag}/{FILE_NAME}"
                self.assertEqual(expected, manifest["downloadUrl"])
                validate(root, tag=tag, channel=channel)
                for value in (None, "https://example.com/data.gz", expected.replace("test", "other"), expected.replace("https:", "http:")):
                    changed = dict(manifest, downloadUrl=value)
                    (root / "manifest.json").write_text(json.dumps(changed), encoding="utf-8")
                    with self.assertRaises(ValueError):
                        validate(root, tag=tag, channel=channel)
                changed = dict(manifest)
                del changed["downloadUrl"]
                (root / "manifest.json").write_text(json.dumps(changed), encoding="utf-8")
                with self.assertRaises(ValueError):
                    validate(root, channel=channel)
                # downloadUrl を省略できる旧版は、データ本体も旧形式を使う。
                import gzip
                import hashlib
                rows = dataset(2)[1]
                for index, row in enumerate(rows, 1):
                    row["osmId"] = index
                    del row["id"]
                raw = json.dumps(rows).encode("utf-8")
                archive = gzip.compress(raw)
                (root / FILE_NAME).write_bytes(archive)
                changed["dataSchemaVersion"] = 3
                changed.update(changed.pop("metadata"))
                changed["latestPointTimestamp"] = metadata_maps(changed)["sources"]["osm-peaks"]["latestTimestamp"]
                changed["sourceTimestamp"] = "2026-09-29T20:22:51Z"
                changed["mountainCount"] = changed.pop("pointCount")
                changed["latestMountainTimestamp"] = changed.pop("latestPointTimestamp")
                changed.update(schemaVersion=3, sizeBytes=len(archive),
                               uncompressedSizeBytes=len(raw), sha256=hashlib.sha256(archive).hexdigest())
                (root / "manifest.json").write_text(json.dumps(changed), encoding="utf-8")
                validate(root, channel=channel)

"""件数の境界・地域の欠落・ファイル破損の判定を確かめる。"""

import copy
import json
from pathlib import Path
import tempfile
import unittest

from scripts.build_data import FILE_NAME, write_distribution
from scripts.check_release import assess, validate


def dataset(count=10000, latitude=35):
    return ({"version": "test", "schemaVersion": 2, "sourceTimestamp": "2026-09-29T20:22:51Z"},
            [{"id": str(i + 1), "name": "山", "latitude": latitude, "longitude": 139,
              "elevationM": None, "nameReading": None, "aliases": [], "wikipediaUrl": None,
              "wikidataUrl": None} for i in range(count)])


class CheckReleaseTests(unittest.TestCase):
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
            self.assertEqual(rows, validate(root, tag="peaks-test")[1])
            write_distribution(rows, root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z", channel="dev")
            self.assertEqual(rows, validate(root, tag="peaks-dev-test", channel="dev")[1])
            write_distribution(rows, root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
            with self.assertRaises(ValueError):
                validate(root, tag="peaks-dev-test", channel="stable")
            with self.assertRaises(ValueError):
                validate(root, tag="peaks-other")
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
                    manifest = {**original, "latestPointTimestamp": value}
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
            original["schemaVersion"] = 2
            original["dataSchemaVersion"] = 2
            del original["latestPointTimestamp"]
            original["mountainCount"] = original.pop("pointCount")
            path.write_text(json.dumps(original), encoding="utf-8")
            self.assertEqual(2, validate(root)[0]["schemaVersion"])

    def test_data_schema_version_is_checked_independently(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rows = dataset(1)[1]
            manifest = write_distribution(rows, root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
            self.assertEqual(5, manifest["dataSchemaVersion"])
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
            (root / "manifest.json").write_text(json.dumps(old_manifest), encoding="utf-8")
            self.assertEqual(old_rows, validate(root)[1])
            write_distribution(rows, root, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
            legacy = dict(manifest)
            del legacy["dataSchemaVersion"]
            (root / "manifest.json").write_text(json.dumps(legacy), encoding="utf-8")
            self.assertEqual(rows, validate(root)[1])
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
                    if value in ("peak", "parking", "trailhead", "landmark", "mountain_hut", "campsite", "water_source", "toilet", "viewpoint", "pass", "junction", "future_type"):
                        self.assertEqual(rows, validate(directory)[1])
                    else:
                        with self.assertRaises(ValueError):
                            validate(directory)

    def test_ids_and_optional_osm_id(self):
        with tempfile.TemporaryDirectory() as directory:
            valid = [{}, {"osmId": 1}]
            invalid = [{"id": value} for value in (None, 1, "", "0", "01", "osm:way:1", "curated:landmark:test")]
            invalid += [{"osmId": value} for value in (None, True, 0, -1, "1", 2)]
            for index, changes in enumerate(valid + invalid):
                with self.subTest(changes=changes):
                    rows = dataset(1)[1]
                    rows[0].update(changes)
                    write_distribution(rows, directory, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
                    if index < len(valid):
                        self.assertEqual(rows, validate(directory)[1])
                    else:
                        with self.assertRaises(ValueError):
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
            for field, value in (("elevationM", "high"), ("nameReading", 1), ("aliases", None),
                                 ("wikipediaUrl", []), ("wikidataUrl", False)):
                with self.subTest(field=field):
                    rows = dataset(1)[1]
                    rows[0][field] = value
                    write_distribution(rows, directory, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
                    with self.assertRaises(ValueError):
                        validate(directory)

    def test_bad_rows_rejected_even_with_valid_checksum(self):
        with tempfile.TemporaryDirectory() as directory:
            for changes in ({"id": "0"}, {"name": ""}, {"latitude": 91}, {"aliases": "別名"}):
                with self.subTest(changes=changes):
                    rows = dataset(1)[1]
                    rows[0].update(changes)
                    write_distribution(rows, directory, "test", "2026-09-29T20:22:51Z", "2026-09-29T12:00:00Z")
                    with self.assertRaises(ValueError):
                        validate(directory)


class DownloadUrlTests(unittest.TestCase):
    def test_urls_match_channel_and_reject_invalid_targets(self):
        for channel, tag in (("stable", "peaks-test"), ("dev", "peaks-dev-test")):
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
                changed["mountainCount"] = changed.pop("pointCount")
                changed["latestMountainTimestamp"] = changed.pop("latestPointTimestamp")
                changed.update(schemaVersion=3, sizeBytes=len(archive),
                               uncompressedSizeBytes=len(raw), sha256=hashlib.sha256(archive).hexdigest())
                (root / "manifest.json").write_text(json.dumps(changed), encoding="utf-8")
                validate(root, channel=channel)

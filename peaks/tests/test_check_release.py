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
            [{"osmId": i + 1, "name": "山", "latitude": latitude, "longitude": 139,
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
            for key, value in [("sha256", "0" * 64), ("mountainCount", 10000),
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
                    manifest = {**original, "latestMountainTimestamp": value}
                    path.write_text(json.dumps(manifest), encoding="utf-8")
                    with self.assertRaises(ValueError):
                        validate(root)
            original["schemaVersion"] = 2
            del original["latestMountainTimestamp"]
            path.write_text(json.dumps(original), encoding="utf-8")
            self.assertEqual(2, validate(root)[0]["schemaVersion"])

    def test_bad_rows_rejected_even_with_valid_checksum(self):
        with tempfile.TemporaryDirectory() as directory:
            for changes in ({"osmId": 0}, {"name": ""}, {"latitude": 91}, {"aliases": "別名"}):
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
                changed["schemaVersion"] = 3
                (root / "manifest.json").write_text(json.dumps(changed), encoding="utf-8")
                validate(root, channel=channel)

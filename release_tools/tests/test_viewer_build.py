"""生成日時による配布名と再実行の識別を確認する。"""

from datetime import datetime, timedelta, timezone
import unittest

from release_tools.viewer_build import plan


class ViewerBuildTests(unittest.TestCase):
    def test_utc_date_and_automatic_bundle_version(self):
        now = datetime(2026, 11, 13, 8, 45, 6, tzinfo=timezone(timedelta(hours=9)))
        self.assertEqual({
            "tag": "viewer-20261112T234506Z-123-1",
            "title": "Yamamuki Data Viewer 20261112T234506Z-123-1",
            "version": "2026.11.12",
        }, plan(now, "123", "1"))

    def test_same_second_and_rerun_have_distinct_tags(self):
        now = datetime(2026, 11, 12, tzinfo=timezone.utc)
        tags = {plan(now, run, attempt)["tag"]
                for run, attempt in (("123", "1"), ("124", "1"), ("123", "2"))}
        self.assertEqual(3, len(tags))

"""地点タグの任意性と入力制約を検証する。"""

import unittest
from scripts.point_tags import validate_tags


class PointTagsTests(unittest.TestCase):
    def test_optional_empty_and_multiple_tags(self):
        for row in ({}, {"tags": []}, {"tags": ["日本百名山", "花の百名山"]}):
            with self.subTest(row=row):
                validate_tags([row])

    def test_rejects_invalid_tags(self):
        for tags in (None, "日本百名山", [1], [""], [" "], [" 日本百名山"], ["日本百名山 "], ["日本百名山", "日本百名山"]):
            with self.subTest(tags=tags), self.assertRaises(ValueError):
                validate_tags([{"tags": tags}])

    def test_preserves_spelling_and_order(self):
        row = {"tags": ["日本100名山", "日本百名山", "A", "a"]}
        before = list(row["tags"])
        validate_tags([row])
        self.assertEqual(row["tags"], before)

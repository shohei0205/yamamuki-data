"""入力値と未入力時の扱いを Summary に明示する。"""

import unittest

from release_tools.workflow_summary import render


class WorkflowSummaryTests(unittest.TestCase):
    def test_generation_date_and_scheduled_default(self):
        for inputs, expected in (({}, "未指定（latest を取得）"), ({"source_date": "2026-09-29"}, "2026-09-29")):
            with self.subTest(inputs=inputs):
                summary = render("generate", inputs, {"GITHUB_EVENT_NAME": "schedule", "GITHUB_REF_NAME": "main", "RELEASE_CHANNEL": "stable"})
                self.assertIn(expected, summary)
                self.assertIn("schedule", summary)
                self.assertIn("stable", summary)
                self.assertNotIn("reason", summary)

    def test_publish_explicit_and_omitted_inputs(self):
        summary = render("publish", {"tag": "peaks-dev-v1", "sha256": "a" * 64, "reason": "件数を確認"}, {})
        for expected in ("peaks-dev-v1", "a" * 64, "件数を確認"):
            self.assertIn(expected, summary)
        summary = render("publish", {"tag": "peaks-v1"}, {})
        self.assertIn("未指定（検査結果から自動取得）", summary)
        self.assertIn("未記入", summary)

    def test_user_input_cannot_break_table_or_insert_links(self):
        summary = render("publish", {"reason": "a|b\r\n# heading\n[link](url)<script>"}, {})
        self.assertIn("a&#124;b<br>\\# heading", summary)
        self.assertIn("\\[link\\]\\(url\\)&lt;script&gt;", summary)
        self.assertNotIn("\n# heading", summary)

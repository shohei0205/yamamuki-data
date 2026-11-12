"""生成日時による配布名と再実行の識別を確認する。"""

from datetime import datetime, timedelta, timezone
import unittest
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap

from release_tools.viewer_build import plan


class ViewerBuildTests(unittest.TestCase):
    def test_utc_date_and_automatic_bundle_version(self):
        now = datetime(2026, 11, 13, 8, 45, 6, tzinfo=timezone(timedelta(hours=9)))
        self.assertEqual({
            "tag": "viewer-20261112T234506Z-123-1",
            "title": "YamamukiDataViewer-20261112T234506Z-123-1",
            "version": "2026.11.12",
        }, plan(now, "123", "1"))

    def test_same_second_and_rerun_have_distinct_tags(self):
        now = datetime(2026, 11, 12, tzinfo=timezone.utc)
        tags = {plan(now, run, attempt)["tag"]
                for run, attempt in (("123", "1"), ("124", "1"), ("123", "2"))}
        self.assertEqual(3, len(tags))

    def test_draft_workflow_does_not_create_git_tag(self):
        bash = shutil.which("bash")
        if os.name == "nt":
            candidate = Path("C:/Program Files/Git/bin/bash.exe")
            bash = str(candidate) if candidate.exists() else bash
        if not bash:
            self.skipTest("bashが必要です")
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/release-native-viewer.yml").read_text(encoding="utf-8-sig")
        step = workflow.split("      - name: 2環境のZIPを添付して下書きを作成\n", 1)[1]
        script = textwrap.dedent(step.split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            (directory / "dist").mkdir()
            tag = "viewer-test"
            for platform in ("windows-x64", "macos-arm64"):
                (directory / "dist" / f"{tag}-{platform}.zip").write_bytes(b"test")
            log = directory / "commands.log"
            env = dict(os.environ, GH_REPO="owner/repo", RELEASE_TAG=tag,
                       RELEASE_TITLE="Viewer", GITHUB_SHA="a" * 40,
                       GITHUB_STEP_SUMMARY=(directory / "summary.md").as_posix(),
                       YAMAMUKI_TEST_GH_LOG=log.as_posix())
            mock = r'''
set -euo pipefail
gh() {
  printf '%q ' "$@" >> "$YAMAMUKI_TEST_GH_LOG"
  printf '\n' >> "$YAMAMUKI_TEST_GH_LOG"
  if [[ "$1" == api && "$2" == "repos/$GH_REPO/git/matching-refs/tags/$RELEASE_TAG" ]]; then return 0; fi
  if [[ "$1" == release && "$2" == create ]]; then return 0; fi
  echo "許可していないGitHub操作: $*" >&2
  return 1
}
git() { echo "Draft作成時のGit操作は禁止" >&2; return 1; }
'''
            result = subprocess.run([bash, "-c", mock + script], cwd=directory,
                                    env=env, capture_output=True, text=True)
            self.assertEqual(0, result.returncode, result.stderr)
            calls = log.read_text(encoding="utf-8").splitlines()
            self.assertEqual(2, len(calls))
            self.assertTrue(calls[0].startswith("api repos/owner/repo/git/matching-refs/tags/"))
            self.assertTrue(calls[1].startswith("release create viewer-test "))
            self.assertIn("--draft", calls[1])
            self.assertIn("--title draft-viewer-test", calls[1])
            self.assertIn("--target " + "a" * 40, calls[1])
            self.assertNotIn("--verify-tag", calls[1])

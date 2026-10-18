"""共通形式の地点データを、外部通信や公開をせずに検証する。"""

from copy import deepcopy
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from release_tools import point_release as target
from scripts.point_document import document


class PointReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        environment = patch.dict(os.environ, GH_REPO="owner/repo", GITHUB_ACTIONS="false",
                                 GITHUB_ACTOR="tester", PAGES_DIRECTORY=str(self.root / "pages"),
                                 PAGES_INITIALIZE="false")
        environment.start()
        self.addCleanup(environment.stop)

    def fixture(self, dataset="gsi-peaks", channel="dev"):
        directory = self.root / dataset
        directory.mkdir(exist_ok=True)
        body = document([dict(id="fixture:1", name="確認用の山", type="peak", latitude=35, longitude=139)],
                        sources={"fixture": {"license": "CC0-1.0", "attribution": "テスト"}})
        raw = json.dumps(body, ensure_ascii=False).encode()
        payload = gzip.compress(raw, mtime=0)
        filename = dataset + ".json.gz"
        tag = target.release_tag(dataset, "test", channel)
        manifest = dict(schemaFormat="yamamuki-manifest", schemaVersion=6,
                        dataSchemaFormat="yamamuki-points", dataSchemaVersion=6,
                        name="確認用地点", version="test", fileName=filename,
                        downloadUrl=f"https://github.com/owner/repo/releases/download/{tag}/{filename}",
                        dataTimestamp="2026-10-18T00:00:00Z", metadata=body["metadata"],
                        sha256=hashlib.sha256(payload).hexdigest(), sizeBytes=len(payload),
                        uncompressedSizeBytes=len(raw))
        (directory / filename).write_bytes(payload)
        (directory / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        return directory, tag, manifest

    def test_both_datasets_and_future_dataset_share_validation(self):
        for dataset in ("osm-peaks", "gsi-peaks", "landmarks"):
            directory, tag, manifest = self.fixture(dataset)
            self.assertEqual(manifest, target.validate(directory, dataset, tag, "dev")[0])

    def test_real_gsi_artifacts_and_separate_parts_are_accepted(self):
        from importlib import import_module
        gsi_peaks = import_module("points.gsi-peaks.release_snapshot")
        tag, manifest = gsi_peaks.prepare("test", self.root, "dev")
        self.assertEqual(manifest, target.validate(self.root, "gsi-peaks", tag, "dev")[0])
        # 補足の破損も公開前に検出する。
        path = self.root / "supplements.json.gz"
        path.write_bytes(gzip.compress(gzip.decompress(path.read_bytes()) + b" "))
        with self.assertRaises(ValueError):
            target.validate(self.root, "gsi-peaks", tag, "dev")

    def test_corrupt_manifest_payload_metadata_and_coordinates_stop(self):
        directory, tag, original = self.fixture()
        for changes in ({"downloadUrl": "https://example.com/file.gz"}, {"version": "other"},
                        {"fileName": "../file.json.gz"}, {"sizeBytes": 1}, {"sha256": "a" * 64},
                        {"uncompressedSizeBytes": 1}, {"dataSchemaFormat": "terrain"},
                        {"schemaVersion": True}, {"dataTimestamp": "invalid"},
                        {"metadata": {**original["metadata"], "pointCount": 2}}):
            with self.subTest(changes=changes):
                (directory / "manifest.json").write_text(json.dumps({**original, **changes}), encoding="utf-8")
                with self.assertRaises((ValueError, OSError)):
                    target.validate(directory, "gsi-peaks", tag, "dev")
        original_body = json.loads(gzip.decompress((directory / original["fileName"]).read_bytes()))
        for changes in ({"latitude": 91}, {"id": ""}, {"elevationM": float("inf")},
                        {"nameReading": {"invalid": True}}):
            body = deepcopy(original_body)
            body["points"][0].update(changes)
            raw = json.dumps(body).encode()
            payload = gzip.compress(raw, mtime=0)
            (directory / original["fileName"]).write_bytes(payload)
            manifest = {**original, "sha256": hashlib.sha256(payload).hexdigest(),
                        "sizeBytes": len(payload), "uncompressedSizeBytes": len(raw)}
            (directory / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                target.validate(directory, "gsi-peaks", tag, "dev")

    def test_fetch_rejects_incomplete_parts(self):
        directory, tag, manifest = self.fixture()
        def gh(*args):
            if args[:2] == ("release", "download"):
                destination = Path(args[-1])
                destination.mkdir(exist_ok=True)
                shutil.copy(directory / "manifest.json", destination / "manifest.json")
                return ""
            return json.dumps({"assets": [{"name": "supplements-manifest.json"}]})
        with patch.object(target.site, "gh", side_effect=gh), self.assertRaisesRegex(ValueError, "そろって"):
            target.fetch("gsi-peaks", tag, self.root / "download", "dev")

    def test_publish_preserves_other_datasets_channels_and_histories(self):
        directory, tag, manifest = self.fixture()
        old_manifest = {**manifest, "version": "old", "downloadUrl": manifest["downloadUrl"].replace("-test/", "-old/")}
        catalog = target.site.Catalog({
            "points/osm-peaks-dev/manifest.json": {"version": "osm-existing"},
            "points/gsi-peaks/manifest.json": {**old_manifest, "version": "stable"},
            "points/gsi-peaks-dev/manifest.json": old_manifest},
            {"points/osm-peaks-dev/history.json": [{"kind": "snapshot", "version": "osm-existing"}]})
        expected = deepcopy(catalog)
        release = dict(tag_name=tag, draft=True, prerelease=True, body=f"[Codex] 検査済み\n- SHA-256: `{manifest['sha256']}`\n")
        def fetch(dataset, selected, path, channel):
            return (manifest if selected == tag else old_manifest), [dict(id="fixture:1")]
        with patch.object(target.site, "releases", return_value=[release]), \
                patch.object(target, "fetch", side_effect=fetch), \
                patch.object(target, "previous_release", return_value=(old_manifest, [{}])), \
                patch.object(target.site, "read_catalog", return_value=catalog), \
                patch.object(target.site, "gh") as gh:
            target.publish("gsi-peaks", tag, "dev")
        contents = json.loads((self.root / "pages/catalog.json").read_bytes())
        expected["points/gsi-peaks-dev/manifest.json"] = manifest
        self.assertEqual(expected, contents["manifests"])
        history = contents["histories"]["points/gsi-peaks-dev/history.json"]
        self.assertIn("/gsi-peaks-dev-old", history[0]["releaseUrl"])
        self.assertIn("/gsi-peaks-dev-test", history[-1]["releaseUrl"])
        self.assertEqual(catalog.histories["points/osm-peaks-dev/history.json"], contents["histories"]["points/osm-peaks-dev/history.json"])
        self.assertEqual(2, len(gh.call_args_list))
        self.assertIn("--draft=false", gh.call_args.args)

    def test_checksum_mismatch_or_previous_failure_never_publishes_draft(self):
        directory, tag, manifest = self.fixture()
        release = dict(tag_name=tag, draft=True, prerelease=True, body="")
        for checksum, previous_error in (("a" * 64, None), (manifest["sha256"], RuntimeError("HTTP 403")),
                                         (manifest["sha256"], target.site.MissingPreviousRelease("削除済み"))):
            with patch.object(target.site, "releases", return_value=[release]), \
                    patch.object(target, "fetch", return_value=(manifest, [{}])), \
                    patch.object(target, "previous_release", side_effect=previous_error), \
                    patch.object(target.site, "gh") as gh, \
                    patch.object(target.site, "update_latest") as update:
                with self.assertRaises((ValueError, RuntimeError)):
                    target.publish("gsi-peaks", tag, "dev", checksum)
                gh.assert_not_called()
                update.assert_not_called()

    def test_restore_published_release_never_rewrites_assets_or_notes(self):
        directory, tag, manifest = self.fixture()
        release = dict(tag_name=tag, draft=False, prerelease=True, body=f"- SHA-256: `{manifest['sha256']}`")
        with patch.object(target.site, "releases", return_value=[release]), \
                patch.object(target, "fetch", return_value=(manifest, [{}])), \
                patch.object(target, "previous_release", side_effect=RuntimeError("参照先欠落")), \
                patch.object(target.site, "gh") as gh, \
                patch.object(target.site, "update_latest") as update:
            target.publish("gsi-peaks", tag, "dev")
            gh.assert_not_called()
            update.assert_called_once()

    def test_new_dataset_can_fetch_validate_and_publish_without_configuration(self):
        directory, tag, manifest = self.fixture("landmarks")
        release = dict(tag_name=tag, draft=True, prerelease=True,
                       body=f"## 要確認\n初回の件数を確認してください\n- SHA-256: `{manifest['sha256']}`")
        published = []
        def gh(*args):
            if args[:2] == ("release", "download"):
                destination = Path(args[-1])
                destination.mkdir(exist_ok=True)
                for index, argument in enumerate(args):
                    if argument == "--pattern":
                        filename = args[index + 1]
                        shutil.copy(directory / filename, destination / filename)
            elif args[:2] == ("release", "view"):
                return json.dumps({"assets": [{"name": manifest["fileName"]}]})
            elif args[:2] == ("release", "edit"):
                published.append(args)
                if "--notes-file" in args:
                    contents = Path(args[-1]).read_text(encoding="utf-8-sig")
                    self.assertEqual(manifest["sha256"], target.site.reviewed_checksum({"body": contents}))
                    self.assertFalse(target.site.needs_review({"body": contents}))
            return ""
        with patch.object(target.site, "releases", return_value=[release]), \
                patch.object(target.site, "read_catalog", return_value=None), \
                patch.object(target.site, "gh", side_effect=gh), \
                patch.dict(os.environ, PAGES_INITIALIZE="true"):
            target.publish("landmarks", tag, "dev")
        self.assertEqual(2, len(published))
        self.assertEqual(manifest, json.loads((self.root / "pages/points/landmarks-dev/manifest.json").read_bytes()))

    def test_previous_release_is_scoped_to_dataset_and_channel(self):
        directory, tag, manifest = self.fixture(channel="stable")
        entries = [dict(tag_name=tag, draft=False, prerelease=False, body="検査済み"),
                   dict(tag_name="osm-peaks-zzz", draft=False, prerelease=False, body="検査済み"),
                   dict(tag_name="gsi-peaks-zzz", draft=True, prerelease=False, body="検査済み"),
                   dict(tag_name="gsi-peaks-dev-zzz", draft=False, prerelease=True, body="検査済み")]
        with patch.object(target, "fetch", return_value=(manifest, [{}])) as fetch:
            target.previous_release("gsi-peaks", "stable", self.root, entries)
            self.assertEqual(tag, fetch.call_args.args[1])
        dev_manifest = {**manifest, "version": "old"}
        with patch.object(target.site, "read_catalog", return_value={"points/gsi-peaks-dev/manifest.json": dev_manifest}), \
                self.assertRaises(target.site.MissingPreviousRelease):
            target.previous_release("gsi-peaks", "dev", self.root, entries)

    def test_pages_verification_entry_uses_shared_verifier(self):
        with patch.object(target.sys, "argv", ["point_release", "--verify-pages", str(self.root)]), \
                patch.object(target.site, "verify_pages") as verify:
            target.main()
            verify.assert_called_once_with(self.root)

    def test_similar_dataset_prefix_does_not_select_wrong_previous_release(self):
        entries = [dict(tag_name=tag, draft=False, prerelease=False, body="検査済み")
                   for tag in ("custom-peaks-z1", "custom-v1")]
        selected = []
        def fetch(dataset, tag, directory, channel):
            selected.append(tag)
            if tag == "custom-peaks-z1":
                raise target.DifferentDataset("manifest の version は z1")
            return ({"version": "v1"}, [{}])
        with patch.object(target, "fetch", side_effect=fetch):
            self.assertEqual("v1", target.previous_release("custom", "stable", self.root, entries)[0]["version"])
        self.assertEqual(["custom-v1"], selected)
        # 似た ID の版が並び順で先に来ても、manifest で区別する。
        entries[0]["tag_name"] = "custom-zpeaks-v1"
        selected.clear()
        def fetch_similar(dataset, tag, directory, channel):
            selected.append(tag)
            if tag == "custom-zpeaks-v1":
                raise target.DifferentDataset("別のデータ")
            return ({"version": "v1"}, [{}])
        with patch.object(target, "fetch", side_effect=fetch_similar):
            target.previous_release("custom", "stable", self.root, entries)
        self.assertEqual(["custom-zpeaks-v1", "custom-v1"], selected)

    def test_fetch_uses_manifest_to_reject_similar_dataset_id(self):
        directory, tag, manifest = self.fixture("custom-peaks", "stable")
        def gh(*args):
            destination = Path(args[-1])
            destination.mkdir(exist_ok=True)
            shutil.copy(directory / "manifest.json", destination / "manifest.json")
            return ""
        with patch.object(target.site, "gh", side_effect=gh) as run, self.assertRaises(target.DifferentDataset):
            target.fetch("custom", tag, self.root / "fetched", "stable")
        self.assertEqual(1, run.call_count)

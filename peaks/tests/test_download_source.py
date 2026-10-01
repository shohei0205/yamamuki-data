"""中断・再開・取得先の更新があっても異なる PBF を混ぜないことを確かめる。"""

import hashlib
import io
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

from scripts.download_source import download, resolve_source, transfer


class Response(io.BytesIO):
    def __init__(self, content=b"", *, status=200, headers=None, url=""):
        super().__init__(content)
        self.status = status
        self.headers = headers or {"Content-Length": str(len(content))}
        self.url = url


class DownloadSourceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.output = self.root / "japan-latest.osm.pbf"
        self.data = b"PBF fixture"
        self.source = {"url": "https://download.geofabrik.de/asia/japan-260929.osm.pbf",
                       "sizeBytes": len(self.data), "md5": hashlib.md5(self.data).hexdigest()}
        self.partial = self.output.with_name(f"{self.output.name}.{self.source['md5']}.part")

    def test_resolve_dated_url_and_checksum(self):
        responses = [Response(headers={"Content-Length": "123"}, url=self.source["url"] + "/"),
                     Response((self.source["md5"] + "  japan-260929.osm.pbf\n").encode())]
        with patch("scripts.download_source.request", side_effect=responses) as request:
            self.assertEqual({**self.source, "sizeBytes": 123}, resolve_source())
            self.assertEqual(self.source["url"] + ".md5", request.call_args.args[0])

    def test_reject_latest_url_and_missing_checksum(self):
        with patch("scripts.download_source.request", return_value=Response(url="https://download.geofabrik.de/asia/japan-latest.osm.pbf")):
            with self.assertRaises(ValueError):
                resolve_source()
        with patch("scripts.download_source.request", side_effect=[
                Response(headers={"Content-Length": "123"}, url=self.source["url"]), Response()]):
            with self.assertRaises(ValueError):
                resolve_source()

    def test_resume_from_saved_bytes(self):
        self.partial.write_bytes(self.data[:3])
        response = Response(self.data[3:], status=206, headers={
            "Content-Range": f"bytes 3-{len(self.data) - 1}/{len(self.data)}"})
        with patch("scripts.download_source.request", return_value=response) as request:
            transfer(self.source, self.partial)
            self.assertEqual({"Range": "bytes=3-"}, request.call_args.kwargs["headers"])
        self.assertEqual(self.data, self.partial.read_bytes())

    def test_ignored_range_restarts_instead_of_appending(self):
        self.partial.write_bytes(self.data[:3])
        with patch("scripts.download_source.request", return_value=Response(self.data)):
            transfer(self.source, self.partial)
        self.assertEqual(self.data, self.partial.read_bytes())

    def test_wrong_range_keeps_existing_partial(self):
        self.partial.write_bytes(self.data[:3])
        response = Response(self.data, status=206, headers={"Content-Range": "bytes 0-10/11"})
        with patch("scripts.download_source.request", return_value=response):
            with self.assertRaises(ValueError):
                transfer(self.source, self.partial)
        self.assertEqual(self.data[:3], self.partial.read_bytes())

    def test_interrupted_download_retries_and_resumes(self):
        responses = [Response(self.data[:3], headers={"Content-Length": str(len(self.data))}),
                     Response(self.data[3:], status=206, headers={
                         "Content-Range": f"bytes 3-{len(self.data) - 1}/{len(self.data)}"})]
        with patch("scripts.download_source.resolve_source", return_value=self.source) as resolve, \
                patch("scripts.download_source.request", side_effect=responses) as request:
            download(self.output, retry_delay=0)
            resolve.assert_called_once()
            self.assertEqual({"Range": "bytes=3-"}, request.call_args.kwargs["headers"])
        self.assertEqual(self.data, self.output.read_bytes())
        self.assertFalse(self.partial.exists())

    def test_corrupt_download_never_replaces_previous_output(self):
        self.output.write_bytes(b"previous")
        with patch("scripts.download_source.resolve_source", return_value=self.source), \
                patch("scripts.download_source.request", return_value=Response(b"x" * len(self.data))):
            with self.assertRaises(OSError):
                download(self.output, attempts=1)
        self.assertEqual(b"previous", self.output.read_bytes())
        self.assertFalse(self.partial.exists())

    def test_completed_download_is_reused(self):
        self.output.write_bytes(self.data)
        with patch("scripts.download_source.resolve_source", return_value=self.source), \
                patch("scripts.download_source.request") as request:
            download(self.output)
            request.assert_not_called()

    def test_other_version_partial_is_not_used(self):
        old = self.root / (self.output.name + ".old.part")
        old.write_bytes(b"old data")
        with patch("scripts.download_source.resolve_source", return_value=self.source), \
                patch("scripts.download_source.request", return_value=Response(self.data)) as request:
            download(self.output)
            self.assertEqual({}, request.call_args.kwargs["headers"])
        self.assertEqual(self.data, self.output.read_bytes())
        self.assertEqual(b"old data", old.read_bytes())

    def test_deadline_keeps_output_unchanged(self):
        self.output.write_bytes(b"previous")
        with patch("scripts.download_source.request", return_value=Response(self.data)):
            with self.assertRaises(TimeoutError):
                transfer(self.source, self.partial, deadline=time.monotonic() - 1)
        self.assertEqual(b"previous", self.output.read_bytes())


if __name__ == "__main__":
    unittest.main()

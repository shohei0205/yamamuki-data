"""中断・再開・取得先の更新があっても異なる PBF を混ぜないことを確かめる。"""

import hashlib
import io
from pathlib import Path
import tempfile
import time
import unittest
import urllib.error
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from threading import Thread
from unittest.mock import patch

from scripts.download_source import download, resolve_source, transfer, request, source_url_for_date, SOURCE_URL


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

    def test_date_selects_fixed_url_and_empty_keeps_latest(self):
        self.assertEqual(SOURCE_URL, source_url_for_date(""))
        self.assertEqual(self.source["url"], source_url_for_date("2026-09-29"))
        self.assertTrue(source_url_for_date("2024-02-29").endswith("japan-240229.osm.pbf"))

    def test_invalid_date_stops_before_network(self):
        for value in ("2026-02-29", "2026-9-29", "260929", "2100-01-01", "2026-09-29;echo bad"):
            with self.subTest(value=value), patch("scripts.download_source.resolve_source") as resolve:
                with self.assertRaises(ValueError):
                    download(self.output, source_date=value)
                resolve.assert_not_called()

    def test_dated_source_uses_matching_checksum_url(self):
        responses = [Response(headers={"Content-Length": "123"}, url=self.source["url"]),
                     Response((self.source["md5"] + "  japan-260929.osm.pbf").encode())]
        with patch("scripts.download_source.request", side_effect=responses) as req:
            self.assertEqual({**self.source, "sizeBytes": 123}, resolve_source(self.source["url"]))
            self.assertEqual([self.source["url"], self.source["url"] + ".md5"],
                             [call.args[0] for call in req.call_args_list])

    def test_dated_redirect_to_different_day_is_rejected(self):
        with patch("scripts.download_source.request", return_value=Response(
                url="https://download.geofabrik.de/asia/japan-260930.osm.pbf")):
            with self.assertRaises(ValueError):
                resolve_source(self.source["url"])

    def test_missing_dated_source_never_falls_back_to_latest(self):
        error = urllib.error.HTTPError(self.source["url"], 404, "Not Found", {}, None)
        with patch("scripts.download_source.resolve_source", side_effect=error) as resolve, \
                self.assertLogs(level="INFO") as logs:
            with self.assertRaises(urllib.error.HTTPError):
                download(self.output, source_date="2026-09-29", attempts=2, retry_delay=0)
        self.assertEqual(2, resolve.call_count)
        self.assertTrue(all(call.args == (self.source["url"],) for call in resolve.call_args_list))
        self.assertIn(self.source["url"], "\n".join(logs.output))

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


class RequestLoggingTests(unittest.TestCase):
    def test_redirected_head_404_records_both_urls_without_response_body(self):
        class Handler(BaseHTTPRequestHandler):
            def do_HEAD(self):
                self.send_response(302 if self.path == "/latest" else 404)
                if self.path == "/latest":
                    self.send_header("Location", "/missing")
                self.send_header("Retry-After", "30")
                self.send_header("Set-Cookie", "private-cookie")
                self.end_headers()

            # urllib は転送時に HEAD を GET に変える場合がある。
            do_GET = do_HEAD

            def log_message(self, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        worker = Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            url = f"http://127.0.0.1:{server.server_port}/latest"
            with self.assertLogs(level="INFO") as logs:
                with self.assertRaises(urllib.error.HTTPError) as caught:
                    with request(url, method="HEAD"):
                        self.fail("404 は例外になる")
            text = "\n".join(logs.output)
            self.assertEqual(404, caught.exception.code)
            for part in ("HEAD", url, "/missing", "HTTP 302", "HTTP 404", "Retry-After", "30"):
                self.assertIn(part, text)
            self.assertNotIn("private-cookie", text)
        finally:
            server.shutdown()
            worker.join()
            server.server_close()

    def test_timeout_before_response_keeps_exception_and_url(self):
        error = TimeoutError("接続待ちが終了")
        with patch("scripts.download_source.urllib.request.build_opener") as opener, self.assertLogs(level="INFO") as logs:
            opener.return_value.open.side_effect = error
            with self.assertRaises(TimeoutError) as caught:
                with request("https://example.test/file", headers={"Range": "bytes=3-"}):
                    pass
        self.assertIs(error, caught.exception)
        text = "\n".join(logs.output)
        for part in ("GET", "https://example.test/file", "bytes=3-", "TimeoutError", "HTTP=未取得"):
            self.assertIn(part, text)

    def test_read_timeout_records_received_response(self):
        response = Response(status=206, url="https://example.test/dated")
        with patch("scripts.download_source.urllib.request.build_opener") as opener, self.assertLogs(level="INFO") as logs:
            opener.return_value.open.return_value = response
            with self.assertRaises(TimeoutError):
                with request("https://example.test/latest"):
                    raise TimeoutError("本文の受信が停止")
        self.assertTrue(response.closed)
        text = "\n".join(logs.output)
        self.assertIn("応答URL=https://example.test/dated", text)
        self.assertIn("HTTP=206", text)
        self.assertIn("本文の受信が停止", text)

    def test_http_error_does_not_read_or_log_body(self):
        body = io.BytesIO(b"private-response-body")
        error = urllib.error.HTTPError("https://example.test/missing", 404, "Not Found", {}, body)
        with patch("scripts.download_source.urllib.request.build_opener") as opener, self.assertLogs(level="INFO") as logs:
            opener.return_value.open.side_effect = error
            with self.assertRaises(urllib.error.HTTPError) as caught:
                with request("https://example.test/latest"):
                    pass
        self.assertIs(error, caught.exception)
        self.assertTrue(body.closed)
        self.assertNotIn("private-response-body", "\n".join(logs.output))

    def test_exhausted_retries_record_final_attempt_and_stage(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch("scripts.download_source.resolve_source", side_effect=OSError("取得先不明")), \
                self.assertLogs(level="INFO") as logs:
            with self.assertRaises(OSError):
                download(Path(directory) / "source.pbf", attempts=2, retry_delay=0)
        text = "\n".join(logs.output)
        for part in ("段階=取得先の日付・サイズ・MD5 の確認", "試行=1/2", "試行=2/2",
                     "japan-latest.osm.pbf", "OSError", "試行回数の上限"):
            self.assertIn(part, text)

    def test_invalid_source_records_nonretryable_failure(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch("scripts.download_source.resolve_source", side_effect=ValueError("不正な取得先")) as resolve, \
                self.assertLogs(level="ERROR") as logs:
            with self.assertRaises(ValueError):
                download(Path(directory) / "source.pbf")
        resolve.assert_called_once()
        self.assertIn("再試行対象外", "\n".join(logs.output))


if __name__ == "__main__":
    unittest.main()

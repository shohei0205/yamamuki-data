"""全国取得の再開と失敗時の扱いを通信せずに確かめる。"""
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import urllib.error
from PIL import Image
import fetch_national as fetch


class FetchTests(unittest.TestCase):
    def test_priority_visits_every_tile_once(self):
        area=fetch.bounds([122,20,154,46],8)
        batches=list(fetch.tile_batches(area,8))
        points=[point for _,batch in batches for point in batch]
        x0,y0,x1,y1=area
        self.assertEqual(len(points),(x1-x0+1)*(y1-y0+1))
        self.assertEqual(len(set(points)),len(points))
        self.assertEqual(batches[0][0],"中部")
        self.assertNotEqual(points[0],(x0,y0))
        self.assertEqual(batches[-1][0],"残りの範囲")

    def test_original_png_and_resume(self):
        output=io.BytesIO()
        Image.new("RGB", (256,256), (0,1,0)).save(output,format="PNG")
        body=output.getvalue()
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            args=fetch.parser().parse_args(["--bbox","138.34","35.95","138.35","35.96","--cache",str(root/"cache"),"--output",str(root/"out")])
            response=unittest.mock.MagicMock()
            response.__enter__.return_value.read.return_value=body
            with patch("urllib.request.urlopen",return_value=response),patch.object(fetch.RateLimit,"wait"):
                fetch.run(args)
            self.assertTrue(all(p.read_bytes()==body for p in args.cache.rglob("*.png")))
            with patch("urllib.request.urlopen") as request:
                fetch.run(args)
                request.assert_not_called()
            record=json.loads((args.output/"progress.json").read_text("utf-8"))
            self.assertEqual(record["phase"],"complete")
            self.assertEqual(record["completedTiles"],record["savedTiles"])

    def test_low_disk_stops_before_network(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            args=fetch.parser().parse_args(["--bbox","138.34","35.95","138.35","35.96","--cache",str(root/"cache"),"--output",str(root/"out")])
            with patch("shutil.disk_usage",return_value=type("Disk",(),{"free":9_000_000_000})()),patch("urllib.request.urlopen") as request:
                fetch.run(args)
                request.assert_not_called()
            record=json.loads((args.output/"progress.json").read_text("utf-8"))
            self.assertEqual(record["phase"],"disk_space_low")
            self.assertEqual(record["completedTiles"],0)

    def test_404_and_403(self):
        with tempfile.TemporaryDirectory() as folder:
            with patch.object(fetch.RateLimit,"wait"),patch("urllib.request.urlopen",side_effect=urllib.error.HTTPError("u",404,"missing",{},None)):
                result=fetch.fetch_tile(1,2,14,Path(folder),fetch.RateLimit(8))
                self.assertEqual(result[2],404)
                self.assertFalse(list(Path(folder).rglob("*.png")))
            with patch.object(fetch.RateLimit,"wait"),patch("urllib.request.urlopen",side_effect=urllib.error.HTTPError("u",403,"denied",{},None)):
                with self.assertRaises(urllib.error.HTTPError):
                    fetch.fetch_tile(1,2,14,Path(folder),fetch.RateLimit(8))

    def test_transient_error_retries(self):
        with tempfile.TemporaryDirectory() as folder:
            errors=[urllib.error.HTTPError("u",429,"busy",{},None),urllib.error.HTTPError("u",404,"missing",{},None)]
            with patch.object(fetch.RateLimit,"wait"),patch("time.sleep"),patch("urllib.request.urlopen",side_effect=errors) as request:
                self.assertEqual(fetch.fetch_tile(1,2,14,Path(folder),fetch.RateLimit(8))[2],404)
                self.assertEqual(request.call_count,2)

"""追加範囲、PBF の統合、取得失敗と公開時の検査を確認する。"""
import gzip
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from scripts.build_data import build, read_mountains, FILE_NAME
from scripts.download_source import SOURCES, cache_values, resolve_source, download, save_source_info, source_url_for_date
from scripts.source_regions import REGIONS, region_for
from scripts.check_release import assess, report, validate
from test_download_source import Response

STAMP = "2026-10-03T20:21:04Z"


def node(identifier, lat, lon, name="山", version=1, stamp="2026-10-01T00:00:00Z", tags=""):
    return (f'<node id="{identifier}" version="{version}" timestamp="{stamp}" lat="{lat}" lon="{lon}">'
            f'<tag k="natural" v="peak"/><tag k="name" v="{name}"/>{tags}</node>')


class SourceRegionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.supplements = self.root / "supplements.json"
        self.supplements.write_text('{"schemaVersion":1,"points":[]}', encoding="utf-8")
        self.pbfs, self.xmls = {}, {}
        fixtures = {
            "japan": node(1,35.36,138.72,"富士山"),
            "far-eastern-fed-district": node(2,44.35,146.25,"Тятя",tags='<tag k="name:ja" v="爺爺岳"/>')
                + node(3,45.03,147.87,"Берутарубе",tags='<tag k="name:ja" v="Берутарубе"/>') + node(4,46,143,"対象外"),
            "south-korea": node(5,37.24,131.87,"봉우리",tags='<tag k="name:ja" v="日本語の山名"/>') + node(6,37.5,127,"対象外"),
        }
        for source, xml in fixtures.items():
            self.xmls[source] = self.root / (source + ".osm")
            self.xmls[source].write_text('<osm version="0.6">'+xml+'</osm>',encoding="utf-8")
            self.pbfs[source] = self.root / (source + ".osm.pbf")
            self.pbfs[source].write_bytes(source.encode())
            self.save_source(source)

    def save_source(self, source, day="2026-10-03"):
        pbf = self.pbfs[source]
        save_source_info(pbf,{"url":source_url_for_date(day,source),"sizeBytes":pbf.stat().st_size,
                              "md5":hashlib.md5(pbf.read_bytes()).hexdigest()})

    def extract(self, command, **kwargs):
        source = next(key for key,path in self.pbfs.items() if str(path)==command[2])
        shutil.copyfile(self.xmls[source],command[-1])

    def additional(self):
        return {source:path for source,path in self.pbfs.items() if source!="japan"}

    def generate(self):
        with patch("scripts.build_data.subprocess.check_output",return_value=STAMP), \
                patch("scripts.build_data.subprocess.run",side_effect=self.extract):
            return build(self.pbfs["japan"],self.root/"out","test",additional_pbfs=self.additional(), supplements_path=self.supplements)

    def rows(self):
        return json.loads(gzip.decompress((self.root/"out"/FILE_NAME).read_bytes()))

    def test_locations_and_outside_land(self):
        for lat,lon,expected in [(44.35,146.25,"kunashiri"),(45.03,147.87,"etorofu"),
                                 (43.8,146.75,"shikotan"),(43.5,146.2,"habomai"),(37.24,131.87,"takeshima")]:
            self.assertEqual(expected,region_for(lat,lon)["id"])
        for lat,lon in [(43.38,145.82),(44.08,145.12),(46,143),(45.65,149.5),(37.5,127)]:
            self.assertIsNone(region_for(lat,lon))
        for region in REGIONS:
            west,south,east,north=region["bbox"]
            self.assertEqual(region,region_for((south+north)/2,(west+east)/2))
            self.assertIsNotNone(region_for(south,west))
            self.assertIsNotNone(region_for(north,east))

    def test_combines_names_and_records_all_sources(self):
        manifest=self.generate()
        self.assertEqual([1,2,3,5],[row["osmId"] for row in self.rows()])
        self.assertEqual("爺爺岳",self.rows()[1]["name"])
        self.assertEqual("Берутарубе",self.rows()[2]["name"])
        self.assertEqual(3,len(manifest["sourcePbfs"]))
        self.assertEqual(5,manifest["schemaVersion"])
        self.assertEqual(manifest,validate(self.root/"out")[0])
        text=report((manifest,self.rows()),None,[])
        for name in ("国後島: 1 件","択捉島: 1 件","色丹島: 0 件","竹島: 1 件"):
            self.assertIn(name,text)

    def test_additional_sources_require_nonblank_japanese_name(self):
        for source, lat, lon in [("far-eastern-fed-district", 44.35, 146.25), ("south-korea", 37.24, 131.87)]:
            xml = (node(10, lat, lon, "現地名だけ")
                   + node(11, lat, lon, "空欄", tags='<tag k="name:ja" v="  "/>')
                   + node(12, lat, lon, "現地名", tags='<tag k="name:ja" v=" 日本語名 "/>'))
            path = self.xmls[source]
            path.write_text('<osm>' + xml + '</osm>', encoding="utf-8")
            rows, _ = read_mountains(path, source=source, allow_empty=True)
            self.assertEqual([12], [row["osmId"] for row in rows])
            self.assertEqual("日本語名", rows[0]["name"])
            # 日本 PBF では従来どおり name も使う。
            self.assertEqual(3, len(read_mountains(path)[0]))

    def test_supplements_apply_after_sources_are_combined(self):
        from scripts.supplements import validate_companions
        supplement = {"schemaVersion": 1, "sources": {"資料": {"source": "試験用資料"}},
                      "points": [{"osmId": 2, "aliases": ["追加した別名"], "expected": {"name": "爺爺岳"}}]}
        self.supplements.write_text(json.dumps(supplement, ensure_ascii=False), encoding="utf-8")
        manifest = self.generate()
        rows = self.rows()
        self.assertEqual(["追加した別名"], next(row for row in rows if row["osmId"] == 2)["aliases"])
        self.assertEqual(3, len(manifest["sourcePbfs"]))
        self.assertEqual(supplement["sources"], manifest["supplementSources"])
        validate_companions(self.root / "out", manifest, rows)

    def test_zero_peaks_in_south_korea_is_valid(self):
        self.xmls["south-korea"].write_text('<osm/>',encoding="utf-8")
        self.assertEqual(3,self.generate()["pointCount"])

    def test_empty_northern_peaks_stop_before_output(self):
        self.xmls["far-eastern-fed-district"].write_text('<osm/>',encoding="utf-8")
        with self.assertRaisesRegex(ValueError,"国後島"):
            self.generate()
        self.assertFalse((self.root/"out").exists())

    def test_corrupt_or_missing_additional_pbf_stops(self):
        for source in self.additional():
            original=self.pbfs[source].read_bytes()
            self.pbfs[source].write_bytes(b"broken")
            with self.subTest(source=source),self.assertRaises(ValueError):
                self.generate()
            self.pbfs[source].write_bytes(original)
        self.pbfs["south-korea"].unlink()
        with self.assertRaises(OSError):
            self.generate()
        self.assertFalse((self.root/"out").exists())

    def test_different_day_is_rejected(self):
        self.save_source("south-korea","2026-10-02")
        with self.assertRaisesRegex(ValueError,"配布日"):
            self.generate()

    def test_incomplete_source_set_is_rejected(self):
        with self.assertRaises(ValueError):
            build(self.pbfs["japan"],self.root/"out","test",additional_pbfs={})

    def test_extraction_failure_preserves_previous_output(self):
        self.generate()
        original=(self.root/"out"/FILE_NAME).read_bytes()
        with patch("scripts.build_data.subprocess.run",side_effect=subprocess.CalledProcessError(1,"osmium")), \
                patch("scripts.build_data.subprocess.check_output",return_value=STAMP):
            with self.assertRaises(subprocess.CalledProcessError):
                build(self.pbfs["japan"],self.root/"out","next",additional_pbfs=self.additional(), supplements_path=self.supplements)
        self.assertEqual(original,(self.root/"out"/FILE_NAME).read_bytes())

    def test_duplicate_uses_newer_node_version(self):
        self.xmls["japan"].write_text('<osm>'+node(2,44.35,146.25,"古い名前")+'</osm>',encoding="utf-8")
        p=self.xmls["far-eastern-fed-district"]
        p.write_text(p.read_text(encoding="utf-8").replace('version="1"','version="2"'),encoding="utf-8")
        self.generate()
        self.assertEqual([2,3,5],[row["osmId"] for row in self.rows()])
        self.assertEqual("爺爺岳",self.rows()[0]["name"])

    def test_identical_duplicate_is_included_once(self):
        self.xmls["japan"].write_text('<osm>'+node(3,45.03,147.87,"Берутарубе",tags='<tag k="name:ja" v="Берутарубе"/>')+'</osm>',encoding="utf-8")
        self.generate()
        self.assertEqual([2,3,5],[row["osmId"] for row in self.rows()])

    def test_conflicting_same_revision_is_rejected(self):
        self.xmls["japan"].write_text('<osm>'+node(2,44.35,146.25,"異なる名前")+'</osm>',encoding="utf-8")
        with self.assertRaisesRegex(ValueError,"同じ版"):
            self.generate()

    def test_each_pbf_timestamp_is_checked(self):
        with patch("scripts.build_data.subprocess.check_output",side_effect=[STAMP,"2026-09-30T00:00:00Z"]), \
                patch("scripts.build_data.subprocess.run",side_effect=self.extract):
            with self.assertRaisesRegex(ValueError,"基準日時"):
                build(self.pbfs["japan"],self.root/"out","test",additional_pbfs=self.additional(), supplements_path=self.supplements)

    def test_invalid_provenance_is_rejected_on_publication(self):
        original=self.generate()
        for mutation in (lambda m:m["sourcePbfs"].pop(),
                         lambda m:m["sourcePbfs"][1].update(md5="bad"),
                         lambda m:m["sourcePbfs"][1].update(url=SOURCES["south-korea"]+"-261003.osm.pbf"),
                         lambda m:m.update(sourceTimestamp="2026-10-04T00:00:00Z")):
            manifest=json.loads(json.dumps(original));mutation(manifest)
            (self.root/"out/manifest.json").write_text(json.dumps(manifest),encoding="utf-8")
            with self.assertRaises(ValueError):
                validate(self.root/"out")

    def test_additions_do_not_hide_existing_decline(self):
        manifest=self.generate()
        old=[{"latitude":35,"longitude":139}]*100
        new=old[:79]+[{"latitude":44.35,"longitude":146.25}]*100
        self.assertTrue(any("追加範囲を除く" in warning for warning in assess((manifest,new),(manifest,old))))

    def test_small_island_decline_is_reported(self):
        manifest=self.generate()
        old=[{"latitude":37.24,"longitude":131.87}]
        self.assertTrue(any("竹島の件数" in warning for warning in assess((manifest,[]),(manifest,old))))

    def test_regions_have_fixed_urls_and_separate_caches(self):
        keys=set()
        for region in SOURCES:
            source={"url":source_url_for_date("2026-10-03",region),"md5":"a"*32,"sizeBytes":12}
            responses=[Response(url=source["url"],headers={"Content-Length":"12"}),Response(b"a"*32)]
            with patch("scripts.download_source.request",side_effect=responses):
                self.assertEqual(source,resolve_source(SOURCES[region]+"-latest.osm.pbf"))
            keys.add(cache_values(source)["cache-key"])
        self.assertEqual(3,len(keys))

    def test_other_region_redirect_is_rejected(self):
        response = Response(url=source_url_for_date("2026-10-03", "japan"))
        with patch("scripts.download_source.request", return_value=response):
            with self.assertRaises(ValueError):
                resolve_source(SOURCES["south-korea"] + "-latest.osm.pbf")

    def test_cli_default_output_is_separate_for_each_region(self):
        from scripts.download_source import main
        for region in SOURCES:
            with patch("sys.argv", ["download_source.py", "--region", region]), \
                    patch("scripts.download_source.download", return_value={}) as fetch, \
                    patch.dict("scripts.download_source.os.environ", {}, clear=True):
                main()
            expected = "japan-latest.osm.pbf" if region == "japan" else region + ".osm.pbf"
            self.assertEqual(Path("build") / expected, fetch.call_args.args[0])
            self.assertEqual(region, fetch.call_args.kwargs["region"])

    def test_additional_download_failure_is_propagated(self):
        with patch("scripts.download_source.resolve_source",side_effect=OSError("取得失敗")):
            with self.assertRaises(OSError):
                download(self.root/"missing",region="south-korea",attempts=1)
        self.assertFalse((self.root/"missing").exists())

    @unittest.skipUnless(shutil.which("osmium"),"osmium がないため、3 つの PBF の生成テストは CI で実行")
    def test_three_real_pbfs_to_distribution(self):
        for source in SOURCES:
            self.pbfs[source].unlink()
            subprocess.run(["osmium","cat",str(self.xmls[source]),"-o",str(self.pbfs[source]),
                            f"--output-header=osmosis_replication_timestamp={STAMP}"],check=True)
            self.save_source(source)
        manifest=build(self.pbfs["japan"],self.root/"out","test",additional_pbfs=self.additional(), supplements_path=self.supplements)
        self.assertEqual(4,manifest["pointCount"])
        validate(self.root/"out")

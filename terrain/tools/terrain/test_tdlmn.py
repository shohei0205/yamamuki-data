"""標準地図から座標だけを抽出し、標高 PNG を検査する。"""
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from PIL import Image
from download_dem_with_tdlmn import ROOT, select_tiles, load_missing


class TdlmnTests(unittest.TestCase):
    def test_missing_records_are_scoped_to_catalog(self):
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'records.jsonl'
            records=[{'url':'https://maps.gsi.go.jp/xyz/dem_png/14/10/20.png','status':status,'catalogSha256':catalog}
                     for status,catalog in [(404,'old'),(429,'current'),(503,'current'),(404,'current')]]
            path.write_text(''.join(json.dumps(r)+'\n' for r in records)+'{"partial":',encoding='utf-8')
            self.assertEqual(load_missing(path,'current'),{'14/10/20.png'})
            self.assertEqual(load_missing(path,'new'),set())
            path.write_text(''.join(json.dumps(r)+'\n' for r in records+[dict(records[-1],status=200)]),encoding='utf-8')
            self.assertEqual(load_missing(path,'current'),set())

    def test_only_404_is_persisted(self):
        ruby=ROOT/'build/rubyinstaller-4.0.7-1-x64/bin/ruby.exe'
        with tempfile.TemporaryDirectory() as folder:
            code='''require ARGV.shift
ENV['DEM_CATALOG_SHA256']='snapshot'
response=Struct.new(:status)
error=Struct.new(:io)
[429,503,403].each{|status| abort 'wrongly cached' if record_dem_not_found(error.new(response.new([status.to_s])), 'https://maps.gsi.go.jp/xyz/dem_png/14/10/20.png', 'tile')}
abort '404 not cached' unless record_dem_not_found(error.new(response.new(['404'])), 'https://maps.gsi.go.jp/xyz/dem_png/14/10/20.png', 'tile')
'''
            subprocess.run([str(ruby),'-e',code,str(ROOT/'tools/terrain/tdlmn_dem.rb')],cwd=folder,check=True)
            records=[json.loads(line) for line in (Path(folder)/'dem-downloads.jsonl').read_text('utf-8').splitlines()]
            self.assertEqual(len(records),1)
            self.assertEqual(records[0]['status'],404)
            self.assertEqual(load_missing(Path(folder)/'dem-downloads.jsonl','snapshot'),{'14/10/20.png'})

    def test_only_coordinates_are_reused(self):
        lines=['14/10/20.png,123,456,standard-map-md5\n','13/10/20.png,1,1,x\n','14/11/20.png,1,1,x\n']
        self.assertEqual(list(select_tiles(lines,(10,20,10,20))),['14/10/20.png'])

    def test_valid_and_corrupt_png(self):
        ruby=ROOT/'build/rubyinstaller-4.0.7-1-x64/bin/ruby.exe'
        if not ruby.exists():
            self.fail('検証用 Ruby が見つかりません')
        with tempfile.TemporaryDirectory() as directory:
            output=io.BytesIO()
            Image.new('RGB',(256,256),(0,1,0)).save(output,format='PNG')
            body=output.getvalue()
            valid=Path(directory)/'valid.png';valid.write_bytes(body)
            bad=Path(directory)/'bad.png';bad.write_bytes(body[:40]+b'broken'+body[46:])
            code='require ARGV.shift; abort "valid rejected" unless valid_dem_png?(File.binread(ARGV[0])); abort "corrupt accepted" if valid_dem_png?(File.binread(ARGV[1])); abort "text accepted" if valid_dem_png?("not png")'
            subprocess.run([str(ruby),'-e',code,str(ROOT/'tools/terrain/tdlmn_dem.rb'),str(valid),str(bad)],check=True)

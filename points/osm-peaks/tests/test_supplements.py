"""補足の適用・由来の保持・配布ファイルの再現性を検査する。"""
from copy import deepcopy
import gzip
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts.supplements import apply_supplements, validate_supplements, write_companions, validate_companions, complete_supplement, read_supplements, COMPANION_FILES
from scripts.build_data import write_distribution
from scripts import release_data


class SupplementTests(unittest.TestCase):
    def setUp(self):
        self.points = [{"id":"1","osmId":1,"name":"元の山","nameReading":"誤り","aliases":["旧別名","残す別名"],"latitude":35,"longitude":139},
                       {"id":"2","osmId":2,"name":"重複地点","latitude":35,"longitude":139}]
        self.data = {"schemaVersion":1,"sources":{"資料":{"source":"https://example.com/reference","license":"CC BY 4.0"}},"points":[
            {"osmId":1,"name":"補正した山","nameReading":None,"tags":["分類A","分類B"],"aliases":["追加別名"],"aliasesRemove":["旧別名"],"reason":"資料による修正","note":"管理用メモ","expected":{"nameReading":"誤り"}},
            {"osmId":2,"exclude":True,"reason":"同じ山の重複地点"}]}
        self.temp = tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)

    def test_apply_keeps_original_and_removes_only_requested_values(self):
        original=deepcopy(self.points);data=deepcopy(self.data)
        result=apply_supplements(self.points,self.data)
        self.assertEqual(self.points,original);self.assertEqual(self.data,data)
        self.assertEqual(len(result),1);row=result[0]
        self.assertEqual(row['name'],'補正した山');self.assertNotIn('nameReading',row)
        self.assertEqual(row['tags'],['分類A','分類B']);self.assertEqual(row['aliases'],['残す別名','追加別名'])
        for key in ['note','reason','references','expected']:self.assertNotIn(key,row)

    def test_omitted_values_and_empty_arrays_keep_original(self):
        result=apply_supplements(self.points,{'schemaVersion':1,'points':[{'osmId':1,'tags':[],'aliases':[]}]})
        self.assertEqual(result[0]['nameReading'],'誤り');self.assertEqual(result[0]['aliases'],self.points[0]['aliases'])

    def test_remove_original_tag_without_changing_original(self):
        self.points[0]['tags']=['元タグ','残すタグ']
        result=apply_supplements(self.points,{'schemaVersion':1,'points':[{'osmId':1,'tagsRemove':['元タグ']}]})
        self.assertEqual(result[0]['tags'],['残すタグ'])
        self.assertEqual(self.points[0]['tags'],['元タグ','残すタグ'])

    def test_unknown_id_and_stale_expected_fail_atomically(self):
        for changed in [{'osmId':999},{'expected':{'name':'古い名前'}}]:
            data=deepcopy(self.data);data['points'][0].update(changed);original=deepcopy(self.points)
            with self.assertRaises(ValueError):apply_supplements(self.points,data)
            self.assertEqual(self.points,original)

    def test_invalid_values_and_unknown_fields(self):
        for changed in [{'name':''},{'nameReading':''},{'nameReading':False},{'exclude':1},{'tags':['同じ','同じ']},{'aliases':[None]},
                        {'aliasesRemove':['追加別名']},{'reason':''},{'references':{'name':['未登録']}},{'references':{'unknown':['資料']}},{'extra':1}]:
            with self.subTest(changed=changed):
                data=deepcopy(self.data);data['points'][0].update(changed)
                with self.assertRaises(ValueError):validate_supplements(data)
        data=deepcopy(self.data);data['points'].append(data['points'][0])
        with self.assertRaises(ValueError):validate_supplements(data)

    def distribution(self):
        integrated=apply_supplements(self.points,self.data)
        manifest=write_distribution(integrated,self.root,'test','2026-10-01T00:00:00Z','2026-10-01T00:00:00Z')
        companion=write_companions(self.root,self.points,self.data,manifest)
        return integrated,manifest,companion

    def test_companions_reproduce_legacy_gzip_and_keep_original(self):
        integrated,manifest,companion=self.distribution()
        validate_companions(self.root,manifest,integrated)
        self.assertEqual(manifest['schemaVersion'],5)
        from scripts.release_channels import download_url
        self.assertEqual(manifest['fileName'],'osm-peaks.json.gz')
        self.assertEqual(manifest['downloadUrl'],download_url('test','stable'))
        with gzip.open(self.root/manifest['fileName'],'rt',encoding='utf-8') as stream:
            self.assertEqual(json.load(stream),integrated)
        with gzip.open(self.root/COMPANION_FILES[0],'rt',encoding='utf-8') as f:self.assertEqual(json.load(f),self.points)
        self.assertEqual(companion['sources'],self.data['sources'])
        self.assertIn(manifest['version'],companion['supplements']['downloadUrl'])
        self.assertEqual(read_supplements(self.root/'supplements.json'),self.data)

    def test_companion_tampering_missing_and_version_mismatch(self):
        for kind in ['tamper','missing','version','integrated']:
            with self.subTest(kind=kind):
                integrated,manifest,companion=self.distribution()
                if kind=='tamper':(self.root/'supplements.json').write_text('{}',encoding='utf-8')
                if kind=='missing':(self.root/COMPANION_FILES[0]).unlink()
                if kind=='version':manifest['version']='another'
                if kind=='integrated':integrated[0]['name']='違う山'
                with self.assertRaises(ValueError):validate_companions(self.root,manifest,integrated)

    def test_old_distribution_without_companions(self):
        validate_companions(self.root,{},[])

    def test_complete_preserves_multiple_tags_and_corrections(self):
        data=deepcopy(self.data);data['points'][0].pop('osmId');data['points'][0]['note']='元の山'
        input_path=self.root/'input.json';points_path=self.root/'points.json';output_path=self.root/'output.json'
        input_path.write_text(json.dumps(data,ensure_ascii=False),encoding='utf-8');points_path.write_text(json.dumps(self.points,ensure_ascii=False),encoding='utf-8')
        before=input_path.read_bytes();report,unresolved=complete_supplement(input_path,points_path,output_path)
        self.assertFalse(unresolved);self.assertEqual(input_path.read_bytes(),before)
        actual=read_supplements(output_path);self.assertEqual(actual['points'][0]['osmId'],1);self.assertEqual(actual['points'][0]['tags'],['分類A','分類B']);self.assertEqual(actual['sources'],self.data['sources'])

    def test_fetch_companions_when_present_and_rejects_missing_asset(self):
        assets={'assets':[{'name':name} for name in COMPANION_FILES]}
        with patch.object(release_data,'gh',return_value=json.dumps(assets)) as gh,patch.object(release_data,'validate',return_value=({},[])):
            release_data.fetch('osm-peaks-test',self.root)
            self.assertEqual(gh.call_count,3)
            for name in COMPANION_FILES:self.assertIn(name,gh.call_args.args)
        assets['assets'].pop()
        with patch.object(release_data,'gh',return_value=json.dumps(assets)),self.assertRaises(ValueError):release_data.fetch('osm-peaks-test',self.root)

    def test_build_default_path_generates_complete_companion_set(self):
        from scripts.build_data import build
        path=self.root/'input-supplements.json'
        path.write_text(json.dumps(self.data,ensure_ascii=False),encoding='utf-8')
        with patch('scripts.build_data.verified_source_url',return_value='https://example.com/source'), patch('scripts.build_data.subprocess.check_output',return_value='2026-10-01T00:00:00Z'), patch('scripts.build_data.subprocess.run'), patch('scripts.build_data.read_mountains',return_value=(deepcopy(self.points),'2026-10-01T00:00:00Z')):
            manifest=build(self.root/'source.pbf',self.root/'dist','test',supplements_path=path)
        self.assertEqual(manifest['tagSources'],[{'name':'資料',**self.data['sources']['資料']}])
        with gzip.open(self.root/'dist/osm-peaks.json.gz','rt',encoding='utf-8') as f:
            integrated=json.load(f)
        validate_companions(self.root/'dist',manifest,integrated)

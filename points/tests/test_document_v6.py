"""版6の出典保持、旧配列の読み込み、項目順の違いを確認する。"""

import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'points'))
sys.path.insert(0,str(ROOT/'points/osm-peaks'))
from all_points_io import read_points, write_points
from scripts.point_document import rows


class DocumentV6Tests(unittest.TestCase):
    def test_metadata_survives_streaming_with_any_field_order(self):
        point={'id':'1','name':'山','latitude':35,'longitude':139}
        tags={'日本百名山':{'reading':'にほんひゃくめいざん'}}
        info={'sourceUrl':'https://example.com','license':'PDL-1.0','attribution':'国土地理院','supplementSources':{'資料':{'source':'資料名'}},'latestPointTimestamp':'2026-10-14T00:00:00Z'}
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'input.json'
            for doc in [{'schemaVersion':6,**info,'tags':tags,'points':[point]},{'points':[point],'tags':tags,**info,'schemaVersion':6}]:
                path.write_text(json.dumps(doc,ensure_ascii=False),encoding='utf-8')
                metadata={}
                self.assertEqual(list(read_points(path,chunk_size=1,metadata=metadata)),[point])
                self.assertEqual(metadata,{'schemaVersion':6,**info,'tags':tags})
                self.assertEqual(rows(doc,6),[point])
            write_points([point],folder,'output',tags=tags,metadata=info)
            doc=json.loads((Path(folder)/'output.json').read_bytes())
            self.assertEqual(doc,{'schemaVersion':6,'metadata':{**info,'tags':tags,'pointCount':1},'points':[point]})
            path.write_text(json.dumps([point]),encoding='utf-8')
            self.assertEqual(list(read_points(path,chunk_size=1)),[point])
            with self.assertRaises(ValueError): rows([point],6)
            with self.assertRaises(ValueError): rows(doc,5)

    def test_schema_version_is_required_and_matches_manifest_version(self):
        point={'id':'1','name':'山','latitude':35,'longitude':139}
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'input.json'
            for metadata in [{},{'schemaVersion':5},{'schemaVersion':7},{'schemaVersion':True},{'schemaVersion':'6'}]:
                doc={**metadata,'points':[point]}
                with self.subTest(metadata=metadata):
                    with self.assertRaises(ValueError): rows(doc,6)
                    path.write_text(json.dumps(doc),encoding='utf-8')
                    with self.assertRaises(ValueError): list(read_points(path))
            self.assertEqual(rows({'schemaVersion':6,'points':[point]},6),[point])
            self.assertEqual(rows([point],5),[point])

    def test_invalid_document_is_rejected(self):
        for doc in [{'points':[],'pointCount':1},{'points':[],'pointCount':True},{'points':[],'latestPointTimestamp':None},{'points':{},'tags':{}},{'points':[],'tags':None},{'points':[],'sourceUrl':None},{'points':[],'unknown':1}]:
            with self.subTest(doc=doc),self.assertRaises(ValueError): rows(doc,6)

    def test_invalid_metadata_is_rejected_by_all_python_paths(self):
        point={'id':'1','name':'山','latitude':35,'longitude':139}
        invalid=[{'license':123},{'attribution':None},{'sourceUrl':[]},
                 {'supplementSources':{'資料':{'source':123}}},
                 {'supplementSources':{'資料':{'unknown':'値'}}},
                 {'latestPointTimestamp':'not-a-date'},
                 {'latestPointTimestamp':'2026-10-14T00:00:00+00:60'},
                 {'latestPointTimestamp':'2026-02-30T00:00:00Z'},
                 {'latestPointTimestamp':'2026-10-14T00:00:00'},
                 {'pointCount':True},{'pointCount':0}]
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'input.json'
            for metadata in invalid:
                with self.subTest(metadata=metadata):
                    doc={'schemaVersion':6,**metadata,'points':[point]}
                    with self.assertRaises(ValueError): rows(doc,6)
                    path.write_text(json.dumps(doc,ensure_ascii=False),encoding='utf-8')
                    with self.assertRaises(ValueError): list(read_points(path,chunk_size=1))
                    with self.assertRaises(ValueError): write_points([point],folder,'output',metadata=metadata)
            path.write_text('{"schemaVersion":6,"pointCount":12,"points":'+json.dumps([dict(point,id=str(i)) for i in range(12)])+'}',encoding='utf-8')
            self.assertEqual(len(list(read_points(path,chunk_size=1))),12)
            path.write_text('{"points":[]}',encoding='utf-8')
            with self.assertRaises(ValueError): list(read_points(path))

    def test_supplement_manifest_accepts_old_and_new_data_versions(self):
        schema=json.loads((ROOT/'points/schemas/supplements-manifest-v1.schema.json').read_bytes())
        self.assertEqual(schema['properties']['dataSchemaVersion']['enum'],[5,6])
        self.assertEqual(schema['properties']['schemaVersion']['const'],1)

    def test_reading_is_written_immediately_after_name(self):
        from scripts.point_document import document
        point={'id':'1','name':'山','latitude':35,'longitude':139,'nameReading':'やま','aliases':[]}
        before=list(point)
        with tempfile.TemporaryDirectory() as folder:
            write_points([point],folder,'output')
            saved=json.loads((Path(folder)/'output.json').read_bytes())['points'][0]
            for row in [saved,document([point])['points'][0]]:
                keys=list(row)
                self.assertEqual(keys[keys.index('name')+1],'nameReading')
                self.assertEqual(row,point)
        self.assertEqual(list(point),before)

    def test_nested_metadata_and_alias_objects_round_trip(self):
        from scripts.point_document import document
        from scripts.supplements import read_supplements, supplement_document
        point={'id':'gsi:1','name':'燧ヶ岳','latitude':35,'longitude':139,'aliases':[{'name':'柴安嵓','nameReading':'しばやすぐら'}]}
        doc=document([point],{'日本百名山':{'reading':'にほんひゃくめいざん'}},license='PDL-1.0')
        self.assertEqual(set(doc),{'schemaVersion','metadata','points'})
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'points.json'
            path.write_text(json.dumps(doc,ensure_ascii=False),encoding='utf-8')
            self.assertEqual(rows(doc,6),[point])
            self.assertEqual(list(read_points(path,chunk_size=1)),[point])
            data={'schemaVersion':1,'metadata':{'sources':{'資料':{'source':'資料名'}},'tags':{}},'points':[{'id':'gsi:1','aliases':[{'name':'柴安嵓','nameReading':'しばやすぐら'}]}]}
            path.write_text(json.dumps(data,ensure_ascii=False),encoding='utf-8')
            self.assertEqual(supplement_document(read_supplements(path)),data)

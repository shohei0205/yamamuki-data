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
    def test_schema_format_identifies_points_and_supplements(self):
        from scripts.point_document import document
        from scripts.supplements import supplement_document, validate_supplements
        point={'id':'1','name':'山','latitude':35,'longitude':139}
        doc=document([point])
        self.assertEqual(doc['schemaFormat'],'yamamuki-points')
        supplement=supplement_document({'schemaVersion':1,'points':[]})
        self.assertEqual(supplement['schemaFormat'],'yamamuki-point-supplements')
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'points.json'
            for value in (None,'yamamuki-point-supplements','yamamuki-manifest',123):
                bad=dict(doc)
                if value is None:bad.pop('schemaFormat')
                else:bad['schemaFormat']=value
                path.write_text(json.dumps(bad),encoding='utf-8')
                with self.subTest(value=value):
                    with self.assertRaises(ValueError):rows(bad)
                    with self.assertRaises(ValueError):list(read_points(path,chunk_size=1))
                bad_supp=dict(supplement)
                if value is None:bad_supp.pop('schemaFormat')
                else:bad_supp['schemaFormat']=value if value != 'yamamuki-point-supplements' else 'yamamuki-points'
                with self.assertRaises(ValueError):validate_supplements(bad_supp)

    def test_multiple_sources_preserve_credits_and_reject_collisions(self):
        from document_metadata import source_entries, validate_metadata
        extra={'reference':{'source':'https://example.com/reference','license':'CC BY 4.0','attribution':'Author','changes':'Names extracted'}}
        sources=source_entries('osm-peaks','https://example.com/osm','ODbL-1.0','OSM contributors',extra)
        self.assertEqual(sources['reference'],extra['reference'])
        self.assertEqual(len(sources),2)
        self.assertNotIn('osm-peaks',extra)
        with self.assertRaises(ValueError):
            source_entries('reference','https://example.com/different',supplements=extra)
        for invalid in (None,[{'name':'duplicate'},{'name':'duplicate'}],{'reference':{'source':123}},{'reference':{'license':123}}):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                validate_metadata({'sources':invalid})

    def test_metadata_survives_streaming_with_any_field_order(self):
        point={'id':'1','name':'山','latitude':35,'longitude':139}
        tags={'日本百名山':{'nameReading':'にほんひゃくめいざん'}}
        info={'sources':{'国土地理院':{'source':'https://example.com','license':'PDL-1.0','attribution':'国土地理院','lastestTimeStamp':'2026-10-14T00:00:00Z'},'資料':{'source':'資料名'}}}
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'input.json'
            for doc in [{'schemaFormat':'yamamuki-points','schemaVersion':6,**info,'tags':tags,'points':[point]},{'points':[point],'tags':tags,**info,'schemaVersion':6,'schemaFormat':'yamamuki-points'}]:
                path.write_text(json.dumps(doc,ensure_ascii=False),encoding='utf-8')
                metadata={}
                self.assertEqual(list(read_points(path,chunk_size=1,metadata=metadata)),[point])
                self.assertEqual(metadata,{'schemaVersion':6,**info,'tags':tags})
                self.assertEqual(rows(doc,6),[point])
            write_points([point],folder,'output',tags=tags,metadata=info)
            from document_metadata import metadata_arrays
            doc=json.loads((Path(folder)/'output.json').read_bytes())
            self.assertEqual(doc,{'schemaFormat':'yamamuki-points','schemaVersion':6,'metadata':metadata_arrays({**info,'tags':tags,'pointCount':1}),'points':[point]})
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
            self.assertEqual(rows({'schemaFormat':'yamamuki-points','schemaVersion':6,'points':[point]},6),[point])
            self.assertEqual(rows([point],5),[point])

    def test_invalid_document_is_rejected(self):
        for doc in [{'points':[],'pointCount':1},{'points':[],'pointCount':True},{'points':[],'latestPointTimestamp':None},{'points':{},'tags':{}},{'points':[],'tags':None},{'points':[],'sourceUrl':None},{'points':[],'unknown':1}]:
            with self.subTest(doc=doc),self.assertRaises(ValueError): rows(doc,6)

    def test_invalid_metadata_is_rejected_by_all_python_paths(self):
        point={'id':'1','name':'山','latitude':35,'longitude':139}
        invalid=[{'sources':{'資料':{'license':123}}},
                 {'sources':{'資料':{'lastestTimeStamp':'not-a-date'}}},
                 {'sources':{'資料':{'lastestTimeStamp':'2026-10-14T00:00:00+00:60'}}},
                 {'sources':{'資料':{'lastestTimeStamp':'2026-02-30T00:00:00Z'}}},
                 {'sources':{'資料':{'lastestTimeStamp':'2026-10-14T00:00:00'}}},
                 {'pointCount':True},{'pointCount':0}]
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'input.json'
            for metadata in invalid:
                with self.subTest(metadata=metadata):
                    doc={'schemaFormat':'yamamuki-points','schemaVersion':6,**metadata,'points':[point]}
                    with self.assertRaises(ValueError): rows(doc,6)
                    path.write_text(json.dumps(doc,ensure_ascii=False),encoding='utf-8')
                    with self.assertRaises(ValueError): list(read_points(path,chunk_size=1))
                    with self.assertRaises(ValueError): write_points([point],folder,'output',metadata=metadata)
            path.write_text('{"schemaFormat":"yamamuki-points","schemaVersion":6,"pointCount":12,"points":'+json.dumps([dict(point,id=str(i)) for i in range(12)])+'}',encoding='utf-8')
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
        doc=document([point],{'日本百名山':{'nameReading':'にほんひゃくめいざん'}},sources={'国土地理院':{'license':'PDL-1.0'}})
        self.assertEqual(set(doc),{'schemaFormat','schemaVersion','metadata','points'})
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'points.json'
            path.write_text(json.dumps(doc,ensure_ascii=False),encoding='utf-8')
            self.assertEqual(rows(doc,6),[point])
            self.assertEqual(list(read_points(path,chunk_size=1)),[point])
            data={'schemaFormat':'yamamuki-point-supplements','schemaVersion':2,'metadata':{'sources':[{'name':'資料','source':'資料名'}],'tags':[]},'points':[{'id':'gsi:1','aliases':[{'name':'柴安嵓','nameReading':'しばやすぐら'}]}]}
            path.write_text(json.dumps(data,ensure_ascii=False),encoding='utf-8')
            self.assertEqual(supplement_document(read_supplements(path)),data)


class MetadataArrayTests(unittest.TestCase):
    def test_arrays_round_trip_and_reject_duplicate_identifiers(self):
        from document_metadata import metadata_maps, metadata_arrays, validate_metadata
        info = {"sources":[{"name":"reference","license":"CC0-1.0"}],"tags":[{"name":"分類","nameReading":"ぶんるい"}]}
        self.assertEqual(metadata_arrays(metadata_maps(info)),info)
        validate_metadata({"sources":[],"tags":[]})
        for field,key in (("sources","name"),("tags","name")):
            for items in ([{key:"同じ"},{key:"同じ"}],[{}],[{key:" 空白"}],[None]):
                with self.subTest(field=field,items=items), self.assertRaises(ValueError):
                    validate_metadata({field:items})


class ExtensibleDocumentTests(unittest.TestCase):
    def test_unknown_fields_are_accepted_without_affecting_known_values(self):
        from document_metadata import validate_metadata
        from scripts.supplements import validate_supplements, apply_supplements
        extra={"future":{"values":[1,None,True]}}
        metadata={**extra,"sources":[{"name":"資料","source":"資料名",**extra}],"tags":[{"name":"分類","nameReading":"ぶんるい",**extra}]}
        validate_metadata(metadata)
        point={"id":"1","name":"山","latitude":35,"longitude":139,"aliases":[{"name":"別名",**extra}],"provenance":extra,**extra}
        doc={"schemaFormat":"yamamuki-points","schemaVersion":6,"metadata":metadata,"points":[point],**extra}
        self.assertEqual(rows(doc,6),[point])
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/"input.json"
            path.write_text(json.dumps(doc),encoding="utf-8")
            self.assertEqual(list(read_points(path,chunk_size=1)),[point])
            write_points([point],folder,"output",tags=metadata["tags"],metadata={k:v for k,v in metadata.items() if k != "tags"})
        supplement={"schemaFormat":"yamamuki-point-supplements","schemaVersion":2,"metadata":metadata,"points":[{"id":"1","name":"新しい山","expected":extra,**extra}],**extra}
        validate_supplements(supplement)
        self.assertEqual(apply_supplements([point],supplement)[0]["name"],"新しい山")


class CommonIdentifierTests(unittest.TestCase):
    def test_point_and_supplement_share_identifier_limits(self):
        from scripts.supplements import validate_supplements
        point={"id":"山:1","name":"山","latitude":35,"longitude":139}
        def check_point(changes):
            return rows({"schemaFormat":"yamamuki-points","schemaVersion":6,"points":[{**point,**changes}]},6)
        def check_supplement(reference):
            return validate_supplements({"schemaFormat":"yamamuki-point-supplements","schemaVersion":2,"points":[{**reference,"name":"山"}]})
        for value in ("山:1","1","a b"):
            check_point({"id":value})
            check_supplement({"id":value})
        for value in (None,1,""," "," a","a "):
            with self.subTest(field="id",value=value):
                with self.assertRaises(ValueError):check_point({"id":value})
                with self.assertRaises(ValueError):check_supplement({"id":value})
        for value in ("osm:1","osm:9007199254740991","gsi-vector:sample"):
            check_point({"relatedIds":[value]})
            check_supplement({"id":"山:1","relatedIds":[value]})
        for value in (None,1,"", " osm:1", "osm:1 "):
            with self.subTest(field="relatedIds",value=value):
                with self.assertRaises(ValueError):check_point({"relatedIds":[value]})
                with self.assertRaises(ValueError):check_supplement({"id":"山:1","relatedIds":[value]})
        with self.assertRaises(ValueError):check_point({"osmId":1})
        with self.assertRaises(ValueError):check_supplement({"osmId":1})


class RelatedPointTests(unittest.TestCase):
    def test_legacy_osm_relation_is_preserved_without_self_relation(self):
        from document_metadata import distributed_point
        self.assertEqual(distributed_point({"id":"osm:123","osmId":123,"name":"山"}),{"id":"osm:123","name":"山"})
        self.assertEqual(distributed_point({"id":"gsi-vector:sample","osmId":123,"name":"山","relatedIds":["custom:sample"]})["relatedIds"],["custom:sample","osm:123"])

    def test_related_ids_replace_clear_and_do_not_resolve_target(self):
        from scripts.supplements import apply_supplements,validate_supplements
        point={"id":"gsi-vector:sample","name":"山","relatedIds":["custom:old"]}
        def patch(values):
            return {"schemaFormat":"yamamuki-point-supplements","schemaVersion":2,"points":[{"id":point["id"],"relatedIds":values,"expected":{"relatedIds":["custom:old"]}}]}
        before=json.dumps(point)
        applied=apply_supplements([point],patch(["osm:123","custom:not-loaded"]))
        self.assertEqual(applied[0]["relatedIds"],["osm:123","custom:not-loaded"])
        self.assertEqual(apply_supplements([point],patch([]))[0]["relatedIds"],[])
        self.assertEqual(json.dumps(point),before)
        for values in [None,"osm:123",[None],[1],[""],[" osm:123"],["osm:123","osm:123"],[point["id"]]]:
            with self.subTest(values=values),self.assertRaises(ValueError):
                validate_supplements(patch(values))
        wrong=patch(["osm:123"]);wrong["points"][0]["expected"]["relatedIds"]=[]
        with self.assertRaises(ValueError):apply_supplements([point],wrong)
        with self.assertRaises(ValueError):apply_supplements([point],{"schemaFormat":"yamamuki-point-supplements","schemaVersion":2,"points":[{"id":"osm:123","name":"別の名前"}]})


class DocumentLayoutTests(unittest.TestCase):
    def test_one_point_per_line_preserves_nested_fields(self):
        from document_metadata import point_document_json
        import json
        body={"schemaFormat":"yamamuki-points","schemaVersion":6,"metadata":{"pointCount":2},"points":[{"id":"osm:1","name":"山\n峰","aliases":[{"name":"別名"}]},{"id":"gsi-vector:2","name":"山2"}]}
        raw=point_document_json(body)
        self.assertEqual(json.loads(raw),body)
        lines=[line.strip().removesuffix(",") for line in raw.splitlines() if line.lstrip().startswith('{"id":')]
        self.assertEqual([json.loads(line) for line in lines],body["points"])
        self.assertTrue(raw.endswith("\n"))

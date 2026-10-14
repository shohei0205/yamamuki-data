"""ブラウザと生成側で同じ単一補足形式を適用する。"""
import json
from pathlib import Path
import shutil
import subprocess
import unittest


class UnifiedViewerTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('node'),'Node.js がありません')
    def test_unified_validation_application_and_roundtrip(self):
        page=(Path(__file__).resolve().parents[1]/'viewer.html').read_text(encoding='utf-8')
        source=page.split('<script>')[1].split('// 穴のあるポリゴン')[0]
        source += 'function entriesFromJson('+page.split('function entriesFromJson(',1)[1].split('\n',1)[0]+'\n'
        source += 'function pointSearchText('+page.split('function pointSearchText(',1)[1].split('\n',1)[0]+'\n'
        checks=r'''
const assert=require('node:assert/strict');
const base=[{osmId:'1',pointName:'元の山',name:'元の山',tags:['元タグ'],source:{name:'元の山',nameReading:'もと',aliases:['旧別名']}}];
const data={schemaVersion:1,sources:{資料:{source:'https://example.com'}},points:[{osmId:1,name:'補正した山',nameReading:null,aliases:['別名'],aliasesRemove:['旧別名'],tags:['分類A','分類B'],reason:'資料で確認',expected:{name:'元の山'}}]};
const withReadings=applySupplement(base,{schemaVersion:1,points:[{osmId:1,aliases:[{name:'柴安嵓',nameReading:'しばやすぐら'}]}]})[0];assert.deepEqual(withReadings.aliasReadings,{'柴安嵓':'しばやすぐら'});assert.ok(pointSearchText(withReadings).includes('しばやすぐら'));
assert.deepEqual(applySupplement([withReadings],{schemaVersion:1,points:[{osmId:1,aliases:[{name:'柴安嵓',nameReading:null}]}]})[0].aliasReadings,{});
assert.deepEqual(applySupplement([withReadings],{schemaVersion:1,points:[{osmId:1,aliasesRemove:['柴安嵓']}]})[0].aliasReadings,{});
assert.throws(()=>applySupplement(base,{schemaVersion:1,points:[{osmId:1,aliasReadings:{'未登録':'よみ'}}]}));
for(const aliasReadings of [null,[],{'柴安嵓':''},{' 柴安嵓':'よみ'},{'柴安嵓':1}])assert.throws(()=>validateSupplement({schemaVersion:1,points:[{osmId:1,aliasReadings}]}));
const tags={'日本百名山':{reading:'にほんひゃくめいざん'},'読みなし':{}};assert.deepEqual(parseSupplement(JSON.stringify({schemaVersion:1,tags,points:[]})).tags,tags);for(const invalid of [null,[],{'タグ':{reading:''}},{' タグ':{}},{'タグ':{extra:1}}])assert.throws(()=>validateTagMetadata(invalid));
const doc={schemaVersion:6,sourceUrl:'https://example.com',license:'PDL-1.0',attribution:'国土地理院',supplementSources:{資料:{source:'資料名'}},tags:{'日本百名山':{reading:'にほんひゃくめいざん'}},points:[{id:'gsi:1',name:'山',latitude:35,longitude:139}]};const loaded=entriesFromJson(doc);assert.equal(loaded.length,1);assert.deepEqual(loaded.metadata.tags,doc.tags);assert.equal(loaded.metadata.license,doc.license);assert.equal(entriesFromJson(doc.points).length,1);assert.throws(()=>entriesFromJson({...doc,tags:null}));
const oldPoint={osmId:123456,name:'見本山',latitude:35,longitude:139,elevationM:1000};const oldOriginal=JSON.stringify(oldPoint);for(const extra of [{},{nameReading:'みほんやま',aliases:[],wikipediaUrl:null,wikidataUrl:null}]){const old=entriesFromJson([{...oldPoint,...extra}])[0];assert.equal(old.osmId,'123456');assert.equal(old.source.id,undefined);assert.equal(old.pointName,'見本山');}assert.equal(JSON.stringify(oldPoint),oldOriginal);assert.throws(()=>entriesFromJson({points:[oldPoint]}));assert.throws(()=>entriesFromJson([{...oldPoint,osmId:0}]));assert.throws(()=>entriesFromJson([{...oldPoint,osmId:'123456'}]));assert.throws(()=>entriesFromJson([oldPoint,oldPoint]));
for(const bad of [{points:[]},{latestPointTimestamp:'not-a-date'},{latestPointTimestamp:'2026-10-14T00:00:00+00:60'},{latestPointTimestamp:'2026-02-30T00:00:00Z'},{latestPointTimestamp:'2026-10-14T00:00:00'},{license:123},{supplementSources:{資料:{source:123}}}])assert.throws(()=>entriesFromJson({...doc,...bad}));
const nested={schemaVersion:1,metadata:{sources:{資料:{source:'資料名'}},tags:{日本百名山:{reading:'にほんひゃくめいざん'}}},points:[{osmId:1,aliases:[{name:'柴安嵓',nameReading:'しばやすぐら'}]}]};assert.deepEqual(supplementJson(parseSupplement(JSON.stringify(nested))),nested);const nestedDoc=entriesFromJson({schemaVersion:6,metadata:{pointCount:1,license:'PDL-1.0',tags:nested.metadata.tags},points:[{id:'gsi:1',name:'燧ヶ岳',latitude:35,longitude:139,aliases:[{name:'柴安嵓',nameReading:'しばやすぐら'}]}]});assert.ok(pointSearchText(nestedDoc[0]).includes('しばやすぐら'));assert.equal(nestedDoc.metadata.pointCount,1);
const original=JSON.stringify(base);const parsed=parseSupplement(JSON.stringify(data));const result=applySupplement(base,parsed)[0];assert.equal(result.pointName,'補正した山');assert.equal(result.nameReading,null);assert.deepEqual(result.aliases,['別名']);assert.deepEqual(result.tags,['元タグ','分類A','分類B']);assert.equal(JSON.stringify(base),original);assert.deepEqual(parseSupplement(JSON.stringify(parsed)),data);assert.deepEqual(parseSupplement(JSON.stringify({schemaVersion:1,points:[{osmId:1,aliases:[{name:'柴安嵓',nameReading:'しばやすぐら'}]}]})).points[0].aliases,[{name:'柴安嵓',nameReading:'しばやすぐら'}]);
assert.equal(supplementTables(data).size,3);assert.ok(supplementTables(data).has('分類A'));assert.ok(supplementTables(data).has('分類B'));
for(const fields of [{name:'補正名'},{exclude:true},{nameReading:null},{aliasesRemove:['旧別名']}])for(const reason of [{},{reason:''},{reason:'資料で確認'}]){const optional={schemaVersion:1,points:[{osmId:1,...fields,...reason}]};validateSupplement(optional);applySupplement(base,optional);}
assert.equal(applySupplement(base,{schemaVersion:1,points:[{osmId:1,exclude:true,reason:'重複'}]})[0].excluded,true);assert.deepEqual(applySupplement(base,{schemaVersion:1,points:[{osmId:1,tagsRemove:['元タグ']}]})[0].tags,[]);
for(const update of [{name:''},{reason:1},{nameReading:''},{tags:['a','a']},{aliasesRemove:['別名']},{references:{name:['不明']}},{extra:true}])assert.throws(()=>validateSupplement({...data,points:[{...data.points[0],...update}]}));
assert.throws(()=>applySupplement(base,{...data,points:[{...data.points[0],expected:{name:'別の山'}}]}),/想定/);
assert.throws(()=>applySupplement(base,{...data,points:[{...data.points[0],osmId:999}]}),/存在しない/);
'''
        result=subprocess.run([shutil.which('node'),'-e',source+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode('utf-8',errors='replace'))

    @unittest.skipUnless(shutil.which('node'),'Node.js がありません')
    def test_graphic_deletion_validation_and_export(self):
        page=(Path(__file__).resolve().parents[1]/'viewer.html').read_text(encoding='utf-8')
        source=page.split('<script>')[1].split('// 穴のあるポリゴン')[0]
        source+='function exportSupplement('+page.split('function exportSupplement(',1)[1].split('function refreshSupplement',1)[0]
        checks=r'''
const assert=require('node:assert/strict');
const originalGraphic={svg:'original',scale:2};
const basePoints=[{osmId:'1',tags:[],source:{graphic:originalGraphic}}];
const supplementDocument={schemaVersion:1,points:[{osmId:1,graphic:null}]};
const tagEdits=new Map([[0,{add:new Set(),remove:new Set()}]]);
assert.equal(applySupplement(basePoints,supplementDocument)[0].graphic,null);
assert.equal(basePoints[0].source.graphic,originalGraphic);
assert.equal(exportSupplement().points[0].graphic,null);
assert.deepEqual(parseSupplement(JSON.stringify(exportSupplement())),supplementDocument);
for(const graphic of [{},{svg:3},{svg:''},{svg:'x',extra:1},... [0,-1,true,'2',Infinity].map(scale=>({svg:'x',scale}))])assert.throws(()=>validateSupplement({schemaVersion:1,points:[{osmId:1,graphic}]}));
assert.deepEqual(applySupplement(basePoints,{schemaVersion:1,points:[]})[0].source.graphic,originalGraphic);
'''
        result=subprocess.run([shutil.which('node'),'-e',source+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode('utf-8',errors='replace'))

    @unittest.skipUnless(shutil.which('node'),'Node.js がありません')
    def test_detail_describes_actual_changes(self):
        page=(Path(__file__).resolve().parents[1]/'viewer.html').read_text(encoding='utf-8')
        aliases='function aliasEntries('+page.split('function aliasEntries(',1)[1].split('function validateDocumentMetadata(',1)[0]
        source=aliases+'function supplementChangeLines('+page.split('function supplementChangeLines(',1)[1].split('function showSupplementOrigin',1)[0]
        checks=r'''
const assert=require('node:assert/strict');
const source={name:'元の山',nameReading:'もと',tags:['残すタグ','旧タグ'],aliases:['旧別名']};
const point={source,pointName:'新しい山',nameReading:null,tags:['残すタグ','日本百名山'],aliases:['新別名'],excluded:true};
assert.deepEqual(supplementChangeLines(point,{reason:'資料を確認'}),['地点名: 元の山 → 新しい山','よみがな: もと → なし（削除）','タグを追加: 日本百名山','タグを削除: 旧タグ','別名を追加: 新別名','別名を削除: 旧別名','表示・配布対象から除外','理由: 資料を確認']);
assert.deepEqual(supplementChangeLines({source,pointName:source.name,tags:source.tags},{}),['補足による変更なし']);
'''
        result=subprocess.run([shutil.which('node'),'-e',source+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode('utf-8',errors='replace'))

    @unittest.skipUnless(shutil.which('node'),'Node.js がありません')
    def test_graphic_columns_use_effective_image_and_numeric_scale(self):
        page=(Path(__file__).resolve().parents[1]/'viewer.html').read_text(encoding='utf-8')
        source='function pointGraphic('+page.split('function pointGraphic(',1)[1].split('function appendGraphicCells',1)[0]
        source+='function pointSortKey('+page.split('function pointSortKey(',1)[1].split('function sortPointRows',1)[0]
        checks=r'''
const assert=require('node:assert/strict');
const points=[{source:{graphic:{svg:'original',scale:10}}},{source:{graphic:{svg:'original'}},graphic:{svg:'supplement',scale:2}},{source:{graphic:{svg:'original'}},graphic:null},{source:{graphic:{svg:'default'}}}];
const sortKeys=new Map();let sortColumn=8;
assert.equal(pointGraphic(points[0]).svg,'original');assert.equal(pointGraphic(points[1]).svg,'supplement');assert.equal(pointGraphic(points[2]),null);
assert.deepEqual(points.map((_,index)=>pointSortKey(index)),[10,2,null,1]);
assert.equal(graphicScaleText({}), '1.0倍');assert.equal(graphicScaleText({scale:2}), '2.0倍');assert.equal(graphicScaleText({scale:1.25}), '1.25倍');
'''
        result=subprocess.run([shutil.which('node'),'-e',source+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode('utf-8',errors='replace'))

    @unittest.skipUnless(shutil.which('node'), 'Node.js がありません')
    def test_id_references_gsi_export_and_batch_addition(self):
        page = (Path(__file__).resolve().parents[1] / 'viewer.html').read_text(encoding='utf-8')
        source = page.split('<script>')[1].split('let points=[];')[0]
        source += 'function exportSupplement(' + page.split('function exportSupplement(', 1)[1].split('function refreshSupplement', 1)[0]
        checks = r'''
const assert=require('node:assert/strict');
const gsi={osmId:'',pointName:'山',name:'山',tags:['元タグ'],source:{id:'gsi-mountain-list:1',name:'山'}};
const other={osmId:'',pointName:'別の山',name:'別の山',tags:[],source:{id:'gsi-vector:2',name:'別の山'}};
const basePoints=[gsi,other];
const supplementDocument={schemaVersion:1,points:[{id:gsi.source.id,tags:['日本百名山']}]};
const tagEdits=new Map([[1,{add:new Set(['日本百名山']),remove:new Set()}]]);
const exported=exportSupplement();assert.equal(exported.points.length,2);assert.equal(exported.points[0].id,gsi.source.id);assert.equal(exported.points[1].id,other.source.id);assert.equal(Object.hasOwn(exported.points[1],'osmId'),false);
assert.deepEqual(applySupplement(basePoints,parseSupplement(JSON.stringify(exported))).map(p=>p.tags),[['元タグ','日本百名山'],['日本百名山']]);
for(const ref of [{},{id:'1',osmId:1},{id:''},{id:1},{id:' 1'},{osmId:null}])assert.throws(()=>validateSupplement({schemaVersion:1,points:[{...ref,tags:[]}]}));
const osm=[{osmId:'1',pointName:'A',tags:[],source:{id:'2'}},{osmId:'2',pointName:'B',tags:[],source:{id:'1'}}];
assert.deepEqual(applySupplement(osm,{schemaVersion:1,points:[{id:'1',tags:['B']},{osmId:1,tags:['A']}]}).map(p=>p.tags),[['A'],['B']]);
assert.throws(()=>applySupplement(osm,{schemaVersion:1,points:[{id:'2',tags:['A']},{osmId:1,tags:['B']}]}),/同じ地点/);
assert.throws(()=>applySupplement(basePoints,{schemaVersion:1,points:[{id:'missing',tags:[]}]}),/存在しない/);
const batch=completeTagList('#id,name\ngsi-mountain-list:1,山',basePoints);assert.deepEqual(batch.issues,[]);assert.deepEqual(batch.mappings,[{id:gsi.source.id,name:'山'}]);
const named=completeTagList('山',basePoints);assert.deepEqual(named.mappings,batch.mappings);
assert.deepEqual(completeTagList(mappingListText(batch.mappings),basePoints).mappings,batch.mappings);
assert.deepEqual(completeTagList('#id,name\n2,A',osm).mappings,[{id:'2',name:'A'}]);
assert.ok(completeTagList('#id,osmId,name\n2,1,A',osm).issues.length);
assert.ok(completeTagList('#id,name\n2,A\n2,A',osm).issues.length);
const added=addSupplementTags(structuredClone(supplementDocument),batch.mappings,'別タグ',basePoints);assert.equal(added.points.length,1);assert.deepEqual(added.points[0].tags,['日本百名山','別タグ']);
const existingId={schemaVersion:1,points:[{id:'2',tags:['元タグ']}]};addSupplementTags(existingId,[{osmId:'1',name:'A'}],'新タグ',osm);assert.equal(existingId.points.length,1);assert.equal(existingId.points[0].id,'2');assert.deepEqual(existingId.points[0].tags,['元タグ','新タグ']);
'''
        result = subprocess.run([shutil.which('node'), '-'], input=(source + checks).encode('utf-8'), capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode('utf-8', errors='replace'))

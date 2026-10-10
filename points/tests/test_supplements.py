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
        checks=r'''
const assert=require('node:assert/strict');
const base=[{osmId:'1',pointName:'元の山',name:'元の山',tags:['元タグ'],source:{name:'元の山',nameReading:'もと',aliases:['旧別名']}}];
const data={schemaVersion:1,sources:{資料:{source:'https://example.com'}},points:[{osmId:1,name:'補正した山',nameReading:null,aliases:['別名'],aliasesRemove:['旧別名'],tags:['分類A','分類B'],reason:'資料で確認',expected:{name:'元の山'}}]};
const original=JSON.stringify(base);const parsed=parseSupplement(JSON.stringify(data));const result=applySupplement(base,parsed)[0];assert.equal(result.pointName,'補正した山');assert.equal(result.nameReading,null);assert.deepEqual(result.aliases,['別名']);assert.deepEqual(result.tags,['元タグ','分類A','分類B']);assert.equal(JSON.stringify(base),original);assert.deepEqual(parseSupplement(JSON.stringify(parsed)),data);
assert.equal(supplementTables(data).size,3);assert.ok(supplementTables(data).has('分類A'));assert.ok(supplementTables(data).has('分類B'));
assert.equal(applySupplement(base,{schemaVersion:1,points:[{osmId:1,exclude:true,reason:'重複'}]})[0].excluded,true);assert.deepEqual(applySupplement(base,{schemaVersion:1,points:[{osmId:1,tagsRemove:['元タグ']}]})[0].tags,[]);
for(const update of [{name:''},{reason:''},{nameReading:''},{tags:['a','a']},{aliasesRemove:['別名']},{references:{name:['不明']}},{extra:true}])assert.throws(()=>validateSupplement({...data,points:[{...data.points[0],...update}]}));
assert.throws(()=>applySupplement(base,{...data,points:[{...data.points[0],expected:{name:'別の山'}}]}),/想定/);
assert.throws(()=>applySupplement(base,{...data,points:[{...data.points[0],osmId:999}]}),/存在しない/);
'''
        result=subprocess.run([shutil.which('node'),'-e',source+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode('utf-8',errors='replace'))

    @unittest.skipUnless(shutil.which('node'),'Node.js がありません')
    def test_detail_describes_actual_changes(self):
        page=(Path(__file__).resolve().parents[1]/'viewer.html').read_text(encoding='utf-8')
        source='function supplementChangeLines('+page.split('function supplementChangeLines(',1)[1].split('function showSupplementOrigin',1)[0]
        checks=r'''
const assert=require('node:assert/strict');
const source={name:'元の山',nameReading:'もと',tags:['残すタグ','旧タグ'],aliases:['旧別名']};
const point={source,pointName:'新しい山',nameReading:null,tags:['残すタグ','日本百名山'],aliases:['新別名'],excluded:true};
assert.deepEqual(supplementChangeLines(point,{reason:'資料を確認'}),['地点名: 元の山 → 新しい山','よみがな: もと → なし（削除）','タグを追加: 日本百名山','タグを削除: 旧タグ','別名を追加: 新別名','別名を削除: 旧別名','表示・配布対象から除外','理由: 資料を確認']);
assert.deepEqual(supplementChangeLines({source,pointName:source.name,tags:source.tags},{}),['補足による変更なし']);
'''
        result=subprocess.run([shutil.which('node'),'-e',source+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode('utf-8',errors='replace'))

"""改名時に名前と読みを別名へ移す編集操作を確かめる。"""
import shutil
import subprocess
from pathlib import Path
import unittest


class RenameTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node.js がありません")
    def test_nearby_actions_keep_multiple_filters_and_search(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("selectedFilterValues","refreshSupplementKeepingFilters")
        code="\n".join(next(line for line in page.splitlines() if line.startswith("function "+name+"(")) for name in names)
        checks=r"""
const assert=require('node:assert/strict');
function select(values){const options=['all',...values].map(value=>({value,selected:value!=='all'}));return {options,get selectedOptions(){return this.options.filter(option=>option.selected);}};}
const tagFilter=select(['tag:A','tag:B']),provenanceFilters=[select(['vector']),select(['312','810']),select(['dem1a_png'])];
const inputs={search:{value:'山名'},'hide-excluded':{checked:true}};const document={getElementById(id){return inputs[id];}};
let filtered=false;
function refreshSupplement(){for(const field of [tagFilter,...provenanceFilters])field.options=field.options.map(option=>({value:option.value,selected:option.value==='all'}));inputs.search.value='';inputs['hide-excluded'].checked=false;}
function filter(){filtered=true;assert.deepEqual(selectedFilterValues(tagFilter),['tag:A','tag:B']);assert.deepEqual(selectedFilterValues(provenanceFilters[1]),['312','810']);assert.equal(inputs.search.value,'山名');}
refreshSupplementKeepingFilters();
assert.ok(filtered);assert.deepEqual(selectedFilterValues(provenanceFilters[0]),['vector']);assert.deepEqual(selectedFilterValues(provenanceFilters[2]),['dem1a_png']);assert.equal(inputs['hide-excluded'].checked,true);
"""
        result=subprocess.run(["node","-e",code+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode("utf-8",errors="replace"))
        handlers=next(line for line in page.splitlines() if line.startswith('function renderNearby('))
        self.assertEqual(handlers.count('refreshSupplementKeepingFilters();'),2)

    @unittest.skipUnless(shutil.which("node"), "Node.js がありません")
    def test_inherit_nearby_name_and_exclude_as_one_supplement(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("pointReference","pointReferenceKeys","supplementKey","supplementMatches","supplementForPoint","aliasEntries","mergedAliasEntries","inheritedAliasEntries","moveNameToAlias","excludeNearbyPoint","inheritNearbyName")
        code="\n".join(next(line for line in page.splitlines() if line.startswith("function "+name+"(")) for name in names)
        checks=r"""
const assert=require('node:assert/strict');
const current={source:{id:'gsi:1',name:'元の山'},pointName:'柴安嵓',nameReading:'しばやすぐら'};
const nearby={osmId:'123',source:{id:'123',name:'別の山'},pointName:'燧ヶ岳',nameReading:'ひうちがたけ'};
const data={schemaFormat:'yamamuki-point-supplements',schemaVersion:2,points:[{id:'gsi:1',tags:['日本百名山'],aliases:[{name:'別名'},{name:'柴安嵓',nameReading:'旧読み'}],aliasesRemove:['柴安嵓'],reason:'既存の理由'},{osmId:123,note:'相手のメモ',nameReading:'ひうちがたけ'}]};
const before=JSON.stringify(data);const result=inheritNearbyName(data,current,nearby);
assert.equal(JSON.stringify(data),before);assert.equal(result.points.length,2);
assert.deepEqual(result.points[0],{id:'gsi:1',name:'燧ヶ岳',nameReading:'ひうちがたけ',tags:['日本百名山'],aliases:[{name:'別名'},{name:'柴安嵓',nameReading:'しばやすぐら'}],reason:'既存の理由'});
assert.deepEqual(result.points[1],{...data.points[1],exclude:true});
assert.equal(current.pointName,'柴安嵓');assert.equal(nearby.pointName,'燧ヶ岳');
const missing=inheritNearbyName(data,current,{...nearby,nameReading:null});assert.equal(missing.points[0].nameReading,null);
assert.throws(()=>inheritNearbyName(data,current,current));
const recipient={...current,source:{...current.source,aliases:[{name:'基準の既存別名',nameReading:'きじゅん'},{name:'燧ヶ岳',nameReading:'ひうちがたけ'}]},aliases:['基準の既存別名','燧ヶ岳'],aliasReadings:{'基準の既存別名':'きじゅん','燧ヶ岳':'ひうちがたけ'}};
const donor={...nearby,aliases:['相手の既存別名','基準の既存別名'],aliasReadings:{'相手の既存別名':['よみ1','よみ2'],'基準の既存別名':'きじゅんべつよみ'}};
const preserved=inheritNearbyName(data,recipient,donor).points[0];
assert.deepEqual(preserved.aliases,[{name:'基準の既存別名',nameReading:['きじゅん','きじゅんべつよみ']},{name:'別名'},{name:'柴安嵓',nameReading:'しばやすぐら'},{name:'相手の既存別名',nameReading:['よみ1','よみ2']}]);
assert.ok(preserved.aliasesRemove.includes('燧ヶ岳'));assert.ok(!preserved.aliasesRemove.includes('柴安嵓'));
assert.deepEqual(inheritedAliasEntries([{name:'別名',nameReading:'よみ'}],[{name:'別名',nameReading:'よみ'},{name:'削除済み'}],['削除済み']),[{name:'別名',nameReading:'よみ'}]);
"""
        result=subprocess.run(["node","-e",code+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode("utf-8",errors="replace"))

    @unittest.skipUnless(shutil.which("node"), "Node.js がありません")
    def test_nearby_exclusion_preserves_supplements_and_supports_both_ids(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("pointReference","pointReferenceKeys","supplementKey","supplementMatches","supplementForPoint","excludeNearbyPoint")
        code="\n".join(next(line for line in page.splitlines() if line.startswith("function "+name+"(")) for name in names)
        checks=r"""
const assert=require('node:assert/strict');
const data={schemaFormat:'yamamuki-point-supplements',schemaVersion:2,sources:{資料:{source:'URL'}},points:[{id:'gsi:1',name:'補正名',aliases:[{name:'旧名',nameReading:'きゅうめい'}],tags:['タグ']}]};
const snapshot=JSON.stringify(data);
let result=excludeNearbyPoint(data,{source:{id:'gsi:1'},pointName:'別の名前'});
assert.equal(JSON.stringify(data),snapshot);assert.equal(result.points.length,1);
assert.deepEqual(result.points[0],{...data.points[0],exclude:true});
result=excludeNearbyPoint(result,{osmId:'123',source:{id:'123'},pointName:'同名でなくてもよい'});
assert.deepEqual(result.points[1],{id:'osm:123',exclude:true});
assert.equal(excludeNearbyPoint(result,{osmId:'123'}).points.length,2);
"""
        result=subprocess.run(["node","-e",code+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode("utf-8",errors="replace"))

    @unittest.skipUnless(shutil.which("node"), "Node.js がありません")
    def test_move_current_name_and_reading_without_losing_existing_aliases(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        lines=page.splitlines()
        code="\n".join(next(line for line in lines if line.startswith("function "+name+"(")) for name in ("parseReadingInput","aliasEntries","aliasEditorText","parseAliasEditor","moveNameToAlias"))
        handler=next(line for line in lines if line.startswith("document.getElementById('rename-with-alias').addEventListener"))
        setup=r"""
const assert=require('node:assert/strict');
const fields={};const events={};const document={getElementById(id){return fields[id]??= {value:'',disabled:false,textContent:'',focus(){this.focused=true;},addEventListener(_,callback){events[id]=callback;}};}};
const points=[{source:{name:'柴安嵓',nameReading:'しばやすぐら'}}];let supplementEditIndex=0;
"""
        checks=r"""
fields['supplement-reading-mode']={value:'original'};
fields['supplement-aliases']={value:JSON.stringify([{name:'別の別名',nameReading:'べつ'},{name:'柴安嵓',nameReading:'旧読み'}])};
fields['supplement-aliases-remove']={value:['柴安嵓','削除する別名'].join('\n')};
events['rename-with-alias']();
assert.deepEqual(parseAliasEditor(fields['supplement-aliases'].value),[{name:'別の別名',nameReading:'べつ'},{name:'柴安嵓',nameReading:'しばやすぐら'}]);
assert.equal(fields['supplement-aliases'].value.split('\n').length,2);
assert.equal(fields['supplement-aliases-remove'].value,'削除する別名');
assert.equal(fields['supplement-name'].value,'');assert.equal(fields['supplement-name'].focused,true);
assert.equal(fields['supplement-reading-mode'].value,'set');assert.equal(fields['rename-with-alias'].disabled,true);
assert.equal(points[0].source.name,'柴安嵓');
assert.deepEqual(parseAliasEditor('  \n{"name":"山"}\r\n\n{"name":"峰","nameReading":"みね"}'),[{name:'山'},{name:'峰',nameReading:'みね'}]);
assert.deepEqual(parseAliasEditor(''),[]);
assert.throws(()=>parseAliasEditor('{"name":"山"}\n壊れたJSON'),/2 行目/);
assert.throws(()=>parseAliasEditor('null'));
assert.throws(()=>parseAliasEditor('{"name":"山"}\n{"name":"山"}'),/重複/);
assert.deepEqual(parseReadingInput('しろうまだけ\nはくばだけ'),['しろうまだけ','はくばだけ']);
assert.throws(()=>parseReadingInput('しろうまだけ\nしろうまだけ'),/重複/);
assert.throws(()=>parseReadingInput('  '));
assert.deepEqual(moveNameToAlias('白馬岳',['しろうまだけ','はくばだけ'],[],[]).aliases,[{name:'白馬岳',nameReading:['しろうまだけ','はくばだけ']}]);
const missing=moveNameToAlias('山',null,[],[]);assert.deepEqual(missing.aliases,[{name:'山'}]);
fields['supplement-name'].value='編集中の名前';fields['supplement-reading'].value='へんしゅう';fields['supplement-reading-mode'].value='set';fields['supplement-aliases'].value='[]';
events['rename-with-alias']();assert.deepEqual(parseAliasEditor(fields['supplement-aliases'].value),[{name:'編集中の名前',nameReading:'へんしゅう'}]);
fields['supplement-name'].value='残す名前';fields['supplement-aliases'].value='不正なJSON';
events['rename-with-alias']();assert.equal(fields['supplement-name'].value,'残す名前');assert.ok(fields['supplement-edit-status'].textContent);
"""
        result=subprocess.run(["node","-e",code+setup+handler+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode("utf-8",errors="replace"))

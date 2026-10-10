"""共通ビューアのタグ JSON 検査・タグ付与を Node.js で確かめる。"""

from pathlib import Path
import shutil
import subprocess
import unittest


class BrowserTagJsonTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node.js がないためブラウザ用タグ JSON 処理のテストを実行できません")
    def test_tag_json_validation_and_overlay(self):
        viewer = Path(__file__).resolve().parents[1] / "viewer.html"
        source = viewer.read_text(encoding="utf-8").split("<script>\n", 1)[1].split("let points=[];", 1)[0]
        checks = r'''
const assert=require('node:assert/strict');
const boundary=prefectureIndex({type:'FeatureCollection',features:[{properties:{nam_ja:'甲県'},geometry:{type:'Polygon',coordinates:[[[0,0],[4,0],[4,4],[0,4],[0,0]],[[1,1],[2,1],[2,2],[1,2],[1,1]]]}},{properties:{nam_ja:'乙県'},geometry:{type:'MultiPolygon',coordinates:[[[[4,0],[6,0],[6,4],[4,4],[4,0]]],[[[10,10],[11,10],[11,11],[10,11],[10,10]]]]}}]});
assert.equal(findPrefectures({longitude:.5,latitude:.5},boundary),'甲県');assert.equal(findPrefectures({longitude:1.5,latitude:1.5},boundary),'');assert.equal(findPrefectures({longitude:4,latitude:3},boundary),'甲県・乙県');assert.equal(findPrefectures({longitude:10.5,latitude:10.5},boundary),'乙県');assert.equal(findPrefectures({longitude:99,latitude:99},boundary),'');assert.equal(findPrefectures({longitude:null,latitude:null},boundary),'');

const cityTopology={type:'Topology',transform:{scale:[.5,.5],translate:[0,0]},arcs:[[[0,0],[4,0],[0,4],[-4,0],[0,-4]]],objects:{municipalities:{type:'GeometryCollection',geometries:[{type:'Polygon',properties:{prefecture:'甲県',displayName:'乙市'},arcs:[[0]]}]}}};
assert.deepEqual(findMunicipalities({longitude:1,latitude:1},municipalityIndex(cityTopology)),{prefecture:'甲県',municipality:'乙市'});assert.deepEqual(findMunicipalities({longitude:9,latitude:9},municipalityIndex(cityTopology)),{prefecture:'',municipality:''});
cityTopology.objects.municipalities.geometries[0].arcs=[[-1]];assert.equal(findMunicipalities({longitude:1,latitude:1},municipalityIndex(cityTopology)).municipality,'乙市');
const metadata={source:'Wikipedia 日本百名山',license:'CC BY-SA 4.0',attribution:'Wikipedia の投稿者',changes:'山名抽出\nOSM ID を追加'};
const table=(id=1,name='山',tag='分類')=>parseTagJson(JSON.stringify({tag,points:[{osmId:id,note:name}]}),tag+'.json');
const withMetadata=parseTagJson(tagJsonText('分類',[{osmId:'1',note:'山'}],metadata),'分類.json');assert.deepEqual(withMetadata.metadata,metadata);assert.equal(withMetadata.mappings[0].osmId,'1');
for(const row of [{osmId:0,note:'山'},{osmId:true,note:'山'},{osmId:'1',note:'山'},{osmId:1,note:1},{osmId:1,name:'山'},{note:'山'},{osmId:1,note:'山',extra:1}])assert.throws(()=>parseTagJson(JSON.stringify({tag:'分類',points:[row]}),'分類.json'));
for(const data of [[],{tag:'別名',points:[]},{tag:'分類',source:[],points:[]},{tag:'分類',points:[{osmId:1,note:'山'},{osmId:1,note:'山'}]},{tag:'分類',points:[],extra:1}])assert.throws(()=>parseTagJson(JSON.stringify(data),'分類.json'));
const original={id:'1',name:'山',aliases:['別名'],osmId:1,tags:['既存']};
const base=[{osmId:'1',pointName:'山',tags:['既存'],source:original}];
const tables=new Map([['分類',table(1,'別名')]]);
const result=applyTagTables(base,tables);assert.deepEqual(result.entries[0].tags,['既存','分類']);assert.equal(result.warnings.length,0);assert.deepEqual(original.tags,['既存']);assert.deepEqual(base[0].tags,['既存']);
tables.set('分類',table(1,'自由なメモ\n改行も可'));assert.equal(applyTagTables(base,tables).warnings.length,0);
const edited=applyTagEdits(result.entries,new Map([[0,{add:new Set(['手動']),remove:new Set(['既存'])}]]));
assert.deepEqual(edited[0].tags,['分類','手動']);assert.deepEqual(original.tags,['既存']);
const mountains=[{osmId:'1',pointName:'羊蹄山',latitude:42,longitude:140,source:{aliases:['蝦夷富士']}},{osmId:'2',pointName:'同名山',latitude:35,longitude:139},{osmId:'3',pointName:'同名山',latitude:36,longitude:140}];
for(const text of ['羊蹄山','1','1,羊蹄山','羊蹄山,1','#osmId,name\n,羊蹄山','#osmId,name\n,羊蹄山','name\n蝦夷富士','osmId\n1','#name\tosmId\n羊蹄山\t1']){const completed=completeTagList(text,mountains);assert.deepEqual(completed.issues,[]);assert.deepEqual(completed.mappings,[{osmId:'1',name:'羊蹄山'}]);}
for(const text of ['不明山','999','1,違う山','1\n羊蹄山','0','#osmId,name\n1','osmId\n1.5'])assert.ok(completeTagList(text,mountains).issues.length,text);
const multiple=completeTagList('同名山',mountains);assert.deepEqual(multiple.issues,[]);assert.deepEqual(multiple.mappings,[{osmId:'2',name:'同名山'},{osmId:'3',name:'同名山'}]);assert.equal(multiple.reviews[0].includedCount,2);assert.ok(completeTagList('同名山\n2',mountains).issues.length);
const review=completeTagList('同名山\n1,違う山\n999',mountains).reviews;assert.equal(review.length,3);assert.deepEqual(review[0].candidates.map(p=>p.osmId),['2','3']);assert.equal(review[1].osmId,'1');assert.equal(review[1].name,'違う山');assert.equal(review[1].candidates[0].pointName,'羊蹄山');assert.equal(review[2].osmId,'999');assert.equal(review[2].candidates.length,0);
assert.throws(()=>completeTagList('',mountains));assert.throws(()=>completeTagList('name',mountains));
assert.equal(mappingListText([{osmId:'1',name:'山,"一'}]),'#osmId,name\n1,"山,""一"\n');
assert.equal(validTagFilename('日本百名山'),true);for(const tag of ['', '分類/', 'CON',' 山'])assert.equal(validTagFilename(tag),false);
const supplemental=parseTagJson(JSON.stringify({source:'確認資料',points:[{osmId:1,note:'山',nameReading:'やま',aliases:['別名'] }]}),'補足.json');
assert.equal(supplemental.addTag,false);const supplementBase=[{osmId:'1',pointName:'山',tags:[],source:{id:'1',name:'山',nameReading:'もと',aliases:['旧別名']}}];
const supplemented=applyTagTables(supplementBase,new Map([['補足',supplemental]])).entries[0];assert.deepEqual(supplemented.tags,[]);assert.equal(supplemented.nameReading,'やま');assert.deepEqual(supplemented.aliases,['旧別名','別名']);assert.equal(supplementBase[0].source.nameReading,'もと');assert.deepEqual(supplementBase[0].source.aliases,['旧別名']);
const roundtrip=JSON.parse(tagJsonText('補足',supplemental.mappings,supplemental.metadata,false));assert.equal(Object.hasOwn(roundtrip,'tag'),false);assert.equal(roundtrip.points[0].nameReading,'やま');assert.deepEqual(roundtrip.points[0].aliases,['別名']);
const conflicting=parseTagJson(JSON.stringify({tag:'競合',points:[{osmId:1,note:'山',nameReading:'さん'}]}),'競合.json');assert.throws(()=>applyTagTables(supplementBase,new Map([['補足',supplemental],['競合',conflicting]])),/競合/);
for(const extra of [{nameReading:''},{nameReading:null},{aliases:['同じ','同じ']},{aliases:[null]}])assert.throws(()=>parseTagJson(JSON.stringify({tag:'分類',points:[{osmId:1,note:'山',...extra}]}),'分類.json'));
assert.throws(()=>parseTagJson(JSON.stringify({points:[{osmId:1,note:'山'}]}),'補足.json'));
tables.set('不明',table(999,'不明','不明'));assert.throws(()=>applyTagTables(base,tables));
'''
        result = subprocess.run([shutil.which("node"), "-e", source + checks], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", errors="replace"))

    @unittest.skipUnless(shutil.which("node"), "Node.js がないため保存処理のテストを実行できません")
    def test_save_picker_cancel_failure_and_fallback(self):
        viewer = Path(__file__).resolve().parents[1] / "viewer.html"
        page = viewer.read_text(encoding="utf-8")
        source = "let saving=false;" + page.split("let saving=false;", 1)[1].split("function showSaveResult", 1)[0]
        checks = r'''
const assert=require('node:assert/strict');const status={textContent:''};let clicked=false;
globalThis.document={getElementById:()=>status,body:{append:()=>{}},createElement:()=>({click:()=>clicked=true,remove:()=>{}})};
globalThis.window={};
(async()=>{
let written,closed=false,options;
window.showSaveFilePicker=async value=>{options=value;return{createWritable:async()=>({write:async blob=>written=await blob.text(),close:async()=>closed=true})};};
assert.equal(await saveFile('分類.json','{}\n','application/json;charset=utf-8'),'saved');assert.equal(options.suggestedName,'分類.json');assert.deepEqual(options.types[0].accept,{'application/json':['.json']});assert.equal(written,'{}\n');assert.equal(closed,true);
window.showSaveFilePicker=async()=>{throw new DOMException('cancel','AbortError');};assert.equal(await saveFile('x.json','{}\n','application/json'),null);assert.match(status.textContent,/キャンセル/);assert.equal(clicked,false);
let aborted=false;window.showSaveFilePicker=async()=>({createWritable:async()=>({write:async()=>{throw Error('失敗');},abort:async()=>aborted=true})});assert.equal(await saveFile('x.json','{}\n','application/json'),null);assert.equal(aborted,true);assert.match(status.textContent,/失敗/);assert.equal(saving,false);
window.showSaveFilePicker=undefined;assert.equal(await saveFile('x.json','{}\n','application/json'),'download');assert.equal(clicked,true);
})().catch(error=>{console.error(error);process.exitCode=1;});
'''
        result = subprocess.run([shutil.which("node"), "-e", source + checks], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", errors="replace"))

    @unittest.skipUnless(shutil.which("node"), "Node.js がないため未保存状態のテストを実行できません")
    def test_unsaved_tag_tracking(self):
        viewer = Path(__file__).resolve().parents[1] / "viewer.html"
        page = viewer.read_text(encoding="utf-8")
        source = "function tagState(" + page.split("function tagState(", 1)[1].split("function refreshExportTags", 1)[0]
        setup = """
const assert=require('node:assert/strict');const savedTagStates=new Map(),dirtyTags=new Set();
let points=[{source:{id:'1'},osmId:'1',pointName:'山',tags:['既存']}];let beforeUnload;
const tagTables=new Map();
const document={title:'地点の確認'};let pageTitle=document.title;
const tagFilter={options:[{value:'all',textContent:'すべて'},{value:'tag:分類A'},{value:'tag:分類A'}]};
const window={addEventListener:(name,handler)=>beforeUnload=handler};function refreshExportTags(){}
"""
        checks = """
resetSavedTags();assert.equal(dirtyTags.size,0);points[0].tags.push('分類A','分類B');updateDirtyTags();assert.equal(dirtyTags.size,2);assert.equal(document.title,'（未保存）地点の確認');assert.equal(tagDisplayName('分類B'),'分類B（未保存）');assert.equal(tagFilter.options[1].textContent,'分類A（未保存）');assert.equal(tagFilter.options[2].textContent,'分類A（未保存）');assert.equal(tagFilter.options[0].textContent,'すべて');
let prevented=false;const event={preventDefault:()=>prevented=true};beforeUnload(event);assert.equal(prevented,true);assert.equal(event.returnValue,'');
savedTagStates.set('分類A',tagState('分類A'));updateDirtyTags();assert.deepEqual([...dirtyTags],['分類B']);assert.equal(tagDisplayName('分類A'),'分類A');assert.equal(tagFilter.options[2].textContent,'分類A');
points[0].tags=points[0].tags.filter(tag=>tag!=='分類B');updateDirtyTags();assert.equal(dirtyTags.size,0);
points[0].tags=points[0].tags.filter(tag=>tag!=='分類A');updateDirtyTags();assert.equal(dirtyTags.size,0);
savedTagStates.set('分類A',tagState('分類A'));updateDirtyTags();assert.equal(dirtyTags.size,0);
assert.equal(document.title,'地点の確認');
tagTables.set('既存',{metadata:{source:'出典'}});updateDirtyTags();assert.deepEqual([...dirtyTags],['既存']);savedTagStates.set('既存',tagState('既存'));updateDirtyTags();assert.equal(dirtyTags.size,0);
prevented=false;beforeUnload({preventDefault:()=>prevented=true});assert.equal(prevented,false);
"""
        result = subprocess.run([shutil.which("node"), "-e", setup + source + checks], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", errors="replace"))

    @unittest.skipUnless(shutil.which("node"), "Node.js がないためタグ索引のテストを実行できません")
    def test_filter_index_merges_tag_origins(self):
        page = (Path(__file__).resolve().parents[1] / "viewer.html").read_text(encoding="utf-8")
        source = "const tableIdSets=" + page.split("const tableIdSets=", 1)[1].split("function renderView", 1)[0]
        checks = """
const assert=require('node:assert/strict');const tagFilterIndex=new Map();
const basePoints=[{tags:['分類']},{tags:[]},{tags:[]}];
const points=[{osmId:'1',tags:['分類']},{osmId:'2',tags:['分類']},{osmId:'3',tags:[]}];
const tagTables=new Map([['分類',{mappings:[{osmId:'2'}]}]]),tagEdits=new Map();
assert.deepEqual(tagFilterOptions(),[['tag:分類','分類']]);
assert.deepEqual([...tagFilterIndex.get('tag:分類')],[0,1]);assert.deepEqual([...tagFilterIndex.get('supplemented')],[]);points[2].supplement={nameReading:'やま'};tagFilterOptions();assert.deepEqual([...tagFilterIndex.get('supplemented')],[2]);assert.equal(matchesTagFilter(2,'supplemented'),true);assert.equal(matchesTagFilter(0,'supplemented'),false);assert.deepEqual([...tagFilterIndex.get('untagged')],[2]);
points[1].tags=[];tagFilterOptions();assert.deepEqual([...tagFilterIndex.get('tag:分類')],[0]);
points[2].tags=['分類'];tagEdits.set(2,{add:new Set(['分類'])});tagFilterOptions();assert.deepEqual([...tagFilterIndex.get('tag:分類')],[0,2]);points[0].tags=[];points[2].tags=[];assert.deepEqual(tagFilterOptions(),[]);assert.equal(tagFilterIndex.has('tag:分類'),false);
"""
        result = subprocess.run([shutil.which("node"), "-e", source + checks], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", errors="replace"))

"""補足1地点の保存で、全地点の適用・描画を繰り返さないことを確認する。"""
from pathlib import Path
import shutil
import subprocess
import unittest


class ViewerSaveTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_save_updates_one_point_and_keeps_filters_and_pending_tags(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        validation="function validateAliasReadings("+page.split("function validateAliasReadings(",1)[1].split("function parseSupplement(",1)[0]
        application="function applySupplement("+page.split("function applySupplement(",1)[1].split("function supplementTables(",1)[0]
        incremental="function refreshEditedSupplement("+page.split("function refreshEditedSupplement(",1)[1].split("const tagFilterIndex=",1)[0]
        names=("mapPointName","readingText","validateRelatedIds","supplementKey","pointReferenceKeys","supplementMatches","supplementForPoint","supplementTables","selectedFilterValues","indexedTagFilterOptions")
        functions="\n".join(next(line for line in page.splitlines() if line.startswith("function "+name+"(")) for name in names)
        checks=r"""
const assert=require('node:assert/strict');
function recordSupplementEdit(){}
const raw=Array.from({length:100000},(_,i)=>({source:{id:'gsi-vector:'+i,name:'山'+i,nameReading:'やま',tags:i===4||i===9?['A']:[]},pointName:'山'+i,tags:i===4||i===9?['A']:[]}));
let reads=0;
function guarded(items){return new Proxy(items,{get(target,key){if(['map','forEach','filter','flatMap','slice',Symbol.iterator].includes(key))throw Error('全地点の再走査');if(typeof key==='string'&&/^\d+$/.test(key))reads++;return target[key];}});}
const basePoints=guarded(raw.slice());const current=raw.slice();current[9]={...raw[9],tags:['B']};const unchanged=current[9];let points=guarded(current);
let supplementDocument={schemaFormat:'yamamuki-point-supplements',schemaVersion:2,points:[]};let tagEdits=new Map([[9,{add:new Set(['B']),remove:new Set(['A'])}]]),tagTables=new Map();
const tagFilterIndex=new Map([['untagged',new Set()],['supplemented',new Set([9])],['tag:A',new Set([4])],['tag:B',new Set([9])]]);
const dirtyTags=new Set(),savedTagStates=new Map();const sortKeys=new Map([[5,new Map([[4,'やま'],[9,'やま']])]]);
const marker={setTooltipContent(value){this.title=value;},getPopup(){return null;}};const markers=new Map([[4,marker]]);
const scroll={scrollTop:12345},search={value:'検索中'};
const tagFilter={options:[{value:'all',selected:false},{value:'tag:A',selected:true},{value:'tag:B',selected:true}],get selectedOptions(){return this.options.filter(o=>o.selected);},replaceChildren(){this.options=[];},append(option){this.options.push(option);}};
const provenanceFilters=[{value:'vector'},{value:'810'},{value:'dem1a_png'}];
const document={querySelector(){return scroll;},createElement(){return {};}};
const rows=[];let replacements=0,bindings=0;rows[4]={hidden:true,replaceWith(row){replacements++;}};
function makeTable(entries){assert.equal(entries.length,1);assert.equal(entries[0].pointName,'補正した山');return {firstChild:{replaceWith(){replacements++;}}};}
function bindLargeRow(row,index){assert.equal(index,4);bindings++;}
function tagState(tag,members){return JSON.stringify(members.map(p=>[p.source.id,p.pointName,p.tags]));}
function refreshFilterTagLabels(){}function updatePageTitle(){}function refreshExportTags(){}
let finish;let largeMode=true,largeFilterRun=0;let selected=null;function filter(){largeFilterRun++;assert.deepEqual(selectedFilterValues(tagFilter),['tag:B']);scroll.scrollTop=0;return new Promise(resolve=>finish=resolve);}
function rowVisible(index){return true;}function selectRow(index){selected=index;}function closeJson(){selected=null;}
(async()=>{
 const data={schemaFormat:'yamamuki-point-supplements',schemaVersion:2,points:[{id:'gsi-vector:4',name:'補正した山',nameReading:'ほせい',tags:['B'],tagsRemove:['A']},{id:'gsi-vector:9',tags:['B'],tagsRemove:['A']}]};
 const pending=refreshEditedSupplement(4,data);
 assert.equal(replacements,1);assert.equal(bindings,1);assert.equal(points[4].nameReading,'ほせい');assert.deepEqual(points[4].tags,['B']);assert.equal(points[9],unchanged);assert.equal(supplementDocument.points.length,2);assert.equal(tagEdits.size,0);
 assert.equal(tagFilterIndex.has('tag:A'),false);assert.deepEqual([...tagFilterIndex.get('tag:B')],[9,4]);assert.deepEqual([...tagFilterIndex.get('supplemented')],[9,4]);assert.equal(rows[4].hidden,true);assert.equal(marker.title,'補正した山');assert.equal(sortKeys.get(5).has(4),false);assert.equal(sortKeys.get(5).get(9),'やま');
 finish();await pending;assert.equal(scroll.scrollTop,12345);assert.equal(selected,4);assert.equal(search.value,'検索中');assert.deepEqual(provenanceFilters.map(p=>p.value),['vector','810','dem1a_png']);assert.ok(reads<20);
 const before=points[4];assert.throws(()=>refreshEditedSupplement(4,{...data,points:[{id:'gsi-vector:4',nameReading:''}]}));assert.equal(points[4],before);
 selected=null;const superseded=refreshEditedSupplement(4,data);largeFilterRun++;finish();await superseded;assert.equal(selected,null);
 scroll.scrollTop=12345;largeMode=false;filter=()=>{scroll.scrollTop=0;};await refreshEditedSupplement(4,data);assert.equal(selected,4);assert.equal(scroll.scrollTop,12345);
 rows[4]=undefined;const painted=replacements;await refreshEditedSupplement(4,data);assert.equal(replacements,painted);
 reads=0;let filters=0;filter=()=>{filters++;};await refreshEditedSupplement([4,9],data,false);assert.equal(filters,1);assert.ok(reads<30);assert.equal(replacements,painted);assert.deepEqual(points[9].tags,['B']);
})().catch(error=>{console.error(error);process.exitCode=1;});
"""
        result=subprocess.run(["node","-"],input=validation+application+functions+incremental+checks,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_file_save_keeps_filters_and_uses_incremental_refresh(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        code=next(line for line in page.splitlines() if line.startswith("async function saveSupplementFile("))
        checks=r"""
const assert=require('node:assert/strict');const data={points:[{id:'gsi:1',tags:['A']}]};
const basePoints=[],tagEdits=new Map([[4,{}]]);let supplementDocument={},savedSupplementState='old',refreshed=0,closed=0,saveResult='saved';
const fields={'supplement-sources':{value:'[]'},'supplement-tag-metadata':{value:'[]'},'tag-json-save-dialog':{close(){closed++;}},'tag-json-save-status':{textContent:''}};
const document={getElementById(id){return fields[id];}};
const filters={search:'山',tags:['A','B'],sources:['vector'],hidden:true,scroll:345};
function supplementWithOriginalNames(data){return data;}function exportSupplement(){return {...structuredClone(data),sources:{}};}function metadataMaps(){return {sources:{},tags:{}};}function validateTagMetadata(value){return value;}function validateSupplement(){}function applySupplement(){}function supplementJson(value){return value;}
async function saveFile(){return saveResult;}function recordSupplementEdit(){throw Error('内容が同じ保存では履歴を増やさない');}function showSaveResult(){}function resetSavedTags(){}
async function refreshEditedSupplement(indices,value,record){assert.deepEqual(indices,[4]);assert.equal(record,false);refreshed++;supplementDocument=value;tagEdits.clear();}
function refreshSupplement(){throw Error('全体描画では絞り込みが解除される');}
(async()=>{await saveSupplementFile();assert.equal(refreshed,1);assert.equal(closed,1);assert.equal(savedSupplementState,JSON.stringify({...data,sources:{}}));assert.deepEqual(filters,{search:'山',tags:['A','B'],sources:['vector'],hidden:true,scroll:345});saveResult=null;await saveSupplementFile(true);assert.equal(refreshed,1);assert.equal(closed,1);})().catch(error=>{console.error(error);process.exitCode=1;});
"""
        result=subprocess.run(["node","-"],input=code+checks,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

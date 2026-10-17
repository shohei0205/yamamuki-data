"""地点リストの名前と読みを、元データとの差分を保って描画する。"""
import shutil
import subprocess
from pathlib import Path
import unittest


class ViewerRenderTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_point_reading_renders_and_compares_against_original(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("readingText","aliasEntries","aliasNames","mergedAliasEntries","aliasDisplayText","makeTable")
        functions="\n".join(next(line for line in page.splitlines() if line.startswith("function "+name+"(")) for name in names)
        checks=r"""
const assert=require('node:assert/strict');
function recordSupplementEdit(){}
function element(tag){return {tag,children:[],classList:{add(){}},textContent:'',append(...items){this.children.push(...items);},prepend(item){this.children.unshift(item);},addEventListener(){},setAttribute(){}};}
const document={createElement:element,createDocumentFragment:()=>element('fragment')};
const comparisons=[];
function markSupplementCell(cell,current,original){comparisons.push([current,original]);}
function locationLabel(){return '';}
function removeFilteredTag(){}
function updateTagCell(){}
function appendGraphicCells(){}
function updateRowSearch(){}
function appendPointActions(){}
const source={id:'gsi-mountain-list:1',name:'知床岳',nameReading:'しれとこだけ',aliases:[{name:'別名',nameReading:'べつめい'}]};
const base={source,pointName:source.name,latitude:44,longitude:145,elevation:1254,aliases:['別名'],aliasReadings:{'別名':'べつめい'},tags:[]};
let row=makeTable([base]).children[0];
assert.equal(row.children[5].textContent,'しれとこだけ');
assert.equal(row.children[6].textContent,'別名（べつめい）');
assert.deepEqual(comparisons[1],['しれとこだけ','しれとこだけ']);
comparisons.length=0;
row=makeTable([{...base,nameReading:'ほせいしたよみ'}]).children[0];
assert.equal(row.children[5].textContent,'ほせいしたよみ');
assert.deepEqual(comparisons[1],['ほせいしたよみ','しれとこだけ']);
comparisons.length=0;row=makeTable([{...base,source:{...source,aliases:[{name:'別名'}]},aliasReadings:{'別名':['べつめい','べつのよみ']}}]).children[0];assert.deepEqual(comparisons[2],['別名（べつめい、べつのよみ）','別名']);
row=makeTable([{...base,nameReading:null}]).children[0];assert.equal(row.children[5].textContent,'');
row=makeTable([{...base,nameReading:['しろうまだけ','はくばだけ'],aliasReadings:{'別名':['よみ1','よみ2']}}]).children[0];assert.equal(row.children[5].textContent,'しろうまだけ、はくばだけ');assert.equal(row.children[6].textContent,'別名（よみ1、よみ2）');
"""
        result=subprocess.run(["node","-"],input=functions+checks,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_detail_shows_current_supplement_including_unsaved_edits(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        code="\n".join(next(line for line in page.splitlines() if line.startswith("function "+name+"(")) for name in ("aliasEntries","mergedAliasEntries","mergedPointJson","showSupplementOrigin"))
        checks=r"""
const assert=require('node:assert/strict');
function recordSupplementEdit(){}
const fields={};const document={getElementById(id){return fields[id]??={textContent:''};}};
const tagFilterIndex=new Map();tagFilterIndex.exclusionRelations={relations:new Map([[0,['日本百名山']]])};
const points=[{source:{id:'gsi-vector:example',name:'元の山',nameReading:'もと',latitude:35,longitude:140,provenance:{dataSource:'vector'}},pointName:'変更後の山',nameReading:null,aliases:['別名'],aliasReadings:{'別名':'べつめい'},tags:['未保存のタグ'],relatedIds:['osm:123'],graphic:null}];
let row={id:'gsi-vector:example',name:'変更後の山',aliases:[{name:'元の山',nameReading:'もとのやま'}],tags:['未保存のタグ'],exclude:true};
function exportSupplement(){return {points:row?[row]:[]};}
function supplementForPoint(data,point){return data.points.find(row=>row.id===point.source.id);}
function supplementChangeLines(){return ['補足の説明'];}
showSupplementOrigin(0);
assert.deepEqual(JSON.parse(fields['point-supplement-json'].textContent),row);
assert.deepEqual(JSON.parse(fields['point-merged-json'].textContent),{id:'gsi-vector:example',name:'変更後の山',latitude:35,longitude:140,provenance:{dataSource:'vector'},aliases:[{name:'別名',nameReading:'べつめい'}],tags:['未保存のタグ'],relatedIds:['osm:123']});
assert.equal(points[0].source.name,'元の山');
assert.equal(fields['point-excluded-message'].hidden,true);
points[0].excluded=true;showSupplementOrigin(0);
assert.equal(fields['point-merged-json'].textContent,'');
assert.equal(fields['point-excluded-message'].hidden,false);
assert.equal(fields['point-exclusion-tags'].hidden,false);assert.ok(fields['point-exclusion-tags'].textContent.includes('日本百名山'));assert.ok(fields['point-exclusion-tags'].textContent.includes('推定'));
assert.deepEqual(JSON.parse(fields['point-supplement-json'].textContent),row);
assert.equal(fields['point-supplement-json'].textContent,JSON.stringify(row,null,2));
row=undefined;showSupplementOrigin(0);
assert.equal(fields['point-supplement-json'].textContent,'補足なし');
"""
        result=subprocess.run(["node","-e",code+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode("utf-8",errors="replace"))

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_remove_tag_from_gsi_point_without_osm_id(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        code=next(line for line in page.splitlines() if line.startswith("function pointReference("))
        code+="\nfunction removeFilteredTag("+page.split("function removeFilteredTag(",1)[1].split("function editSupplementTag(",1)[0]
        checks=r"""
const assert=require('node:assert/strict');
function recordSupplementEdit(){}
const point={source:{id:'gsi-mountain-list:100'},osmId:'',pointName:'羊蹄山',tags:['日本百名山']};
const points=[point],basePoints=[point],tagEdits=new Map(),tagTables=new Map();
const tagFilter={value:'tag:日本百名山',options:[{value:'tag:日本百名山'}]};
const fields={search:{value:'羊蹄'},'save-status':{textContent:''}},wrap={scrollTop:123};
const document={getElementById(id){return fields[id];},querySelector(){return wrap;}};
function showToast(message){throw Error(message);}
function applyTagTables(){return {entries:basePoints};}
let rendered=false;function renderView(){rendered=true;}function filter(){}
removeFilteredTag(point);
assert.equal(rendered,true);assert.ok(tagEdits.get(0).remove.has('日本百名山'));
assert.equal(fields.search.value,'羊蹄');assert.equal(wrap.scrollTop,123);
assert.match(fields['save-status'].textContent,/補足 JSON/);
"""
        result=subprocess.run(["node","-e",code+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode("utf-8",errors="replace"))

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_context_menu_survives_row_number_refresh(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        code=next(line for line in page.splitlines() if line.startswith("function bindPointContextMenu("))
        checks=r"""
const assert=require('node:assert/strict');let menu;
const document={createElement(){return menu={listeners:{},offsetWidth:200,offsetHeight:80,setAttribute(){},addEventListener(type,fn){this.listeners[type]=fn;},append(){},matches(){return false;},showPopover(){this.open=true;},style:{}};}};
const window={innerWidth:800,innerHeight:600};const getComputedStyle=()=>({display:'block'});
const numberCell={set textContent(value){this.children=[];}},nameCell={append(value){this.menu=value;}};
const row={children:[numberCell,nameCell],querySelector(selector){return selector==='.point-link'?{parentElement:nameCell}:null;}};
bindPointContextMenu(row,[{disabled:false,focus(){}}]);numberCell.textContent='100';
row.oncontextmenu({preventDefault(){},stopPropagation(){},clientX:790,clientY:590});
assert.equal(nameCell.menu,menu);assert.equal(menu.open,true);assert.equal(menu.style.left,'592px');assert.equal(menu.style.top,'512px');
"""
        result=subprocess.run(["node","-e",code+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode("utf-8",errors="replace"))

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_multi_selection_toggle_and_range_follow_display_order(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        code=next(line for line in page.splitlines() if line.startswith("function listSelection("))
        checks=r"""
const assert=require('node:assert/strict');const order=[8,2,5,1,9];
let result=listSelection(new Set(),null,2,order,{});assert.deepEqual([...result.values],[2]);
result=listSelection(result.values,result.anchor,9,order,{ctrlKey:true});assert.deepEqual([...result.values],[2,9]);
result=listSelection(result.values,result.anchor,2,order,{metaKey:true});assert.deepEqual([...result.values],[9]);
result=listSelection(result.values,2,1,order,{shiftKey:true});assert.deepEqual([...result.values],[2,5,1]);assert.equal(result.anchor,2);
result=listSelection(new Set([9]),1,8,order,{shiftKey:true,ctrlKey:true});assert.deepEqual([...result.values],[9,8,2,5,1]);
result=listSelection(result.values,result.anchor,5,order,{});assert.deepEqual([...result.values],[5]);
result=listSelection(result.values,99,1,order,{shiftKey:true});assert.deepEqual([...result.values],[1]);
"""
        result=subprocess.run(["node","-e",code+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode("utf-8",errors="replace"))

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_selected_point_export_preserves_metadata_and_omits_excluded(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("metadataMaps","metadataArrays","pointReference","aliasEntries","mergedAliasEntries","mergedPointJson","selectedPointDocument","selectedPointJsonText","saveSelectedPoints")
        code="\n".join(next(line for line in page.splitlines() if line.startswith(("function "+name+"(","async function "+name+"("))) for name in names)
        checks=r"""
const assert=require('node:assert/strict');
const points=[{source:{id:'gsi-vector:1',name:'元の名前',latitude:35,longitude:139},pointName:'白馬岳',nameReading:['しろうまだけ','はくばだけ'],tags:['分類']},{source:{id:'gsi-vector:2',name:'除外山'},excluded:true},{source:{id:'123',osmId:123,name:'旧地点',latitude:36,longitude:140},osmId:'123',pointName:'旧地点',tags:[]}];
const basePoints=[...points];basePoints.metadata={schemaVersion:6,sources:{国土地理院:{license:'PDL-1.0'}},pointCount:3};
function exportSupplement(){return {sources:{補足:{source:'資料'}},tags:{分類:{nameReading:['ぶんるい1','ぶんるい2']}},points:[]};}
const original=JSON.stringify(points);const body=selectedPointDocument([0,1,2]);assert.equal(body.metadata.pointCount,2);assert.equal(body.metadata.sources.length,2);assert.equal(body.metadata.tags[0].nameReading.length,2);assert.equal(body.metadata.schemaVersion,undefined);assert.equal(body.points[0].name,'白馬岳');assert.deepEqual(body.points[0].nameReading,['しろうまだけ','はくばだけ']);assert.equal(body.points[1].id,'osm:123');assert.equal(body.points[1].osmId,undefined);assert.equal(JSON.stringify(points),original);assert.deepEqual(JSON.parse(selectedPointJsonText(body)),body);assert.throws(()=>selectedPointDocument([1]),/すべて/);
const selectedIndices=new Set([0,1,2]);let saved;
async function saveFile(name,text){saved=JSON.parse(text);return 'saved';}function showSaveResult(){}function showToast(message){throw Error(message);}
(async()=>{await saveSelectedPoints(points[0]);assert.equal(saved.points.length,2);selectedIndices.clear();selectedIndices.add(0);await saveSelectedPoints(points[2]);assert.equal(saved.points.length,1);assert.equal(saved.points[0].id,'osm:123');})().catch(error=>{console.error(error);process.exitCode=1;});
"""
        result=subprocess.run(["node","-"],input=code+checks,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_detail_title_uses_effective_readings(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        code="\n".join(next(line for line in page.splitlines() if line.startswith("function "+name+"(")) for name in ("readingText","pointDetailTitle"))
        checks=r"""
const assert=require('node:assert/strict');
const point={pointName:'越後駒ヶ岳',source:{nameReading:'えちごこまがたけ'}};
assert.equal(pointDetailTitle(point),'越後駒ヶ岳 (えちごこまがたけ) の詳細');
assert.equal(pointDetailTitle({...point,nameReading:['よみ1','よみ2']}),'越後駒ヶ岳 (よみ1、よみ2) の詳細');
assert.equal(pointDetailTitle({...point,nameReading:null}),'越後駒ヶ岳 の詳細');
assert.equal(pointDetailTitle({pointName:'山',source:{}}),'山 の詳細');
"""
        result=subprocess.run(["node","-e",code+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode("utf-8",errors="replace"))

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_alias_reading_change_marks_cell_bold_and_shows_original(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("readingText","aliasEntries","mergedAliasEntries","aliasDisplayText","markSupplementCell")
        code="\n".join(next(line for line in page.splitlines() if line.startswith("function "+name+"(")) for name in names)
        checks=r"""
const assert=require('node:assert/strict');
function cell(text){const classes=new Set();return {textContent:text,classList:{add(value){classes.add(value);},contains(value){return classes.has(value);}}};}
const source={aliases:[{name:'別名'}]},point={source,aliases:['別名'],aliasReadings:{'別名':['べつめい','べつのよみ']}};
const current=aliasDisplayText(point),original=aliasDisplayText({source});let target=cell(current);markSupplementCell(target,current,original);assert.equal(target.classList.contains('supplement-modified'),true);assert.equal(target.title,'補足 JSON による変更（元の値: 別名）');
target=cell(original);markSupplementCell(target,original,original);assert.equal(target.classList.contains('supplement-modified'),false);
const withReading={aliases:[{name:'別名',nameReading:'もとのよみ'}]};target=cell('別名');markSupplementCell(target,'別名',aliasDisplayText({source:withReading}));assert.equal(target.classList.contains('supplement-modified'),true);assert.match(target.title,/もとのよみ/);
"""
        result=subprocess.run(["node","-"],input=code+checks,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_copy_selected_points_matches_export_selection(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        code=next(line for line in page.splitlines() if line.startswith("async function copySelectedPoints("))
        checks=r"""
const assert=require('node:assert/strict');const points=[{source:{}},{source:{}}],selectedIndices=new Set([0,1]);let copied,indices,message;
function selectedPointDocument(values){indices=values;return {schemaFormat:'yamamuki-points',schemaVersion:6,points:values.map(id=>({id}))};}
function selectedPointJsonText(value){return JSON.stringify(value);}async function copyText(text){copied=JSON.parse(text);}function showToast(text){message=text;}
(async()=>{await copySelectedPoints(points[0]);assert.deepEqual(indices,[0,1]);assert.equal(copied.points.length,2);assert.match(message,/2 件.*コピーしました/);selectedIndices.delete(1);await copySelectedPoints(points[1]);assert.deepEqual(indices,[1]);copyText=async()=>{throw Error('拒否');};await copySelectedPoints(points[0]);assert.match(message,/コピーできませんでした/);})().catch(error=>{console.error(error);process.exitCode=1;});
"""
        result=subprocess.run(["node","-"],input=code+checks,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_supplement_save_notes_are_original_names_for_all_rows(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("supplementKey","pointReferenceKeys","supplementWithOriginalNames")
        code="\n".join(next(line for line in page.splitlines() if line.startswith("function "+name+"(")) for name in names)
        checks=r"""
const assert=require('node:assert/strict');
const entries=[{source:{id:'gsi:1',name:'東岳'},pointName:'悪沢岳'},{source:{id:'gsi:2',name:'別の山'}},{source:{id:'osm:3',name:'元のOSM山'},osmId:'3'}];
const data={schemaVersion:2,points:[{id:'gsi:1',name:'悪沢岳',note:'旧メモ'},{id:'gsi:2',exclude:true},{id:'osm:3',tags:['分類']}]};const before=JSON.stringify(data);
const result=supplementWithOriginalNames(data,entries);assert.deepEqual(result.points.map(row=>row.note),['東岳','別の山','元のOSM山']);assert.equal(JSON.stringify(data),before);assert.equal(result.points[0].name,'悪沢岳');assert.equal(result.points[1].exclude,true);
assert.throws(()=>supplementWithOriginalNames({points:[{id:'unknown'}]},entries),/確認できない/);
assert.deepEqual(supplementWithOriginalNames({points:[]},[]),{points:[]});
"""
        result=subprocess.run(["node","-"],input=code+checks,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertNotIn('id="supplement-note"',page)

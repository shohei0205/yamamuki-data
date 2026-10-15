"""地点リストの名前と読みを、元データとの差分を保って描画する。"""
import shutil
import subprocess
from pathlib import Path
import unittest


class ViewerRenderTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_point_reading_renders_and_compares_against_original(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("aliasEntries","aliasNames","makeTable")
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
row=makeTable([{...base,nameReading:null}]).children[0];assert.equal(row.children[5].textContent,'');
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

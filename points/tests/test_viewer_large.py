"""大量地点のページ分割、マーカー上限、別スレッドの読み込みを確認する。"""
from pathlib import Path
import shutil
import subprocess
import unittest

class LargeViewerTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('node'), 'Node.js が必要です')
    def test_paging_marker_budget_and_file_loading(self):
        page=(Path(__file__).resolve().parents[1]/'viewer.html').read_text(encoding='utf-8')
        provenance=page.split('function provenanceOf(',1)[1].split('const LARGE_POINT_LIMIT=',1)[0]
        provenance='function provenanceOf('+provenance
        helpers=page.split('const LARGE_POINT_LIMIT=',1)[1].split('let points=[];',1)[0]
        labels='const sourceLabels='+page.split('const sourceLabels=',1)[1].split('function refreshProvenanceFilters()',1)[0]
        parser='function entriesFromJson('+page.split('function entriesFromJson(',1)[1].split('const fileInput=',1)[0]
        checks=r"""
const assert=require('node:assert/strict'),vm=require('node:vm'),zlib=require('node:zlib');
let terminated=0;
globalThis.Worker=class{
 constructor(url){this.url=url;}
 async postMessage(file){try{const code=await(await fetch(this.url)).text();const context={Response,DecompressionStream,Uint8Array,postMessage:data=>this.onmessage({data})};vm.createContext(context);vm.runInContext(code,context);await context.onmessage({data:file});}catch(error){this.onerror(error);}}
 terminate(){terminated++;}
};
(async()=>{
 const order=Array.from({length:779673},(_,i)=>i);
 const first=virtualWindow(order.length,0,440);assert.equal(first.start,0);assert.ok(first.end<50);assert.equal(first.height,8000000);
 const middle=virtualWindow(order.length,4000000,440);assert.ok(middle.start>300000&&middle.end<500000);assert.ok(middle.end-middle.start<50);
 const last=virtualWindow(order.length,8000000-440,440);assert.equal(last.end,order.length);assert.ok(last.end-last.start<50);
 const empty=virtualWindow(0,0,440);assert.equal(empty.start,0);assert.equal(empty.end,0);
 const small=virtualWindow(5,0,440);assert.equal(small.end,5);
 const limited=boundedMapIndices(order,779672);assert.equal(limited.length,10000);assert.equal(new Set(limited).size,10000);assert.ok(limited.includes(779672));
 assert.deepEqual(boundedMapIndices([1,2],null),[1,2]);
 const meta={dataSource:'vector',classification:'661',elevationSource:'dem5a_png'};
 const point={source:{id:'1',provenance:meta},elevation:870.12};
 assert.deepEqual(pointProvenanceDetails(point),[{label:'取得元',value:'国土地理院：ベクトルタイル'},{label:'注記の分類',value:'神社（661）'},{label:'標高の出典',value:'DEM5A'}]);
 assert.equal(provenanceLabel('9999','classification'),'注記コード 9999');
 assert.equal(pointProvenanceDetails({source:{provenance:{}}})[1].value,'不明');
 assert.equal(provenanceLabel('unassigned','elevationSource'),'未補完');
 assert.equal(matchesProvenance(point,['vector','661','dem5a_png']),true);
 assert.equal(matchesProvenance(point,['vector','662','dem5a_png']),false);
 assert.equal(matchesProvenance(point,['all','661','all']),true);
 assert.equal(matchesProvenance(point,['vector','661','dem1a_png']),false);
 for(const elevation of [0,-10,870.12])assert.equal(matchesProvenance({source:{provenance:{}},elevation},['all','all','@has-elevation']),true);
 for(const elevation of [null,undefined,NaN,Infinity,'100'])assert.equal(matchesProvenance({source:{provenance:{elevationSource:'osm'}},elevation},['all','all','@has-elevation']),false);
 assert.equal(matchesProvenance(point,['vector','661','@has-elevation']),true);
 assert.equal(matchesProvenance(point,['natural','661','@has-elevation']),false);
 assert.equal(provenanceOf({source:{id:'other'},elevation:null}).elevationSource,'unassigned');
 const raw=[{id:'1',name:'男体山御本殿',latitude:36.22567,longitude:140.09854,elevationM:null,type:null,provenance:meta}];
 const text=JSON.stringify(raw);
 for(const blob of [new Blob([text]),new Blob(['\uFEFF'+text]),new Blob([zlib.gzipSync(text)])]){
  const entries=await readPointFile(blob);assert.equal(entries.length,1);assert.equal(entries[0].elevation,null);assert.equal(entries[0].source.type,null);assert.equal(entries[0].source.provenance.classification,'661');assert.ok(pointSearchText(entries[0]).includes('男体山御本殿'));
 }
 await assert.rejects(()=>readPointFile(new Blob([JSON.stringify([raw[0],raw[0]])])),/重複/);
 await assert.rejects(()=>readPointFile(new Blob(['{}'])),/配列/);
 for(const partial of [{},{dataSource:'vector'},{classification:'661'},{elevationSource:'osm'}]){const entries=await readPointFile(new Blob([JSON.stringify([{...raw[0],provenance:partial}])]));const info=provenanceOf(entries[0]);assert.equal(info.dataSource,partial.dataSource??'unknown');assert.equal(info.classification,partial.classification??'unknown');assert.equal(info.elevationSource,partial.elevationSource??'unknown');}
 for(const invalid of [{dataSource:null},{dataSource:''},{dataSource:' osm '},{extra:'osm'}])await assert.rejects(()=>readPointFile(new Blob([JSON.stringify([{...raw[0],provenance:invalid}])])),/provenance/);
 assert.equal(terminated,13);
})().catch(error=>{console.error(error);process.exitCode=1;});
"""
        rendering=page.split('function renderLargeRows(){',1)[1].split('function refreshLargeMap(){',1)[0]
        rendering='function renderLargeRows(){'+rendering
        revealing='function revealRow('+page.split('function revealRow(',1)[1].split('function fitVisible()',1)[0]
        checks+=r"""
let largeMode=true,displayOrder=Array.from({length:779673},(_,i)=>i),rows=[],selectedIndex=null;
const points=displayOrder.map(i=>({pointName:String(i)}));
let created=0,rendered=[];
const table={style:{},replaceChildren(fragment){rendered=fragment.children;},setAttribute(){}};
const container={scrollTop:0,clientHeight:484,getBoundingClientRect(){return{top:0,bottom:484};},querySelector(){return{getBoundingClientRect(){return{height:44};}};}};
const document={createElement(){return{style:{},append(){},setAttribute(){}};},createDocumentFragment:()=>({children:[],append(row){this.children.push(row);}}),querySelector:selector=>selector==='.table-wrap'?container:table};
function makeTable(entries){created+=entries.length;return{firstChild:{point:entries[0],getBoundingClientRect(){return{top:-44,bottom:0};},classList:{toggle(){}},setAttribute(){},addEventListener(){}}};}
function bindLargeRow(){} function updatePrefectures(){}
renderLargeRows();assert.ok(created<50);assert.ok(rendered.length<52);const initial=created;
renderLargeRows();assert.equal(created,initial);
revealRow(779672);assert.ok(rows[779672]);assert.ok(container.scrollTop>7900000);
container.scrollTop=4000000;renderLargeRows();assert.ok(rendered.length<52);assert.ok(Object.keys(rows).every(i=>Number(i)>300000));
container.scrollTop=8000000-440;renderLargeRows();assert.ok(rendered.length<52);assert.equal(rows[779672].point.pointName,'779672');
const fixedHeight=table.style.height;assert.equal(fixedHeight,'8000044px');
for(let i=0;i<200;i++){container.scrollTop=i%2?8000000-440:0;renderLargeRows();assert.equal(table.style.height,fixedHeight);assert.equal(container.scrollTop,i%2?8000000-440:0);assert.ok(rendered.length<50);}
container.scrollTop=4000000;renderLargeRows();assert.equal(table.style.height,fixedHeight);assert.ok(table.style.transform.startsWith('translateY('));
"""
        result=subprocess.run([shutil.which('node'),'-e',provenance+labels+'const LARGE_POINT_LIMIT='+helpers+parser+rendering+revealing+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode('utf-8',errors='replace'))

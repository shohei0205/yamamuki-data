"""補足の取り消し・やり直しと保存状態を確認する。"""
from pathlib import Path
import shutil
import subprocess
import unittest

class HistoryTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_undo_redo_branch_limit_and_saved_state(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("updateHistoryButtons","recordSupplementEdit","resetEditHistory","restoreSupplementEdit","supplementDirty","changedSupplementIndices","supplementKey","pointReferenceKeys")
        code="\n".join(next(line for line in page.splitlines() if line.startswith(("function "+name+"(","async function "+name+"("))) for name in names)
        checks=r"""
const assert=require('node:assert/strict');
const undoEdits=[],redoEdits=[],basePoints=[{source:{id:'gsi-vector:1'}},{source:{id:'osm:1'}}];let historyPointIndex=null,historyBusy=false,tagEdits=new Map();
let supplementDocument={points:[]},savedSupplementState=JSON.stringify(supplementDocument);
let selectedIndex=3;const largeMode=false,wrap={scrollTop:123},fields={};
const document={getElementById(id){return fields[id]??={open:false};},querySelector(){return wrap;}};
function exportSupplement(){return supplementDocument;}
function validateSupplement(data){assert.ok(Array.isArray(data.points));}
function applySupplement(){}
async function refreshEditedSupplement(indices,data,record){assert.equal(record,false);supplementDocument=data;tagEdits=new Map();wrap.scrollTop=0;}
function rowVisible(){return true;}
function selectRow(index){selectedIndex=index;}
let nearbyRenders=0;function renderNearby(){nearbyRenders++;}function updateDirtyTags(){}function refreshExportTags(){}function showToast(){}
(async()=>{
assert.deepEqual(changedSupplementIndices({points:[]},{points:[{id:'gsi-vector:1',exclude:true}]}),[0]);
assert.deepEqual(changedSupplementIndices({points:[{id:'osm:1',tags:['A']}]},{points:[]}),[1]);
recordSupplementEdit();supplementDocument={points:[{id:'gsi-vector:1',exclude:true,aliases:[{name:'旧名'}],tags:['日本百名山']}]};
assert.equal(supplementDirty(),true);
await restoreSupplementEdit(true);
assert.deepEqual(supplementDocument,{points:[]});assert.equal(supplementDirty(),false);
assert.equal(selectedIndex,3);assert.equal(wrap.scrollTop,123);
document.getElementById('nearby-dialog').open=true;assert.equal(fields['redo-edit'].disabled,false);
await restoreSupplementEdit(false);assert.equal(supplementDocument.points[0].exclude,true);assert.equal(nearbyRenders,1);assert.equal(document.getElementById('nearby-dialog').open,true);
savedSupplementState=JSON.stringify(supplementDocument);
await restoreSupplementEdit(true);assert.equal(supplementDirty(),true);
await restoreSupplementEdit(false);assert.equal(supplementDirty(),false);
await restoreSupplementEdit(true);recordSupplementEdit();supplementDocument={points:[{id:'osm:1',tagsRemove:['A']}]};
assert.equal(redoEdits.length,0);await restoreSupplementEdit(false);assert.equal(supplementDocument.points[0].id,'osm:1');
for(let i=0;i<60;i++){recordSupplementEdit();supplementDocument={points:[{id:'osm:'+(i+2)}]};}
assert.equal(undoEdits.length,50);
resetEditHistory();assert.equal(undoEdits.length,0);assert.equal(redoEdits.length,0);assert.equal(fields['undo-edit'].disabled,true);
})().catch(error=>{console.error(error);process.exitCode=1;});
"""
        result=subprocess.run(["node","-"],input=code+checks,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_browser_back_forward_restores_selection_without_new_history(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("resetSelectionHistory","rememberSelection","restoreSelectionHistory")
        code="\n".join(next(line for line in page.splitlines() if line.startswith(("function "+name+"(","async function "+name+"("))) for name in names)
        checks=r"""
const assert=require('node:assert/strict');let selectionHistoryToken='old',restoringSelectionHistory=false;
const crypto={randomUUID:()=> 'new'};let pushes=0;
const history={state:{other:'preserved'},replaceState(state){this.state=state;},pushState(state){this.state=state;pushes++;}};
const points=[{},{}],tagFilter={options:[{value:'all',selected:false},{value:'tag:A',selected:true}]},provenanceFilters=[];
const fields={search:{value:'山'},'hide-excluded':{checked:true}};
const document={getElementById(id){return fields[id];}};
let selected=null,visible=false,revealed=null,filters=0;
function rowVisible(){return visible;}async function filter(){filters++;visible=true;}
function activatePoint(index){selected=index;rememberSelection(index);}function revealRow(index){revealed=index;}function closeJson(){selected=null;}
(async()=>{
resetSelectionHistory();const empty=history.state;
rememberSelection(0);const first=history.state;rememberSelection(0);assert.equal(pushes,1);
rememberSelection(1);const second=history.state;assert.equal(pushes,2);
await restoreSelectionHistory(first);assert.equal(selected,0);assert.equal(revealed,0);assert.equal(pushes,2);assert.equal(filters,1);assert.equal(fields.search.value,'');assert.equal(fields['hide-excluded'].checked,false);
await restoreSelectionHistory(second);assert.equal(selected,1);assert.equal(pushes,2);
await restoreSelectionHistory(empty);assert.equal(selected,null);
await restoreSelectionHistory({yamamukiSelection:{token:'old',index:0}});assert.equal(selected,null);
assert.equal(history.state.other,'preserved');
})().catch(error=>{console.error(error);process.exitCode=1;});
"""
        result=subprocess.run(["node","-"],input=code+checks,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

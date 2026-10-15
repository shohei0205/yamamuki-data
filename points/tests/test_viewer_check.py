"""選択地点の未入力チェックとポップアップ表示を確認する。"""
from pathlib import Path
import shutil
import subprocess
import unittest

class CheckTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_checks_merged_values_and_reports_without_changing_filters(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("aliasEntries","mergedAliasEntries","metadataMaps","mapPointName","pointReference","pointCheckIssues","checkSelectedPoints")
        code="\n".join(next(line for line in page.splitlines() if line.startswith("function "+name+"(")) for name in names)
        checks=r"""
const assert=require('node:assert/strict');
const points=[{source:{id:'gsi:1',nameReading:'元の読み'},pointName:'白馬岳',nameReading:null,aliases:['別名'],aliasReadings:{},tags:['分類'],elevation:null,excluded:true},{source:{id:'osm:2'},pointName:'山2',nameReading:['よみ1','よみ2'],aliases:['別名2'],aliasReadings:{'別名2':['べつ1','べつ2']},tags:['分類'],elevation:0}];
const basePoints=[...points];basePoints.metadata={tags:[{name:'分類',nameReading:['ぶんるい1','ぶんるい2']}]};const supplementDocument={tags:{}};
const selectedIndices=new Set([0,1]);const fields={};const document={getElementById(id){return fields[id]??={textContent:'',showModal(){this.open=true;}};}};
const filters={search:'山',tags:['分類']};
assert.deepEqual(pointCheckIssues(points[0],{分類:{nameReading:['読み']}}),['よみがななし','別名の読みなし: 別名','標高なし']);assert.deepEqual(pointCheckIssues(points[1],{分類:{nameReading:['読み']}}),[]);
assert.ok(pointCheckIssues(points[1],{}).includes('タグの読みなし: 分類'));
const before=JSON.stringify(points);checkSelectedPoints(points[0]);assert.match(fields['point-check-summary'].textContent,/2 地点/);assert.match(fields['point-check-report'].textContent,/白馬岳（除外） \[gsi:1\]/);assert.match(fields['point-check-report'].textContent,/よみがななし/);assert.equal(fields['point-check-dialog'].open,true);assert.deepEqual([...selectedIndices],[0,1]);assert.deepEqual(filters,{search:'山',tags:['分類']});assert.equal(JSON.stringify(points),before);
selectedIndices.delete(1);checkSelectedPoints(points[1]);assert.match(fields['point-check-summary'].textContent,/1 地点/);assert.equal(fields['point-check-report'].textContent,'確認した項目に未入力はありません。');
"""
        result=subprocess.run(["node","-"],input=code+checks,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_select_all_and_copy_with_fallback_and_failure(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("selectAllListPoints","copyPointCheckReport")
        code="\n".join(next(line for line in page.splitlines() if line.startswith(("function "+name+"(","async function "+name+"("))) for name in names)
        checks=r"""
const assert=require('node:assert/strict');const selectedIndices=new Set([99]);let displayOrder=[8,2,5],selectionAnchor=null,multiSelecting=false,selectedIndex=99,synced=0;
function activatePoint(index){assert.equal(multiSelecting,true);selectedIndex=index;}function syncListSelection(){synced++;}
selectAllListPoints();assert.deepEqual([...selectedIndices],[8,2,5]);assert.equal(selectionAnchor,8);assert.equal(selectedIndex,8);assert.equal(synced,1);assert.equal(multiSelecting,false);
let copied='',removed=0,focused=0;const fields={'point-check-summary':{textContent:'3 地点'},'point-check-report':{textContent:'よみがななし'},'point-check-copy-status':{textContent:''},'point-check-dialog':{append(){}}};
const document={activeElement:{focus(){focused++;}},getElementById(id){return fields[id];},createElement(){return {style:{},focus(){},select(){},remove(){removed++;}};},execCommand(){return true;}};
const navigator={clipboard:{async writeText(text){copied=text;}}};
(async()=>{await copyPointCheckReport();assert.equal(copied,'3 地点\n\nよみがななし');assert.match(fields['point-check-copy-status'].textContent,/コピーしました/);
navigator.clipboard.writeText=async()=>{throw Error('拒否');};await copyPointCheckReport();assert.equal(removed,1);assert.equal(focused,1);assert.match(fields['point-check-copy-status'].textContent,/コピーしました/);
document.execCommand=()=>false;await copyPointCheckReport();assert.equal(removed,2);assert.match(fields['point-check-copy-status'].textContent,/コピーできません/);
})().catch(error=>{console.error(error);process.exitCode=1;});
"""
        result=subprocess.run(["node","-"],input=code+checks,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

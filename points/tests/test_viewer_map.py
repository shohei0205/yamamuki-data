"""地図の対象外地点の色・表示上限・重なり順を確認する。"""
from pathlib import Path
import shutil
import subprocess
import unittest

class MapTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_map_colors_budget_and_stacking(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("mapPointName","markerRadius","boundedMapIndices","mapDisplayIndices","mapPointStyle","refreshMapStacking")
        code="\n".join(next(line for line in page.splitlines() if line.startswith("function "+name+"(")) for name in names)
        checks=r"""
const assert=require('node:assert/strict');assert.equal(mapPointName({pointName:'白馬岳',excluded:true}),'白馬岳（除外）');assert.equal(mapPointName({pointName:'白馬岳',excluded:false}),'白馬岳');const MAP_POINT_LIMIT=10000;const selectedIndices=new Set();
let points=Array.from({length:30000},(_,i)=>({excluded:i>=20000}));
function rowVisible(index){return index<10000;}
let zoom=5;const map={getZoom:()=>zoom};const displayOrder=Array.from({length:10000},(_,i)=>i);
assert.equal(mapPointStyle(0).fillColor,'#176b52');assert.equal(mapPointStyle(10000).fillColor,'#858d96');assert.equal(mapPointStyle(20000).fillColor,'#b83e3e');assert.equal(mapPointStyle(20000,true).color,'#c02575');assert.equal(mapPointStyle(20000,true).fillColor,'#b83e3e');assert.equal(mapPointStyle(0,true).fillColor,'#ffdfef');assert.equal(mapPointStyle(0,true).fillOpacity,1);
const candidates=points.map((_,i)=>i);let chosen=mapDisplayIndices(candidates,null);
assert.equal(chosen.length,10000);assert.equal(new Set(chosen).size,10000);assert.equal(chosen.filter(i=>i<10000).length,6000);assert.equal(chosen.filter(i=>i>=10000&&i<20000).length,2000);assert.equal(chosen.filter(i=>i>=20000).length,2000);
chosen=mapDisplayIndices(candidates,29999);assert.ok(chosen.includes(29999));assert.equal(chosen.length,10000);
assert.deepEqual(mapDisplayIndices([0,10000,20000],null),[0,10000,20000]);
assert.equal(mapDisplayIndices(candidates.slice(10000),null).length,10000);
assert.equal(markerRadius(10000),2);assert.equal(markerRadius(20000),2);displayOrder.length=100;assert.equal(markerRadius(0),7);assert.equal(markerRadius(10000),2);assert.equal(markerRadius(20000),2);assert.equal(mapPointStyle(10000).weight,1);assert.equal(mapPointStyle(20000).weight,1);zoom=9;assert.equal(markerRadius(10000),3);zoom=12;assert.equal(markerRadius(10000),5);assert.equal(markerRadius(20000),5);zoom=15;assert.equal(markerRadius(10000),6);assert.equal(markerRadius(20000),6);
selectedIndices.add(20000);let selectedIndex=20000;const order=[];const markers=new Map([0,20000,10000].map(index=>[index,{bringToFront(){order.push(index);}}]));const selectionPulse={bringToFront(){order.push('pulse');}};
refreshMapStacking();assert.deepEqual(order,[10000,0,20000,20000,'pulse']);selectedIndices.add(123);selectedIndices.add(29997);const selectedMap=mapDisplayIndices(candidates,29997);assert.ok(selectedMap.includes(123));assert.ok(selectedMap.includes(29997));assert.ok(selectedMap.includes(20000));assert.equal(selectedMap.length,10000);
"""
        result=subprocess.run(["node","-"],input=code+checks,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

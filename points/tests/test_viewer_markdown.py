"""選択地点の Markdown 表とクリップボードへのコピーを確認する。"""
from pathlib import Path
import shutil
import subprocess
import unittest

class MarkdownTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_markdown_uses_merged_values_list_order_and_escaped_cells(self):
        page=(Path(__file__).resolve().parents[1]/"viewer.html").read_text(encoding="utf-8")
        names=("readingText","pointReference","aliasEntries","mergedAliasEntries","aliasDisplayText","markdownTableCell","selectedPointsMarkdown","copySelectedPointsMarkdown")
        code="\n".join(next(line for line in page.splitlines() if line.startswith(("function "+name+"(","async function "+name+"("))) for name in names)
        checks=r"""
const assert=require('node:assert/strict');
const points=[{source:{id:'gsi-mountain-list:1',name:'元の峰'},pointName:'新しい山',nameReading:['よみ1','よみ2'],aliases:['別名'],aliasReadings:{'別名':['べつ1','べつ2']}},{source:{id:'gsi-vector:2',name:'山2'},pointName:'山2'},{source:{id:'gsi-vector:3'},excluded:true},{source:{id:'osm:4',name:'山4'},pointName:'山4'}];
const selectedIndices=new Set([0,1,2]),displayOrder=[1,0,2,3];let copied,message;
async function copyText(text){copied=text;}function showToast(text){message=text;}
(async()=>{const body=selectedPointsMarkdown([0,1,2]);assert.equal(body.count,2);assert.match(body.text,/国土地理院での名称/);assert.match(body.text,/元の峰 \| 新しい山 \| よみ1、よみ2 \| 別名（べつ1、べつ2）/);assert.equal(body.text.trim().split('\n').length,4);assert.ok(!body.text.includes('gsi-vector:3'));assert.throws(()=>selectedPointsMarkdown([2]),/すべて/);
assert.equal(markdownTableCell('山|峰\n<別名>&'),String.raw`山\|峰<br>&lt;別名&gt;&amp;`);
assert.equal(markdownTableCell(String.raw`山\峰`),String.raw`山\\峰`);
assert.match(selectedPointsMarkdown([3]).text,/元データでの名称/);
await copySelectedPointsMarkdown(points[0]);assert.ok(copied.indexOf('山2')<copied.indexOf('新しい山'));assert.match(message,/2 件/);assert.match(message,/著書順ではありません/);
await copySelectedPointsMarkdown(points[3]);assert.match(copied,/osm:4/);assert.ok(!copied.includes('新しい山'));
})().catch(error=>{console.error(error);process.exitCode=1;});
"""
        result=subprocess.run(["node","-"],input=code+checks,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

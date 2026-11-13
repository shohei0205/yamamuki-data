"""追加地点の補足保存・再読み込みと生成処理を確認する。"""
from copy import deepcopy
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "points"))
sys.path.insert(0, str(ROOT / "points/osm-peaks"))
from scripts.supplements import apply_supplements, supplement_document, validate_supplements


class PointAdditionTests(unittest.TestCase):
    def setUp(self):
        self.base = [{"id":"osm:1","name":"旭岳","latitude":43.66,"longitude":142.85}]
        self.point = {"id":"custom:daisetsuzan","name":"大雪山","latitude":43.66,"longitude":142.85,"provenance":{"dataSource":"国土地理院"}}
        self.data = {"schemaFormat":"yamamuki-point-supplements","schemaVersion":3,"metadata":{"sources":[{"name":"国土地理院","source":"https://example.com/reference","license":"PDL-1.0"}]},"points":[{**self.point,"add":True}]}

    def test_add_roundtrip_correct_and_exclude(self):
        original = deepcopy(self.base)
        data = validate_supplements(self.data)
        exported = supplement_document(data)
        self.assertEqual(exported["schemaVersion"], 3)
        self.assertEqual(exported["points"], [{**self.point,"add":True}])
        self.assertEqual(apply_supplements(self.base, exported), self.base + [self.point])
        exported["points"][0].update(nameReading="たいせつざん",tags=["日本百名山"])
        added = apply_supplements(self.base, exported)[1]
        self.assertEqual(added["nameReading"], "たいせつざん")
        self.assertEqual(added["tags"], ["日本百名山"])
        exported["points"][0]["exclude"] = True
        self.assertEqual(apply_supplements(self.base, exported), self.base)
        self.assertEqual(self.base, original)

    def test_invalid_added_points_and_collisions(self):
        for changes in ({"id":"osm:1"},{"id":"custom:existing"},{"latitude":91},{"longitude":-181},{"latitude":True},{"elevationM":"100"},{"name":""},{"provenance":{}},{"nameReading":[]},{"type":"山系"},{"aliases":["別名"]}):
            data = deepcopy(self.data)
            data["points"][0].update(changes)
            base = self.base + [{**self.point,"id":"custom:existing"}]
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                apply_supplements(base, data)
        data = deepcopy(self.data)
        data["points"].append({**self.point,"add":True})
        with self.assertRaises(ValueError):validate_supplements(data)
        data = deepcopy(self.data)
        data["metadata"]["sources"] = []
        with self.assertRaises(ValueError):apply_supplements(self.base,data)
        for version in (1,2):
            data = deepcopy(self.data)
            data["schemaVersion"] = version
            with self.assertRaises(ValueError):validate_supplements(data)

    def test_osm_distribution_parts_reproduce_added_point(self):
        from scripts.build_data import write_distribution
        from scripts.distribution_parts import write_parts, validate_parts
        data = validate_supplements(self.data)
        integrated = apply_supplements(self.base,data)
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            manifest = write_distribution(integrated,root,'test-added','2026-10-01T00:00:00Z','2026-10-01T00:00:00Z',supplement_sources=data['sources'])
            write_parts(root,self.base,data,manifest)
            validate_parts(root,manifest,integrated)
            self.assertEqual(manifest['metadata']['pointCount'],2)

    def test_flag_rules_and_required_fields(self):
        for key in ("id","name","latitude","longitude","provenance"):
            data = deepcopy(self.data)
            del data["points"][0][key]
            with self.subTest(key=key), self.assertRaises(ValueError):validate_supplements(data)
        for changes in ({"add":"true"},{"add":False},{"add":True,"expected":{"name":"元の山"}}):
            data = deepcopy(self.data)
            data["points"][0].update(changes)
            with self.subTest(changes=changes), self.assertRaises(ValueError):validate_supplements(data)
        data = {"schemaFormat":"yamamuki-point-supplements","schemaVersion":3,"points":[{"id":"osm:1","add":False,"name":"新しい旭岳"}]}
        self.assertEqual(apply_supplements(self.base,data)[0]["name"],"新しい旭岳")

    @unittest.skipUnless(shutil.which("node"), "Node.js が必要です")
    def test_viewer_roundtrip_and_input_validation(self):
        page = (ROOT / "points/viewer.html").read_text(encoding="utf-8")
        source = page.split("<script>\n",1)[1].split("let points=[];",1)[0]
        source += "\n" + next(line for line in page.splitlines() if line.startswith("function entriesFromJson("))
        source += r"""
const assert=require('node:assert/strict');
const input={id:'custom:daisetsuzan',name:'大雪山',reading:'たいせつざん',latitude:'43.66',longitude:'142.85',elevation:'',type:'',source:'国土地理院',sourceUrl:'https://example.com/reference',license:'PDL-1.0',attribution:'出典：国土地理院'};
const base=entriesFromJson([{id:'osm:1',name:'旭岳',latitude:43.66,longitude:142.85}]);
const old={schemaFormat:'yamamuki-point-supplements',schemaVersion:2,points:[],sources:{}};
const added=addPointSupplement(old,base,input);
assert.equal(old.schemaVersion,2);assert.equal(old.points.length,0);
assert.equal(added.points[0].type,undefined);assert.equal(added.points[0].elevationM,undefined);
let saved=supplementJson(added),loaded=parseSupplement(JSON.stringify(saved));
assert.equal(saved.schemaVersion,3);assert.equal(saved.metadata.sources[0].name,'国土地理院');
assert.equal(applySupplement(base,loaded).length,2);
Object.assign(loaded.points[0],{name:'大雪山系',tags:['日本百名山']});
assert.equal(applySupplement(base,loaded)[1].pointName,'大雪山系');
loaded.points[0].exclude=true;assert.equal(applySupplement(base,loaded)[1].excluded,true);
for(const patch of [{id:''},{id:'osm:1'},{id:'osm:99'},{latitude:''},{latitude:'91'},{longitude:'-181'},{elevation:'NaN'},{sourceUrl:'javascript:alert(1)'},{name:''},{license:''}])assert.throws(()=>addPointSupplement(old,base,{...input,...patch}));
assert.throws(()=>addPointSupplement(loaded,base,input));
assert.throws(()=>addPointSupplement(added,base,{...input,id:'custom:2',license:'別の条件'}));
assert.throws(()=>applySupplement(base,{...loaded,sources:{}}));
assert.throws(()=>validateSupplement({...loaded,schemaVersion:2}));
assert.equal(supplementJson(old).schemaVersion,2);
"""
        result = subprocess.run(["node","-"],input=source,text=True,encoding="utf-8",capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)

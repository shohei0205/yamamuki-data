"""曲面ヒントの生成側とブラウザー側の計算一致を確認する。"""
import json,subprocess,unittest
from pathlib import Path
import numpy as np
from curve_hints import design,ROOT

class CurveHintsTest(unittest.TestCase):
    def test_plane_and_browser(self):
        points=np.array([[0.,0.,100.],[1000.,0.,150.],[0.,1000.,40.]])
        faces=np.array([[0,1,2]])
        weights=np.array([[1-i/4-j/4,i/4,j/4] for i in range(5) for j in range(5-i)])
        base,A=design(points,faces,np.zeros(len(weights),int),weights)
        slopes=np.tile([.05,-.06],3)
        np.testing.assert_allclose(base+A@slopes,(weights*points[:,2]).sum(axis=1),atol=1e-10)
        slopes=np.array([.1,-.04,.02,.08,-.03,-.09])
        fixture=dict(points=points.ravel().tolist(),slopes=slopes.tolist(),samples=np.column_stack([weights@points[:,:2],base+A@slopes]).tolist())
        script="""const fs=require('fs'),assert=require('assert'),{subdivideTerrain}=require('./tools/terrain/pn-terrain.js');const f=JSON.parse(fs.readFileSync(0,'utf8')),r=subdivideTerrain({points:new Float32Array(f.points),indices:new Uint16Array([0,1,2]),vertexNormals:new Float32Array([0,0,1,0,0,1,0,0,1])},4,1,16000,{method:'fitted',slopes:f.slopes});for(const [x,y,z] of f.samples){let found=false;for(let i=0;i<r.points.length;i+=3)if(r.points[i]===x&&r.points[i+1]===y){assert(Math.abs(r.points[i+2]-z)<.0001);found=true;}assert(found);}console.log('生成と表示の高さが一致');"""
        node=Path('C:/Users/hori/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node.exe')
        subprocess.run([str(node),'-e',script],input=json.dumps(fixture),text=True,cwd=ROOT,check=True)
if __name__=='__main__':unittest.main()

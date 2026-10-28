"""保存済み制御点をそのまま結んだ確認用ワイヤーを出力する。"""
import json
from pathlib import Path
import numpy as np
from scipy.spatial import Delaunay
from mls_quant_codec import decode_quant


def export():
    view=Path('build/mls-generated-preview/dem-regions')
    for region in ['fuji','yari','tsukuba']:
        for root,kind in [('mls-six-point-v3','50-six-trained'),('mls-xy-placement-v1','50-six-xy'),
                          ('mls-xy-placement-v1','50-six-more'),('mls-smooth-refit-v1','50-lpf-refit')]:
            p=decode_quant((Path('build')/root/region/kind/'model.mlq1').read_bytes())
            triangles=Delaunay(p[:,:2]).simplices.astype('<u4')
            edges=np.unique(np.sort(np.vstack((triangles[:,[0,1]],triangles[:,[1,2]],triangles[:,[2,0]])),axis=1),axis=0).astype('<u4')
            dest=view/region/kind
            Path(str(dest)+'-controls.bin').write_bytes(p.astype('<f4').tobytes())
            Path(str(dest)+'-triangles.bin').write_bytes(triangles.tobytes())
            Path(str(dest)+'-edges.bin').write_bytes(edges.tobytes())
            Path(str(dest)+'-wire.json').write_text(json.dumps(dict(points=len(p),triangles=len(triangles),edges=len(edges))),encoding='utf-8')


def inject(s):
    s=s.replace('<label>左側 ', '<label>右側表示 <select id="renderMode"><option value="surface">補間後の曲面</option><option value="wire">制御点のワイヤーフレーム</option></select></label>　<label>左側 ')
    s=s.replace('uniform float yaw,tilt,span,aspect,focus;', 'uniform float yaw,tilt,span,aspect,focus;')
    s=s.replace('precision highp float;in vec3 normal;', 'precision highp float;uniform float wireColor;in vec3 normal;')
    s=s.replace('color=vec4(base*(.39+.70*diffuse),1.);', 'color=wireColor>0.5?vec4(.07,.11,.08,1.):vec4(base*(.39+.70*diffuse),1.);')
    s=s.replace('const xyBuffers=[],fieldBuffers=[];', '''let controlWire=null;
function releaseWire(){if(controlWire){gl.deleteVertexArray(controlWire.vao);for(const b of controlWire.buffers)gl.deleteBuffer(b);controlWire=null;}}
function createWire(points,triangles,edges){releaseWire();const p=new Float32Array(points),xy=new Float32Array(p.length/3*2),field=new Float32Array(p.length);for(let i=0;i<p.length/3;i++){xy[2*i]=p[3*i];xy[2*i+1]=p[3*i+1];field[3*i]=p[3*i+2];}const vao=gl.createVertexArray();gl.bindVertexArray(vao);const buffers=[attr('xy',xy,2),attr('field',field,3)];for(const data of [triangles,edges]){const b=gl.createBuffer();gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,b);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,new Uint32Array(data),gl.STATIC_DRAW);buffers.push(b);}controlWire={vao,buffers,triangles:triangles.byteLength/4,edges:edges.byteLength/4,points:p.length/3};}
function drawWire(){gl.bindVertexArray(controlWire.vao);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,controlWire.buffers[2]);gl.colorMask(false,false,false,false);gl.enable(gl.POLYGON_OFFSET_FILL);gl.polygonOffset(1,1);gl.drawElements(gl.TRIANGLES,controlWire.triangles,gl.UNSIGNED_INT,0);gl.disable(gl.POLYGON_OFFSET_FILL);gl.colorMask(true,true,true,true);gl.uniform1f(gl.getUniformLocation(program,'wireColor'),1);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,controlWire.buffers[3]);gl.drawElements(gl.LINES,controlWire.edges,gl.UNSIGNED_INT,0);gl.uniform1f(gl.getUniformLocation(program,'wireColor'),0);window.controlWireStats={points:controlWire.points,edges:controlWire.edges/2};}
const xyBuffers=[],fieldBuffers=[];''')
    s=s.replace('gl.drawElements(gl.TRIANGLES,window.indexCount,gl.UNSIGNED_INT,0);', "if(i===1&&document.getElementById('renderMode').value==='wire'&&controlWire)drawWire();else gl.drawElements(gl.TRIANGLES,window.indexCount,gl.UNSIGNED_INT,0);")
    s=s.replace(' if(ticket!==loadTicket)return;', ''' const wireData=await Promise.all(['controls','triangles','edges'].map(part=>fetch('../dem-regions/'+region+'/'+modelKey+'-'+part+'.bin').then(r=>{if(!r.ok)throw Error('制御点データを取得できません');return r.arrayBuffer();})));
 if(ticket!==loadTicket)return;
 createWire(...wireData);''')
    s=s.replace("+'&reference='+document.getElementById('referenceSelect').value", "+'&reference='+document.getElementById('referenceSelect').value+'&render='+document.getElementById('renderMode').value")
    s=s.replace('adjustVariant();\nloadRange()', "document.getElementById('renderMode').value=params.get('render')==='wire'?'wire':'surface';\ndocument.getElementById('renderMode').onchange=()=>{draw();const url=new URL(location);url.searchParams.set('render',document.getElementById('renderMode').value);history.replaceState(null,'',url);};\nadjustVariant();\nloadRange()")
    s=s.replace('<p>左ドラッグ：', '<p>ワイヤー表示は保存済み XYZ を XY の Delaunay 分割で結んだものです。MLS 曲面の分割線ではありません。線は表示確認用に生成し、保存モデルには追加しません。</p><p>左ドラッグ：')
    return s


if __name__=='__main__':export()

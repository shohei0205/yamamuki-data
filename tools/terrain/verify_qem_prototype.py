"""保存済み QEM 試作の標高差・外周・穴・面の向きを照合する。"""
import argparse
from contextlib import closing
import gzip
import json
from pathlib import Path
import sqlite3
import numpy as np
from overlay_region import Surface


def boundary(vertices,faces):
    edges=np.sort(np.concatenate([faces[:,[0,1]],faces[:,[1,2]],faces[:,[2,0]]]),axis=1)
    edges,counts=np.unique(edges,axis=0,return_counts=True)
    if np.any(counts>2):raise ValueError('一つの辺に三つ以上の面があります')
    return {tuple(sorted((tuple(vertices[a]),tuple(vertices[b])))) for a,b in edges[counts==1]}


def verify(output):
    output=Path(output);report=json.loads((output/'report.json').read_text(encoding='utf-8'))
    previous=None;expected_boundary=None;base_area=None;probes=None;peak=None
    with closing(sqlite3.connect((output/'lod.sqlite3').resolve().as_uri()+'?mode=ro',uri=True)) as db:
        for item in report['levels']:
            nv,nt,body=db.execute('SELECT vertices,triangles,data FROM levels WHERE level=?',(item['level'],)).fetchone()
            raw=gzip.decompress(body)
            if len(raw)!=nv*12+nt*12:raise ValueError('バイナリの長さが不正です')
            v=np.frombuffer(raw[:nv*12],'<f4').reshape(-1,3).astype(float);f=np.frombuffer(raw[nv*12:],'<u4').reshape(-1,3)
            if f.size and f.max()>=nv:raise ValueError('頂点番号が不正です')
            a,b,c=v[f].transpose(1,0,2);area=np.cross(b-a,c-a)[:,2]/2
            if np.any(area<=0):raise ValueError('裏返った面や退化した面があります')
            points={tuple(p) for p in v};edges=boundary(v,f)
            if probes is None:
                probes=np.concatenate([v,v[f].mean(axis=1)]);base_area=area.sum();expected_boundary=edges;peak=v[:,2].max()
            if previous is not None and not points.issubset(previous):raise ValueError('低レベルに新しい頂点が追加されています')
            if edges!=expected_boundary:raise ValueError('外周や穴の輪郭が変わっています')
            if abs(area.sum()-base_area)>base_area*1e-9:raise ValueError('面の合計面積が変わっています')
            if v[:,2].max()!=peak:raise ValueError('最高点の標高が変わっています')
            error=np.abs(Surface(v,f.ravel()).heights(probes[:,:2])-probes[:,2])
            if not np.isfinite(error).all():raise ValueError('最高詳細の標本点に穴が開いています')
            if error.max()>item['sampleErrorLimitMeters']+.001:raise ValueError('元の最高詳細との標高差が制限を超えています')
            if abs(error.max()-item['maxSampleErrorMeters'])>.002:raise ValueError('保存データの標高差と報告値が一致しません')
            previous=points
    print(json.dumps({'levels':len(report['levels']),'samplesPerLevel':len(probes),'unchangedBoundaryEdges':len(expected_boundary),'allChecksPassed':True}))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('output',type=Path,nargs='?',default=Path('build/terrain-qem-final'))
    verify(parser.parse_args().output)

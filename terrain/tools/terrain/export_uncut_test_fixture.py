"""接続復元後の隣接タイルを曲面の一致検証用に書き出す。"""
import sqlite3,zlib,struct,json
import numpy as np
from national_independent_mesh import ROOT
with sqlite3.connect(ROOT/'build/terrain-shared-allocation-terrain128.sqlite3') as db:
    manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
    area=next(a for a in manifest['reliefRegions'] if a['id']=='yari');keys=area['levels'][0]['keys'];x=min(k[1] for k in keys)+8;y=min(k[2] for k in keys)+8
    result=[]
    for dx in [0,1]:
        raw=zlib.decompress(db.execute('SELECT data FROM topology WHERE level=3 AND x=? AND y=?',(x+dx,y)).fetchone()[0]);_,n,nv,ni,core=struct.unpack_from('<4sIIII',raw)
        p=np.frombuffer(raw,'<f4',nv*3,20).reshape(-1,3);f=np.frombuffer(raw,'<u4',ni,20+nv*12)
        normals=np.frombuffer(raw,'<f4',nv*3,20+nv*12+ni*4)
        result.append(dict(dx=dx,points=p.ravel().tolist(),indices=f[:core].tolist(),halo=f[core:].tolist(),normals=normals.tolist()))
    (ROOT/'build/uncut-loop-fixture.json').write_text(json.dumps(result),encoding='utf-8')

"""既存の比較データを残し、頂点法線を分離した配信用SQLiteを作る。"""
import json,sqlite3,struct,zlib
from progressive_tin_compare import ROOT

def split(raw):
    if raw[:4]==b'PVX1':
        first,n=struct.unpack_from('<II',raw,4);points=bytearray(struct.pack('<4sII',b'PVX2',first,n));normals=bytearray(struct.pack('<4sII',b'PVN1',first,n))
        for i in range(n):points.extend(raw[12+i*10:20+i*10]);normals.extend(raw[20+i*10:22+i*10])
        return bytes(points),bytes(normals)
    if raw[:4]==b'YOP1':
        n,ni=struct.unpack_from('<II',raw,4);at=12+n*12+ni*2
        return b'YOP2'+raw[4:at],struct.pack('<4sII',b'PVN1',0,n)+raw[at:]
    raise ValueError('未対応のメッシュ形式')

def generate():
    with sqlite3.connect(ROOT/'build/progressive-split.sqlite3') as out:
        out.execute('CREATE TABLE IF NOT EXISTS blobs(region TEXT,format TEXT,level INTEGER,kind TEXT,data BLOB,PRIMARY KEY(region,format,level,kind))')
        out.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
        meta={}
        for filename,reference in [('progressive-tin-compare.sqlite3',False),('progressive-tin-reference.sqlite3',True),('progressive-tin-coarse.sqlite3',False)]:
            with sqlite3.connect((ROOT/'build'/filename).resolve().as_uri()+'?mode=ro',uri=True) as db:
                m=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
                for region,fmt,level,kind,data in db.execute('SELECT * FROM blobs'):
                    if reference and fmt!='independent':continue
                    target='reference' if reference else 'coarse_'+fmt if 'coarse' in filename else fmt
                    if kind=='vertices':
                        points,normals=split(zlib.decompress(data))
                        for k,raw in [('vertices',points),('normals',normals)]:out.execute('INSERT OR REPLACE INTO blobs VALUES(?,?,?,?,?)',(region,target,level,k,zlib.compress(raw,9)))
                    else:out.execute('INSERT OR REPLACE INTO blobs VALUES(?,?,?,?,?)',(region,target,level,kind,data))
                for region,r in m['regions'].items():
                    for fmt in (['independent'] if reference else ['independent','shared','history']):
                        target='reference' if reference else 'coarse_'+fmt if 'coarse' in filename else fmt
                        r['totalBytes'][fmt]=out.execute('SELECT sum(length(data)) FROM blobs WHERE region=? AND format=?',(region,target)).fetchone()[0]
                        for level,stage in enumerate(r['stages']):
                            size=out.execute('SELECT sum(length(data)) FROM blobs WHERE region=? AND format=? AND level=?',(region,target,level)).fetchone()[0]
                            stage[fmt+'Bytes']=size
                if reference:meta['reference']=m
                elif 'coarse' in filename:meta['coarse']=m
                else:meta.update(m)
        with sqlite3.connect((ROOT/'build/tin-optimized-compare.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
            m=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
            for region,variant,level,data in db.execute('SELECT * FROM meshes'):
                points,normals=split(zlib.decompress(data));total=0
                for kind,raw in [('mesh',points),('normals',normals)]:
                    body=zlib.compress(raw,9);total+=len(body);out.execute('INSERT OR REPLACE INTO blobs VALUES(?,?,?,?,?)',(region,'opt_'+variant,level,kind,body))
                m['regions'][region]['levels'][level]['variants'][variant]['bytes']=total
            meta['optimization']=m
        out.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('manifest',json.dumps(meta,ensure_ascii=False)))
    print('頂点法線を別BLOBに分離しました')
if __name__=='__main__':generate()

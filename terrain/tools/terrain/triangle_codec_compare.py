"""比較SQLiteを読み取り専用で参照し、三角形圧縮の試験入出力を作る。"""
import base64
import json
import sqlite3
import zlib
from pathlib import Path

from hierarchical_lod_compare import OUTPUT


def export():
    rows = []
    with sqlite3.connect((OUTPUT/'variable-tiles.sqlite3').as_uri()+'?mode=ro',uri=True) as db:
        for table in ['meshes','mixed_meshes']:
            query = 'SELECT region,level,x,y,vertices,indices FROM meshes WHERE level BETWEEN 0 AND 3 ORDER BY region,level,x,y' if table=='meshes' else 'SELECT region,width,x,y,vertices,indices FROM mixed_meshes ORDER BY region,width,x,y'
            for region,level,x,y,vertices,indices in db.execute(query):
                body = zlib.decompress(indices)
                assert len(body)%6==0
                rows.append({'region':region,'level':str(level) if table=='meshes' else 'mixed','tile':f'{level}:{x}:{y}','vertexBytes':len(vertices),'indices':base64.b64encode(body).decode('ascii')})
    (OUTPUT/'triangle-codec-input.json').write_text(json.dumps(rows),encoding='utf-8')
    print(f'{len(rows)}タイルを出力')


def save():
    report = json.loads((OUTPUT/'triangle-codec-report.json').read_text(encoding='utf-8'))
    report['storageSettings'] = json.loads(Path(__file__).with_name('mesh_storage_settings.json').read_text(encoding='utf-8'))
    (OUTPUT/'triangle-codec-report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    rows = json.loads((OUTPUT/'triangle-codec-blobs.json').read_text(encoding='utf-8'))
    with sqlite3.connect(OUTPUT/'triangle-codec-comparison.sqlite3') as db:
        db.execute('CREATE TABLE IF NOT EXISTS indices(region TEXT,level TEXT,tile TEXT,codec TEXT,index_count INTEGER,data BLOB,PRIMARY KEY(region,level,tile,codec))')
        for row in rows:
            for codec,body in row['encoded'].items():
                db.execute('INSERT OR REPLACE INTO indices VALUES(?,?,?,?,?,?)',(row['region'],row['level'],row['tile'],codec,row['indexCount'],base64.b64decode(body)))
                saved = db.execute('SELECT data FROM indices WHERE region=? AND level=? AND tile=? AND codec=?',(row['region'],row['level'],row['tile'],codec)).fetchone()[0]
                assert saved==base64.b64decode(body), 'SQLite保存後のBLOB不一致'
        db.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
        db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('report',json.dumps(report,ensure_ascii=False)))
        expected = sum(len(row['encoded']) for row in rows)
        assert db.execute('SELECT count(*) FROM indices').fetchone()[0]==expected
    (OUTPUT/'triangle-codec.html').write_text(Path(__file__).with_suffix('.html').read_text(encoding='utf-8'),encoding='utf-8')
    browser_input = [{'region':r['region'],'level':r['level'],'count':r['indexCount'],'tfc':r['encoded']['frontierGreedy'],'meshopt':r['encoded']['cacheMeshopt']} for r in rows]
    (OUTPUT/'triangle-codec-browser-input.json').write_text(json.dumps(browser_input),encoding='utf-8')
    for name in ['triangle_frontier_wasm.mjs','triangle_frontier_codec.mjs','triangle_codec_browser.mjs']:
        (OUTPUT/name).write_bytes(Path(__file__).with_name(name).read_bytes())
    vendor=OUTPUT.parent/'terrain-codec-tools/web/meshoptimizer'
    for source,dest in [('meshopt_decoder.mjs','meshopt_decoder.mjs'),('LICENSE.md','meshoptimizer-LICENSE.md')]:
        (OUTPUT/dest).write_bytes((vendor/source).read_bytes())


if __name__ == '__main__':
    import sys
    {'export':export,'save':save}[sys.argv[1]]()

"""32 km比較地域を元DEMから用意し、全メッシュと指定面積タイルを生成する。"""
import json,sqlite3,zlib
from pathlib import Path
from fixed_mesh_preview import source_grid,ROOT,REGIONS
from adaptive_mesh_preview import generate
from adaptive_mesh_parts import generate as parts
OUTPUT=ROOT/'build/fixed-mesh-preview-32km.sqlite3'
def main():
    if OUTPUT.exists():raise FileExistsError('32 km 比較DBは既存のため上書きしません')
    with sqlite3.connect(ROOT/'build/fixed-mesh-preview.sqlite3') as old,sqlite3.connect(OUTPUT) as db:
        old.backup(db)
        m=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);m['adaptiveOnly']=True;m['notes']='32 km四方。メッシュのL番号はDEMズームと別。元DEMから1369四方へ再標本化したPNGとの形状近似誤差。'
        with sqlite3.connect((ROOT/'build/terrain-lod.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as src:
            for region,(lat,lon) in REGIONS.items():
                grid,_=source_grid(src,lat,lon,extent=32000,size=1369);packed=zlib.compress(grid.astype('<f4').tobytes(),9)
                db.execute('INSERT OR REPLACE INTO blobs VALUES(?,?,?,?)',(region,'height',1369,packed))
                meta=m['regions'][region];meta.update(extent=32000,gridSize=1369,heightBytes=len(packed),heightTriangles=2*1368**2,adaptiveMeshes=[],adaptiveFloors={});meta.pop('adaptiveSpatial',None)
        db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(m,ensure_ascii=False),));db.commit()
    generate(database=OUTPUT,tolerances=(1000,500,250,100,50,30,20,10,5,2),max_vertices=1369**2)
    for t,floor in [(100,512),(250,128),(500,64),(1000,32)]:generate(database=OUTPUT,min_vertices=floor,tolerances=(t,),max_vertices=1369**2)
    parts(OUTPUT)
if __name__=='__main__':main()

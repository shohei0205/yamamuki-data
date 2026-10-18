"""最大の標高幅を持つ L12 タイルでブラウザー復号を調べる試料を用意する。"""
import json,sqlite3,subprocess
from pathlib import Path

def prepare(root):
 out=root/'build/native-jxl-probe';out.mkdir(exist_ok=True,parents=True)
 with sqlite3.connect((root/'build/terrain-height-jxl-national-L7-L12-d02-regenerated/terrain-height-jxl.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
  row=db.execute('SELECT z,x,y,height_min,height_scale,height_jxl FROM tiles WHERE z=12 ORDER BY height_scale DESC LIMIT 1').fetchone()
 (out/'sample.jxl').write_bytes(row[5])
 (out/'meta.json').write_text(json.dumps(dict(z=row[0],x=row[1],y=row[2],minimum=row[3],scale=row[4])),encoding='utf-8')
 subprocess.run([str(root/'build/terrain-codec-tools/jxl-0.12.0/x64-windows-static/bin/djxl.exe'),str(out/'sample.jxl'),str(out/'sample.pgm'),'--bits_per_sample=16','--color_space=Gra_D65_Rel_Lin','--quiet'],check=True)
if __name__=='__main__':prepare(Path(__file__).resolve().parents[2])

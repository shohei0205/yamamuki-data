"""全国 JPEG XL を、標高値を保ったブラウザー表示用 PNG へ事前復号する。"""
from contextlib import closing
import argparse,hashlib,json,sqlite3,subprocess,tempfile
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[2]
def prepare(source,output,decoder):
 output.mkdir(parents=True,exist_ok=True)
 with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)) as db:
  config=json.loads(db.execute("SELECT value FROM metadata WHERE key='config'").fetchone()[0])
  rows=db.execute('SELECT z,x,y,height_min,height_scale,height_jxl,valid_mask_zstd FROM tiles ORDER BY z,x,y').fetchall()
 digest=hashlib.sha256(source.read_bytes()).hexdigest()
 stamp=output/'source.json'
 if stamp.exists() and json.loads(stamp.read_text())['sha256']!=digest:raise ValueError('別の元データのキャッシュです')
 stamp.write_text(json.dumps(dict(sha256=digest,source=str(source.resolve()))),encoding='utf-8')
 def one(row):
  z,x,y,minimum,scale,jxl,mask=row;folder=output/str(z)/str(x);folder.mkdir(parents=True,exist_ok=True);png=folder/f'{y}.png'
  if not png.exists():
   with tempfile.TemporaryDirectory(dir=output) as temp:
    inp=Path(temp)/'tile.jxl';pgm=Path(temp)/'tile.pgm';inp.write_bytes(jxl)
    subprocess.run([str(decoder),str(inp),str(pgm),'--bits_per_sample=16','--color_space=Gra_D65_Rel_Lin','--num_threads=1','--quiet'],check=True,capture_output=True,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
    q=np.array(Image.open(pgm),dtype=np.uint16);assert q.shape==(256,256)
    rgb=np.zeros((256,256,3),np.uint8);rgb[:,:,0]=q&255;rgb[:,:,1]=q>>8;rgb[:,:,2]=255
    Image.fromarray(rgb).save(png)
  (folder/f'{y}.mask').write_bytes(mask)
  return [z,x,y,minimum,scale,len(jxl)+len(mask),png.stat().st_size]
 tiles=[]
 with ThreadPoolExecutor(max_workers=6) as pool:
  for result in pool.map(one,rows):
   tiles.append(result)
   if len(tiles)%128==0:print(len(tiles),len(rows),flush=True)
 manifest=dict(version=1,sourceSHA256=digest,config=config,tileSize=256,encoding='R + 256*G; meters = min + value*scale',tiles=tiles)
 (output/'manifest.json').write_text(json.dumps(manifest,separators=(',',':'),ensure_ascii=False),encoding='utf-8')
 print('complete',len(tiles),flush=True)
if __name__=='__main__':
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--source',type=Path,default=ROOT/'build/terrain-height-jxl-national-L7-L12-d02-regenerated/terrain-height-jxl.sqlite3');p.add_argument('--output',type=Path,default=ROOT/'build/terrain-height-browser');a=p.parse_args();prepare(a.source,a.output,ROOT/'build/terrain-codec-tools/jxl-0.12.0/x64-windows-static/bin/djxl.exe')

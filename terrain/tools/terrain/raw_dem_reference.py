"""保存済み元PNGの標高を、欠損を維持して直接参照する。"""
import json,hashlib
from pathlib import Path
import numpy as np
from scipy.ndimage import map_coordinates
from ridges import decode_png
WORLD=40075016.68557849
STEP=WORLD/(2**14*256)
SPAN=32000.
class RawDem:
 def __init__(self,cache,center,half=16000.):
  self.center=np.asarray(center,float);lo=np.floor((self.center-half)/STEP-.5).astype(int)-1;hi=np.ceil((self.center+half)/STEP-.5).astype(int)+1
  self.left,self.top=(lo//256)*256;right,bottom=(hi//256+1)*256;self.grid=np.full((bottom-self.top,right-self.left),np.nan,np.float32);manifest=[]
  for y in range(self.top//256,bottom//256):
   for x in range(self.left//256,right//256):
    path=Path(cache)/'14'/str(x)/f'{y}.png'
    if not path.exists():raise FileNotFoundError(f'元DEMタイルがありません: {path}')
    blob=path.read_bytes();h=decode_png(blob).astype('<f4');iy=y*256-self.top;ix=x*256-self.left;self.grid[iy:iy+256,ix:ix+256]=h;manifest.append(dict(x=x,y=y,sha256=hashlib.sha256(blob).hexdigest()))
  self.manifest=manifest;self.digest=hashlib.sha256(json.dumps(manifest,sort_keys=True).encode()).hexdigest()
 def sample(self,xy):
  q=(np.asarray(xy)+self.center)/STEP-.5;cx=q[:,0]-self.left;cy=q[:,1]-self.top
  if np.any(cx<0) or np.any(cy<0) or np.any(cx>=self.grid.shape[1]-1) or np.any(cy>=self.grid.shape[0]-1):raise ValueError('元DEMの読み込み範囲外')
  # 欠損はNaNのまま返し、海抜0mへ置換しない。
  return map_coordinates(self.grid,[cy,cx],order=1,prefilter=False,mode='constant',cval=np.nan)
 def native_rows(self,rows=32):
  lo=np.ceil((self.center-SPAN/2)/STEP-.5).astype(int);hi=np.floor((self.center+SPAN/2)/STEP-.5).astype(int)
  x=np.arange(lo[0],hi[0]+1);localx=(x+.5)*STEP-self.center[0]
  for start in range(lo[1],hi[1]+1,rows):
   y=np.arange(start,min(start+rows,hi[1]+1));localy=(y+.5)*STEP-self.center[1];xx,yy=np.meshgrid(localx,localy);height=self.grid[np.ix_(y-self.top,x-self.left)].ravel();valid=np.isfinite(height)
   yield np.column_stack((xx.ravel(),yy.ravel()))[valid],height[valid],int((~valid).sum())
def center_for(region):
 import sqlite3
 with sqlite3.connect('build/lpf-curve-smooth-prototype/comparison.sqlite3') as db:return json.loads(db.execute('SELECT info FROM regions WHERE name=?',(region,)).fetchone()[0])['center']

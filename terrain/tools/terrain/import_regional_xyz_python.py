"""保存済みPythonチェックポイントを一度だけRust形式へ移す。生成処理には使わない。"""
import argparse,hashlib,json,sqlite3,struct,time
from pathlib import Path
import numpy as np
LEVELS={6:640,7:500,8:400,9:320,10:240,11:160,12:80,13:40,14:20}
def encode(stage,points,active):
 points=np.asarray(points,dtype='<f8');active=np.asarray(active,dtype=np.uint8)
 if points.ndim!=2 or points.shape[1]!=3 or len(points)!=len(active) or not np.isfinite(points).all() or not active[:4].all():raise ValueError('チェックポイントの形状が不正')
 body=struct.pack('<4sII',b'RXT1',stage,len(points))+points.tobytes()+active.tobytes();return body+hashlib.sha256(body).digest()
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--output',type=Path,default=Path('build/natural-xyz-tight-errors-v1'));a=parser.parse_args();root=a.output;db=sqlite3.connect(root/'queue.sqlite3');rows=db.execute("SELECT z,x,y FROM jobs WHERE state!='ready'").fetchall();done=[]
 for z,x,y in rows:
  folder=root/f'tiles/{z}/{x}/{y}';path=folder/'rust-state.bin'
  if path.exists():continue
  fine=root/f'fine/xyz/{LEVELS[z]/2:g}/{x}-{y}';completed=fine/'control-gradient.npz';state=fine/'fit-state.npz'
  if (fine/'selection.json').exists() and completed.exists():
   info=json.loads((root/f'seed/xyz/{LEVELS[z]/2:g}/{x}-{y}/packet.json').read_text());world=np.asarray(info['origin']);p=np.load(completed)['points'];p[:,:2]=np.floor((p[:,:2]+world)/.0625+.5)*.0625-world;p[:,2]=np.floor(p[:,2]/.125+.5)*.125;mask=np.load(folder/'prune-state.npz')['active'] if (folder/'prune-state.npz').exists() else np.ones(len(p),bool);stage=1
  elif state.exists():p=np.load(state)['points'];mask=np.ones(len(p),bool);stage=0
  else:continue
  folder.mkdir(parents=True,exist_ok=True);temp=path.with_suffix('.import-next');temp.write_bytes(encode(stage,p,mask));temp.replace(path);done.append(dict(z=z,x=x,y=y,stage=stage,points=len(p)-4,remaining=int(mask.sum())-4))
 report=dict(imported=done,time=time.time(),source='preserved Python fit-state/control-gradient/prune-state');(root/'rust-import.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8');print(json.dumps(report,ensure_ascii=False))
if __name__=='__main__':main()

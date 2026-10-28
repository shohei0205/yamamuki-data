"""400m誤差目安の距離場を、独立した標高評価点で解像度選択する。"""
import json,struct,shutil
import numpy as np
from terrain_sdf import OUT,SourceSurface,generate
from raw_dem_reference import RawDem,center_for
from loop_dem_fit import sample_faces,metrics

def main():
 dem=RawDem('build/terrain-cache',center_for('yari'),half=8200);source=SourceSurface(dem)
 axis=np.linspace(-7900,7900,257);x,y=np.meshgrid(axis,axis);q=np.column_stack((x.ravel(),y.ravel()));truth=dem.sample(q);audits=[]
 for n,nz in [(16,32),(24,40),(32,48),(48,56),(64,64)]:
  report=generate(n,nz,source);path=OUT/f'yari-{n}.bin';b=path.read_bytes();size,nv,nt=struct.unpack('<III',b[:12]);offset=12+size+nv*24;p=np.frombuffer(b,dtype='<f4',count=nv*6,offset=offset).reshape(nv,6);faces=np.frombuffer(b,dtype='<u4',offset=offset+nv*24).reshape(nt,3)
  result=metrics(sample_faces(p[:,:2],faces,p[:,2],q),truth);audits.append(dict(horizontal=n,vertical=nz,**result));print('audit',audits[-1],flush=True)
  if result['maximum']>400:continue
  report.update(targetError=400,audit=result,auditDomain=[-7900,7900],auditGrid=257,audits=audits)
  header=json.dumps(report).encode();(OUT/'yari-error400.bin').write_bytes(struct.pack('<III',len(header),nv,nt)+header+b[12+size:]);(OUT/'yari-error400.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
  for extension in ['png','sdf']:shutil.copyfile(OUT/f'yari-{n}.{extension}',OUT/f'yari-error400.{extension}')
  return
 raise ValueError('400mの検査条件を満たす解像度がありません')
if __name__=='__main__':main()

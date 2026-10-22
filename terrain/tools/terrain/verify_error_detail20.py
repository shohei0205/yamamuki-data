"""新仕様の同一サイズ・混在サイズの保存被覆と境界標高を検証する。"""
import json,sqlite3,struct,zlib
from contextlib import closing
import numpy as np
from national_independent_mesh import ROOT
from refine_regional_quality import angles
from verify_shared_allocation import verify


def verify_mixed(mode='error-detail20'):
    with closing(sqlite3.connect(ROOT/f'build/terrain-shared-allocation-{mode}-mixed.sqlite3')) as db:
        m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);result={}
        for area in m['reliefRegions']:
            entries=[];ox=min(k[1]*2**k[0] for k in area['levels'][0]['keys']);oy=min(k[2]*2**k[0] for k in area['levels'][0]['keys']);cover=np.zeros((16,16),int);minimum=180.
            for key in area['levels'][0]['keys']:
                level,x,y=key;width=1000*2**level;raw=zlib.decompress(db.execute('select geometry from tiles where level=? and x=? and y=?',key).fetchone()[0]);_,n,ny,nv,ni=struct.unpack_from('<4sHHII',raw);p=np.frombuffer(raw,'<f4',nv*3,16).reshape(-1,3).astype(float);f=np.frombuffer(raw,'<u2',ni,16+nv*12).reshape(-1,3);q=p[f,:2];u=q[:,1]-q[:,0];v=q[:,2]-q[:,0];cross=u[:,0]*v[:,1]-u[:,1]*v[:,0];assert (cross>0).all() and abs(cross.sum()/2-width**2)<.1
                assert n==ny==round(width/31.25)+1;cx=round((x*2**level-ox)/8);cy=round((y*2**level-oy)/8);size=round(width/8000);cover[cy:cy+size,cx:cx+size]+=1
                raw=zlib.decompress(db.execute('select data from topology where level=? and x=? and y=?',key).fetchone()[0]);_,_,nv,ni,_=struct.unpack_from('<4sIIII',raw);points=np.frombuffer(raw,'<f4',nv*3,20).reshape(-1,3);faces=np.frombuffer(raw,'<u4',ni,20+nv*12).reshape(-1,3);minimum=min(minimum,float(angles(points,faces).min()));assert minimum>=10.-1e-6
                entries.append(dict(p=p,x=x*width,y=y*width,w=width))
            assert np.all(cover==1) and len(entries)==76
            shared=0;mixed=0;maximum=0.
            for i,a in enumerate(entries):
                for b in entries[i+1:]:
                    for axis in (0,1):
                        own='x' if axis==0 else 'y';other='y' if axis==0 else 'x'
                        if a[own]+a['w']==b[own]:left,right=a,b
                        elif b[own]+b['w']==a[own]:left,right=b,a
                        else:continue
                        lo=max(left[other],right[other]);hi=min(left[other]+left['w'],right[other]+right['w'])
                        if hi<=lo:continue
                        p=left['p'][left['p'][:,axis]==left['w']];q=right['p'][right['p'][:,axis]==0];p=p[np.argsort(p[:,1-axis])];q=q[np.argsort(q[:,1-axis])];px=p[:,1-axis]+left[other];qx=q[:,1-axis]+right[other];samples=np.unique(np.r_[px[(px>=lo)&(px<=hi)],qx[(qx>=lo)&(qx<=hi)],lo,hi]);difference=np.max(np.abs(np.interp(samples,px,p[:,2])-np.interp(samples,qx,q[:,2])));assert difference<.01;maximum=max(maximum,float(difference));shared+=1;mixed+=left['w']!=right['w']
            result[area['id']]=dict(tiles=len(entries),sharedEdges=shared,mixedEdges=mixed,maximumBoundaryHeightDifference=maximum,minimumAngle=minimum)
        return result

if __name__=='__main__':
    result=dict(uniform=verify('error-detail20'),mixed=verify_mixed());print(json.dumps(result,ensure_ascii=False));(ROOT/'build/terrain-error-detail20-verification.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')

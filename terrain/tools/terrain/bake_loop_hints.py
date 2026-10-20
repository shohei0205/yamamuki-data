"""Loop細分割後の標高をDEMへ合わせ、元頂点の高さ補正を別SQLiteへ保存する。"""
import argparse, hashlib, json, sqlite3, struct, zlib
from pathlib import Path
import numpy as np
from scipy import sparse
from scipy.sparse.linalg import lsmr
import national_independent_mesh as dem


def operator(n, faces):
    adjacent=[set() for _ in range(n)]; edges={}
    for a,b,c in faces:
        for u,v,o in [(a,b,c),(b,c,a),(c,a,b)]:
            adjacent[u].add(v); adjacent[v].add(u)
            edges.setdefault(tuple(sorted((int(u),int(v)))),[]).append(int(o))
    boundary=set(v for e,opp in edges.items() if len(opp)==1 for v in e)
    rows=[]; cols=[]; data=[]
    def put(r,c,w): rows.append(r); cols.append(c); data.append(w)
    for i,near in enumerate(adjacent):
        if i in boundary or not near: put(i,i,1); continue
        k=len(near); beta=(5/8-(3/8+.25*np.cos(2*np.pi/k))**2)/k
        put(i,i,1-k*beta)
        for j in near: put(i,j,beta)
    ids={}
    for e,opp in edges.items():
        r=n+len(ids); ids[e]=r
        for v in e: put(r,v,.375 if len(opp)==2 else .5)
        if len(opp)==2:
            for v in opp: put(r,v,.125)
    out=[]
    for a,b,c in faces:
        ab=ids[tuple(sorted((a,b)))]; bc=ids[tuple(sorted((b,c)))]; ca=ids[tuple(sorted((c,a)))]
        out.extend([(a,ab,ca),(b,bc,ab),(c,ca,bc),(ab,bc,ca)])
    return sparse.csr_matrix((data,(rows,cols)),shape=(n+len(edges),n)),np.array(out),boundary


def metrics(pred,target,valid):
    e=pred[valid]-target[valid]
    return dict(samples=int(valid.sum()),rmse=float(np.sqrt(np.mean(e*e))),maximum=float(np.max(np.abs(e))))


def bake(rows,db,out,level):
    width=1000*2**level; vertices=[]; faces=[]; lookup={}; maps=[]
    for l,x,y,blob in rows:
        raw=zlib.decompress(blob); magic,_,_,nv,ni=struct.unpack_from('<4sHHII',raw)
        assert magic==b'YNT2'
        p=np.frombuffer(raw,dtype='<f4',count=nv*3,offset=16).reshape(-1,3).astype(float)
        p[:,:2]+=[x*width,y*width]; local=[]
        for v in p:
            key=tuple(np.round(v[:2]*1e5).astype(np.int64))
            if key not in lookup: lookup[key]=len(vertices); vertices.append(v)
            j=lookup[key]; assert abs(vertices[j][2]-v[2])<.02
            local.append(j)
        local=np.array(local); maps.append((l,x,y,blob,local))
        t=np.frombuffer(raw,dtype='<u2',count=ni,offset=16+nv*12).reshape(-1,3)
        faces.extend(local[t])
    p=np.array(vertices); f=np.array(faces); n=len(p)
    adjacent=[set() for _ in range(n)]
    for tri in f:
        for i in range(3):adjacent[tri[i]].update([tri[(i+1)%3],tri[(i+2)%3]])
    heuristic=np.zeros(n)
    for i,near in enumerate(adjacent):
        ids=list(near); k=len(ids)
        if k<3:continue
        beta=(5/8-(3/8+.25*np.cos(2*np.pi/k))**2)/k; gamma=1/(k+3/(8*beta)); diagonal=1-k*gamma
        diff=p[ids]-p[i]; weights=1/np.maximum(1e-20,(diff[:,:2]**2).sum(axis=1))
        designLocal=diff[:,:2]*np.sqrt(weights[:,None]); slope=np.linalg.lstsq(designLocal,diff[:,2]*np.sqrt(weights),rcond=1e-12)[0]
        limit=diagonal*p[i]+gamma*p[ids].sum(axis=0); residual=p[i,2]-limit[2]-slope@(p[i,:2]-limit[:2]); heightRange=np.ptp(p[[i,*ids],2])
        heuristic[i]=np.clip(residual/diagonal,-heightRange*.75,heightRange*.75)*.5
    a,f,boundary=operator(n,f); heuristic[list(boundary)]=0; b,f,_=operator(a.shape[0],f); a=b@a
    xy=a@p[:,:2]; target,valid=dem.source_samples(xy[:,0],xy[:,1]); original=a@p[:,2]
    free=np.array([i for i in range(n) if i not in boundary]); design=a[valid][:,free]
    # 元頂点を大幅に押し上げる過適合を抑え、領域外周の補正はゼロに固定する。
    delta=np.zeros(n); residual=target[valid]-original[valid]
    fit=lsmr(design,residual,damp=.25,atol=1e-7,btol=1e-7,maxiter=150)
    delta[free]=np.clip(fit[0],-min(8191,4*[2,5,10,25,50,100,200,400][level]),min(8191,4*[2,5,10,25,50,100,200,400][level]))
    quant=np.rint(delta*4).astype('<i2'); retention=np.full(n,255,dtype='u1'); retention[list(boundary)]=0
    effective=quant.astype(float)*.25*retention/255
    # フィットに使わない各小三角形の重心で誤差を比較する。
    centroid=xy[f].mean(axis=1); truth,ok=dem.source_samples(centroid[:,0],centroid[:,1])
    before=original[f].mean(axis=1); after=(a@(p[:,2]+effective))[f].mean(axis=1)
    report=dict(level=level,vertices=n,fitBefore=metrics(original,target,valid),fitAfter=metrics(original+a@effective,target,valid),holdoutBefore=metrics(before,truth,ok),holdoutHeuristic=metrics((a@(p[:,2]+heuristic))[f].mean(axis=1),truth,ok),holdoutAfter=metrics(after,truth,ok),maxCorrection=float(abs(effective).max()),iterations=int(fit[2]),bytes=0)
    for l,x,y,blob,local in maps:
        body=struct.pack('<4sIf',b'YLB1',len(local),.25)+hashlib.sha256(zlib.decompress(blob)).digest()+quant[local].tobytes()+retention[local].tobytes()
        packed=zlib.compress(body,9); report['bytes']+=len(packed)
        out.execute('INSERT OR REPLACE INTO tiles VALUES (?,?,?,?,?)',(l,x,y,packed,json.dumps(report)))
    out.commit(); print(json.dumps(report),flush=True); return report


def main():
    p=argparse.ArgumentParser();p.add_argument('--input',default='build/terrain-regional-priority-levels-v5.sqlite3');p.add_argument('--source',default='build/terrain-lod.sqlite3');p.add_argument('--output',default='build/terrain-loop-baked.sqlite3');p.add_argument('--levels',nargs='+',type=int,default=[3,4]);args=p.parse_args()
    dem.initialize(args.source,''); db=sqlite3.connect(Path(args.input).resolve().as_uri()+'?mode=ro',uri=True);out=sqlite3.connect(args.output)
    out.execute('CREATE TABLE IF NOT EXISTS tiles(level INTEGER,x INTEGER,y INTEGER,data BLOB,stats TEXT,PRIMARY KEY(level,x,y))');out.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
    reports=[]
    for level in args.levels:
        rows=db.execute('SELECT level,x,y,geometry FROM tiles WHERE level=? ORDER BY x,y',(level,)).fetchall()
        # 離れた地域は独立した連結成分として同時に解く。共有点は一度だけ計算する。
        reports.append(bake(rows,db,out,level))
    out.execute('INSERT OR REPLACE INTO metadata VALUES (?,?)',('manifest',json.dumps(dict(version=1,fitDivisions=4,source=str(args.input),reports=reports))));out.commit()
    Path(args.output+'.json').write_text(json.dumps(reports,ensure_ascii=False,indent=2),encoding='utf-8')

if __name__=='__main__':main()

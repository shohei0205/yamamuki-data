"""中心の山の近傍について、元標高との誤差を既存・候補メッシュで比較する。"""
import json, sqlite3, struct, zlib
import numpy as np
from compare_priority_from_start import national, raster, ROOT


def local_errors(mode):
    national.initialize(ROOT/'build/terrain-lod.sqlite3','')
    output={}
    with sqlite3.connect(ROOT/f'build/terrain-shared-allocation-{mode}.sqlite3') as db:
        manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        width=manifest['sharedAllocation'].get('widthMeters',32000)
        for area in manifest['reliefRegions']:
            keys=area['levels'][0]['keys'];ox=min(k[1] for k in keys);oy=min(k[2] for k in keys);errors={r:[] for r in [2,5]}
            for key in keys:
                l,x,y=key;cx=(x-ox)*8000;cy=(y-oy)*8000
                if cx>width/2+5000 or cx+8000<width/2-5000 or cy>width/2+5000 or cy+8000<width/2-5000:continue
                raw=zlib.decompress(db.execute('SELECT geometry FROM tiles WHERE level=? AND x=? AND y=?',key).fetchone()[0]);_,n,_,nv,ni=struct.unpack_from('<4sHHII',raw)
                p=np.frombuffer(raw,'<f4',nv*3,16).reshape(-1,3).astype(float);f=np.frombuffer(raw,'<u2',ni,16+nv*12).reshape(-1,3)
                spacing=8000/(n-1);p[:,:2]/=spacing;owner,weights=raster(p,f,n)
                predicted=(p[f[owner],2]*weights).sum(axis=1).reshape(n,n)
                axis=np.arange(n)*spacing;xx,yy=np.meshgrid(axis,axis)
                truth,valid=national.source_samples(x*8000+xx+area['comparisonOffset'][0],y*8000+yy+area['comparisonOffset'][1]);truth=np.rint(truth*4)/4
                r2=(cx+xx-width/2)**2+(cy+yy-width/2)**2
                for radius in errors:errors[radius].extend((predicted-truth)[valid&(r2<=(radius*1000)**2)])
            output[area['id']]={str(radius):dict(samples=len(v),rmse=float(np.sqrt(np.mean(np.square(v)))),maximum=float(np.max(np.abs(v)))) for radius,v in errors.items()}
    return output


if __name__=='__main__':
    import argparse
    parser=argparse.ArgumentParser();parser.add_argument('modes',nargs='+');args=parser.parse_args()
    report={mode:local_errors(mode) for mode in args.modes}
    (ROOT/'build/terrain-mountain-quality-audit.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    print(json.dumps(report))

"""全国独立メッシュのSQLite BLOBと静的ビューアーを中継する。"""
import argparse,json,re,sqlite3
from urllib.parse import parse_qs,urlsplit
from functools import partial
from http.server import ThreadingHTTPServer,SimpleHTTPRequestHandler
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
class Handler(SimpleHTTPRequestHandler):
 database=ROOT/'build/terrain-independent-national.sqlite3'
 comparison=None
 neighbor0=None
 neighbor3=None
 priority1024=None
 ridge1024=None
 ridge=None
 neighbor2=None
 graded=None
 peaks=None
 guard=None
 relative=None
 fixed=None
 def end_headers(self):
  self.send_header('Cache-Control','no-store');super().end_headers()
 def do_GET(self):
  path=self.path.split('?')[0]
  dataset=parse_qs(urlsplit(self.path).query).get('dataset',['original'])[0]
  comparison=self.priority1024 if dataset=='priority1024' else self.ridge1024 if dataset=='ridge1024' else self.ridge if dataset=='ridge' else self.neighbor0 if dataset=='neighbor0' else self.neighbor3 if dataset=='neighbor3' else self.neighbor2 if dataset=='neighbor2' else self.graded if dataset=='graded' else self.peaks if dataset=='peaks' else self.guard if dataset=='guard' else self.relative if dataset=='relative' else self.fixed if dataset=='fixed' else self.comparison if dataset=='joint' else None
  joint=comparison is not None
  if path=='/':self.path='/national_mesh.html';return super().do_GET()
  match=re.fullmatch(r'/tile/([2-7])/(-?\d+)/(-?\d+)/(geometry|hints|mask)',path)
  if path=='/manifest' or match:
   with sqlite3.connect(self.database.resolve().as_uri()+'?mode=ro',uri=True) as db:
    if match:
     row=db.execute(f'SELECT {match[4]} FROM tiles WHERE level=? AND x=? AND y=?',(int(match[1]),int(match[2]),int(match[3]))).fetchone()
     if joint:
      with sqlite3.connect(comparison.resolve().as_uri()+'?mode=ro',uri=True) as extra:
       replacement=extra.execute(f'SELECT {match[4]} FROM tiles WHERE level=? AND x=? AND y=?',(int(match[1]),int(match[2]),int(match[3]))).fetchone()
       if replacement:row=replacement
     if not row:return self.send_error(404)
     body=row[0];kind='application/octet-stream'
    else:
     meta=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);meta['levels']=[s for s in meta['levels'] if s['level']>=2]
     meta['tiles']=db.execute('SELECT level,x,y,length(geometry),length(hints),length(mask) FROM tiles WHERE level BETWEEN 2 AND 7 ORDER BY level,x,y').fetchall()
     if self.comparison or self.relative:
      meta['comparisonAvailable']=True
      if joint:
       with sqlite3.connect(comparison.resolve().as_uri()+'?mode=ro',uri=True) as extra:
        replacements={tuple(r[:3]):r for r in extra.execute('SELECT level,x,y,length(geometry),length(hints),length(mask) FROM tiles')}
        meta['tiles']=[replacements.get(tuple(r[:3]),r) for r in meta['tiles']]
        info=json.loads(extra.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        for field in ['jointMesh','reliefRegions','reliefSettings']:
         if field in info:meta[field]=info[field]
      if self.relative:
       meta['reliefComparison']=True;meta['comparisonRegions']={}
       for mode,file in [('priority1024',self.priority1024),('ridge1024',self.ridge1024),('ridge',self.ridge),('fixed',self.fixed),('relative',self.relative),('guard',self.guard),('peaks',self.peaks),('graded',self.graded),('neighbor0',self.neighbor0),('neighbor3',self.neighbor3),('neighbor2',self.neighbor2)]:
        if file and file.exists():
         with sqlite3.connect(file.resolve().as_uri()+'?mode=ro',uri=True) as region_db:
          value=region_db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
          if value:meta['comparisonRegions'][mode]=[r['id'] for r in json.loads(value[0]).get('reliefRegions',[])]
     body=json.dumps(meta,separators=(',',':')).encode();kind='application/json'
   self.send_response(200);self.send_header('Content-Type',kind);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if path not in ['/national_mesh.html','/national_mesh.js','/national_mesh_worker.js','/pn-terrain.js','/hct-terrain.js','/adaptive-curve.js','/surface-methods.js','/loop-gpu.js','/stitch-terrain.js']:return self.send_error(404)
  super().do_GET()
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=61445);p.add_argument('--priority1024',type=Path);p.add_argument('--ridge1024',type=Path);p.add_argument('--ridge',type=Path);p.add_argument('--neighbor0',type=Path);p.add_argument('--neighbor3',type=Path);p.add_argument('--neighbor2',type=Path);p.add_argument('--graded',type=Path);p.add_argument('--peaks',type=Path);p.add_argument('--guard',type=Path);p.add_argument('--relative',type=Path);p.add_argument('--fixed',type=Path);p.add_argument('--comparison',type=Path);p.add_argument('--database',type=Path,default=Handler.database);a=p.parse_args();Handler.priority1024=a.priority1024;Handler.ridge1024=a.ridge1024;Handler.ridge=a.ridge;Handler.database=a.database;Handler.comparison=a.comparison;Handler.neighbor0=a.neighbor0;Handler.neighbor3=a.neighbor3;Handler.neighbor2=a.neighbor2;Handler.graded=a.graded;Handler.peaks=a.peaks;Handler.guard=a.guard;Handler.relative=a.relative;Handler.fixed=a.fixed
 ThreadingHTTPServer(('127.0.0.1',a.port),partial(Handler,directory=str(Path(__file__).parent))).serve_forever()

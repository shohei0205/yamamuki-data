"""地域全レベル比較のSQLite BLOBとHTMLを配信する。"""
import argparse,json,sqlite3,re
from urllib.parse import urlsplit,parse_qs
from http.server import ThreadingHTTPServer,SimpleHTTPRequestHandler
from functools import partial
from pathlib import Path
from regional_priority_levels import OUTPUT,ROOT
BUDGET_PREFIX='terrain-priority-budget'
DEFAULT_INPUT=next((p for p in [ROOT/'build/terrain-regional-priority-levels-v5.sqlite3',ROOT/'build/terrain-regional-priority-levels-v4.sqlite3'] if p.exists()),OUTPUT)

def available_budgets():
 result=[]
 for value in [1000,2000,4000,8000,16000,32000,64000]:
  path=ROOT/f'build/{BUDGET_PREFIX}-{value}.sqlite3'
  if not path.exists():continue
  with sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True) as db:
   if db.execute('SELECT 1 FROM metadata WHERE key="manifest"').fetchone():result.append(value)
 return result

def available_allocations():
 result=[]
 for mode in ['error-detail20-direct-e12.5','error-detail20-direct-e12.5-mixed','error-detail20-direct','error-detail20-direct-mixed','error-detail20-cutoff','error-detail20-cutoff-mixed','error-detail20-cut10','error-detail20-cut10-mixed','error-detail20-shape','error-detail20-shape-mixed','error-detail20-perceptual','error-detail20-perceptual-mixed','error-detail20','error-detail20-mixed',*[f'error-detail20{variant}-e{e}{suffix}' for variant in ['', '-perceptual','-cutoff','-direct'] for e in [50,100,200,400,800] for suffix in ['', '-mixed']],'global-pool128-unlimited','global-pool128','local-fixed128','local-share128','local-fixed128-shift','local-share128-shift','relative','balanced','terrain','terrain128','shape128','landform','landform128','landform-quality128','landform-quality128-b12000-graded',*[f'landform-quality128-b{n}-capped' for n in [12000,24000,48000,96000,192000,384000,768000]],*[f'landform-quality128-b{n}' for n in [12000,24000,48000,96000,384000,768000]],'terrain128-b48000','terrain128-b96000','terrain128-b384000','terrain128-b768000']:
  path=ROOT/f'build/terrain-shared-allocation-{mode}.sqlite3'
  if not path.exists():continue
  try:
   with sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True,timeout=.1) as db:
    row=db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
    if mode.startswith(('local-','global-pool','error-detail20')) and (not row or not json.loads(row[0])['sharedAllocation'].get('generationComplete')):continue
    if mode.startswith('error-detail20') and (not row or json.loads(row[0])['sharedAllocation'].get('mode')!=mode):continue
    if mode.endswith('-capped') and (not row or not json.loads(row[0])['sharedAllocation'].get('generationComplete')):continue
    if row and ('-b' not in mode or json.loads(row[0])['sharedAllocation']['sourceVertices']<=int(mode.split('-b')[1].split('-')[0])):result.append(mode)
  except sqlite3.OperationalError as error:
   if not any(word in str(error) for word in ['locked','no such table']):raise
 return result

class Handler(SimpleHTTPRequestHandler):
 def end_headers(self):
  self.send_header('Cache-Control','no-store');super().end_headers()
 def do_GET(self):
  path=self.path.split('?')[0]
  comparison=re.fullmatch(r'/landcover-compare/(report\.json|(?:fuji|tsukuba|yari)-(?:512|1024|2048)-[a-z0-9.]+\.png)',path)
  if comparison:
   file=ROOT/'build/landcover-compare'/comparison[1]
   if not file.is_file():return self.send_error(404)
   body=file.read_bytes();self.send_response(200);self.send_header('Content-Type','application/json' if file.suffix=='.json' else 'image/png');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  dataset=parse_qs(urlsplit(self.path).query).get('dataset',[''])[0]
  budget=int(dataset[6:]) if re.fullmatch(r'budget(1000|2000|4000|8000|16000|32000|64000)',dataset) else None
  allocation=dataset[11:] if re.fullmatch(r'allocation-(error-detail20(?:-perceptual|-shape|-cut10|-cutoff|-direct)?(?:-e(?:12\.5|50|100|200|400|800))?(?:-mixed)?|global-pool128(?:-unlimited)?|local-fixed128(?:-shift)?|local-share128(?:-shift)?|relative|balanced|terrain|shape128|landform|landform128|landform-quality128(?:-b(?:12000(?:-graded)?|24000|48000|96000|192000|384000|768000)(?:-capped)?)?|terrain128(?:-b(?:48000|96000|384000|768000))?)',dataset) else None
  control=dataset=='control';priority=dataset in ['staged','unified'] or budget is not None or allocation is not None
  input_path=ROOT/f'build/terrain-shared-allocation-{allocation}.sqlite3' if allocation else ROOT/f'build/{BUDGET_PREFIX}-{budget}.sqlite3' if budget else ROOT/f'build/terrain-priority-{dataset}-compare.sqlite3' if priority else ROOT/'build/terrain-loop-control.sqlite3' if control else OUTPUT
  astc=re.fullmatch(r'/landcolor-astc/(fuji|tsukuba|yari)/(128|256|512|1024|2048|4096)/(astc8|astc8preview)',path)
  if astc:
   sidecar=ROOT/'build/terrain-landcover-astc.sqlite3'
   if not sidecar.exists():return self.send_error(404)
   with sqlite3.connect(sidecar.resolve().as_uri()+'?mode=ro',uri=True) as images:
    row=images.execute('SELECT data FROM images WHERE region=? AND size=? AND codec=?',(astc[1],int(astc[2]),astc[3])).fetchone()
    encoded=images.execute('SELECT length(data) FROM images WHERE region=? AND size=? AND codec="astc8"',(astc[1],int(astc[2]))).fetchone()
   if not row or not encoded:return self.send_error(404)
   body=row[0];self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('X-ASTC-Bytes',str(encoded[0]));self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  landcolor=re.fullmatch(r'/landcolor/(fuji|tsukuba|yari)/(128|256|512|1024|2048|4096)\.png',path)
  landcolor_path=ROOT/'build/terrain-landcover-color.sqlite3'
  if not landcolor_path.exists():landcolor_path=ROOT/'build/terrain-landcover-color.sqlite3'
  if landcolor:
   if not landcolor_path.exists():return self.send_error(404)
   with sqlite3.connect(landcolor_path.resolve().as_uri()+'?mode=ro',uri=True) as images:
    row=images.execute('SELECT data FROM images WHERE region=? AND size=?',(landcolor[1],int(landcolor[2]))).fetchone()
   if not row:return self.send_error(404)
   body=row[0];self.send_response(200);self.send_header('Content-Type','image/png');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  relief=re.fullmatch(r'/relief/(fuji|tsukuba|yari)/(512|1024|2048|4096)\.png',path)
  relief_path=ROOT/'build/terrain-baked-relief.sqlite3'
  if relief:
   if not relief_path.exists():return self.send_error(404)
   with sqlite3.connect(relief_path.resolve().as_uri()+'?mode=ro',uri=True) as images:
    row=images.execute('SELECT data FROM images WHERE region=? AND size=?',(relief[1],int(relief[2]))).fetchone()
   if not row:return self.send_error(404)
   body=row[0];self.send_response(200);self.send_header('Content-Type','image/png');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if path=='/':self.path='/national_mesh.html';return super().do_GET()
  match=re.fullmatch(r'/tile/([0-7])/(-?\d+)/(-?\d+)/(geometry|hints|mask|topology)',path)
  baked=re.fullmatch(r'/tile/([0-7])/(-?\d+)/(-?\d+)/loop',path)
  sidecar=ROOT/'build/terrain-loop-baked.sqlite3'
  if baked:
   if control or priority:return self.send_error(400,explain='比較メッシュには元メッシュ用のベイク補正を適用できません')
   if not sidecar.exists():return self.send_error(404)
   with sqlite3.connect(sidecar.resolve().as_uri()+'?mode=ro',uri=True) as hints:
    row=hints.execute('SELECT data FROM tiles WHERE level=? AND x=? AND y=?',tuple(map(int,baked.groups()))).fetchone()
   if not row:return self.send_error(404,explain='このレベルのLoop補助データは未生成です')
   body=row[0];self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if path=='/manifest' or match:
   with sqlite3.connect(input_path.resolve().as_uri()+'?mode=ro',uri=True) as db:
    if match:
     row=db.execute('SELECT data FROM topology WHERE level=? AND x=? AND y=?' if match[4]=='topology' else f'SELECT {match[4]} FROM tiles WHERE level=? AND x=? AND y=?',tuple(map(int,match.groups()[:3]))).fetchone()
     if not row:return self.send_error(404)
     body=row[0];mime='application/octet-stream'
    else:
     row=db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
     if not row:return self.send_error(503,explain='比較データを生成中です')
     manifest=json.loads(row[0]);manifest['coastlineInMesh']=False;manifest['missingElevationPlaceholderMeters']=0;manifest['vertexBudgetComparisons']=available_budgets();manifest['sharedAllocationComparisons']=available_allocations();manifest['priorityComparisonAvailable']=(ROOT/'build/terrain-priority-comparison.ready').exists();manifest['controlComparisonAvailable']=(ROOT/'build/terrain-loop-control.sqlite3').exists();manifest['tiles']=db.execute('SELECT level,x,y,length(geometry),length(hints),length(mask) FROM tiles ORDER BY level,x,y').fetchall()
     if manifest.get('sharedAllocation',{}).get('topology')=='uncut-halo-v3':
      lengths={tuple(r[:3]):r[3] for r in db.execute('SELECT level,x,y,length(data) FROM topology')}
      manifest['tiles']=[[*r[:3],lengths[tuple(r[:3])],0,r[5]] for r in manifest['tiles']]
     if relief_path.exists() and allocation and allocation.startswith('error-detail20'):
      with sqlite3.connect(relief_path.resolve().as_uri()+'?mode=ro',uri=True) as images:
       manifest['bakedRelief']=dict(widthMeters=128000,format='grayscale8-png',images=[dict(region=r,size=n,bytes=b) for r,n,b in images.execute('SELECT region,size,length(data) FROM images ORDER BY region,size')])
     if landcolor_path.exists() and allocation and allocation.startswith('error-detail20'):
      with sqlite3.connect(landcolor_path.resolve().as_uri()+'?mode=ro',uri=True) as images:
       meta=images.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
       if meta:
        details=json.loads(meta[0]);manifest['landcoverColor']=dict(widthMeters=128000,format=details['format'],attribution=details['attribution'],images=[dict(region=r,size=n,bytes=b) for r,n,b in images.execute('SELECT region,size,length(data) FROM images ORDER BY region,size')])
     sizes={}
     if sidecar.exists() and not control and not priority:
      with sqlite3.connect(sidecar.resolve().as_uri()+'?mode=ro',uri=True) as hints:sizes={tuple(r[:3]):r[3] for r in hints.execute('SELECT level,x,y,length(data) FROM tiles')}
     manifest['tiles']=[[*r,sizes.get(tuple(r[:3]),0)] for r in manifest['tiles']];body=json.dumps(manifest,separators=(',',':')).encode();mime='application/json'
   self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if path not in ['/landcover_compare.html','/neighbor_budget.html','/national_mesh.html','/national_mesh.js','/landcover-astc.js','/national_mesh_worker.js','/pn-terrain.js','/hct-terrain.js','/adaptive-curve.js','/surface-methods.js','/loop-gpu.js','/stitch-terrain.js']:return self.send_error(404)
  super().do_GET()

if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=61448);p.add_argument('--input',type=Path,default=DEFAULT_INPUT);p.add_argument('--centered-budgets',action='store_true');a=p.parse_args();OUTPUT=a.input;BUDGET_PREFIX='terrain-priority-centered-budget' if a.centered_budgets else 'terrain-priority-budget';ThreadingHTTPServer(('127.0.0.1',a.port),partial(Handler,directory=str(ROOT/'tools/terrain'))).serve_forever()

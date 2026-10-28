"""GPUへ制御点・Voronoiセル・疎な候補探索木を送り、高密度メッシュは送らない。"""
import json,struct,time
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant
from sibson_aux_surface import AuxiliarySibsonSurface
ROOT=Path('build/sibson-weighted-v1')

def build(region,error,bounds=(-4000.,-4000.,4000.,4000.)):
 started=time.perf_counter();folder=ROOT/f'{region}-{error:g}';points=decode_quant((folder/'original.mlq1').read_bytes()).astype(float)
 with np.load(folder/'natural-global-c1.cache.npz') as cached:
  if not np.array_equal(cached['points'],points):raise ValueError('勾配キャッシュと制御点が一致しません')
  gradient=cached['gradient'].copy()
 return pack(points,gradient,bounds,dict(region=region,error=error),started=started)

def pack(points,gradient,bounds,metadata,started=None,model=None,max_candidates=20,max_depth=11):
 if started is None:started=time.perf_counter()
 if model is None:model=AuxiliarySibsonSurface(points)
 records=np.zeros((len(points),8),'<u4');floats=records.view('<f4');floats[:,:2]=points[:,:2]/1000;floats[:,2]=points[:,2];records[:,3]=model.cell_off[:-1];floats[:,4:6]=gradient;records[:,6]=np.diff(model.cell_off);records[:,7]=(records[:,6]==0);cells=(model.cells/1000).astype('<f4');nodes=[];references=[];leaf_counts=[];unsafe=0
 def intersect(ids,rect):
  c=model.circles[ids];nearest=np.clip(c[:,:2],rect[:2]-.01,rect[2:]+.01);keep=np.sum((nearest-c[:,:2])**2,axis=1)<=c[:,2]*(1+1e-12)+1e-7;return ids[keep]
 def visit(ids,rect,depth):
  nonlocal unsafe
  ids=intersect(ids,rect);candidate=np.unique(model.tri[ids]);index=len(nodes);nodes.append(np.full(8,0xffffffff,'<u4'))
  if len(candidate)>max_candidates and (depth<max_depth or len(candidate)>256) and depth<16:
   x0,y0,x1,y1=rect;mx=(x0+x1)/2;my=(y0+y1)/2
   rectangles=[np.array(r) for r in [(x0,y0,mx,my),(mx,y0,x1,my),(x0,my,mx,y1),(mx,my,x1,y1)]]
   children=[intersect(ids,r) for r in rectangles]
   # 全ての子で候補が同じなら、分割しても候補を減らせない。
   # GPUの256候補上限内ではそのまま葉にして、指数的な木の膨張を防ぐ。
   stable=len(candidate)<=256 and all(np.array_equal(np.unique(model.tri[c]),candidate) for c in children)
   if not stable:
    for i,r in enumerate(rectangles):nodes[index][i]=visit(children[i],r,depth+1)
    return index
  if nodes[index][0]==0xffffffff:
   if len(candidate)>256:raise ValueError('候補数がGPU上限256を超えました。探索木を細かくしてください')
   nodes[index][4]=len(references);nodes[index][5]=len(candidate);nodes[index][6]=0;nodes[index][7]=0;references.extend(candidate.tolist());leaf_counts.append(len(candidate));unsafe+=int(np.any(records[candidate,6]==0))
  return index
 index_bounds=np.asarray(bounds)+[-2,-2,2,2];visit(np.arange(len(model.tri)),index_bounds,0);node=np.asarray(nodes,'<u4');ref=np.asarray(references,'<u4');used=np.unique(ref);remap=np.full(len(points),0xffffffff,'<u4');remap[used]=np.arange(len(used));ref=remap[ref];compact=records[used].copy();compact_cells=[];offset=0
 for i,old in enumerate(used):
  begin,count=int(records[old,3]),int(records[old,6]);compact[i,3]=offset;compact_cells.append(cells[begin:begin+count]);offset+=count
 records=compact;cells=np.concatenate(compact_cells).astype('<f4');info=dict(**metadata,bounds=list(bounds),indexBounds=index_bounds.tolist(),coordinateScale=1000,controls=len(used),sourceControls=len(points),pointBytes=records.nbytes,cellBytes=cells.nbytes,nodeBytes=node.nbytes,referenceBytes=ref.nbytes,nodeCount=len(nodes),leafCount=len(leaf_counts),maxCandidates=max(leaf_counts),maxCellVertices=int(np.diff(model.cell_off).max()),unsafeLeaves=unsafe,buildSeconds=time.perf_counter()-started,format='Sibson-GPU-controls-v1',noHeightmap=True,noTriangleMesh=True)
 header=json.dumps(info).encode();header+=b' '*((-len(header))%4);packet=struct.pack('<I',len(header))+header+records.tobytes()+cells.tobytes()+node.tobytes()+ref.tobytes();return packet,info

if __name__=='__main__':
 folder=ROOT/'gpu-controls';folder.mkdir(exist_ok=True)
 for region in ['fuji','yari','tsukuba']:
  for error in [25,50]:
   packet,info=build(region,error);(folder/f'{region}-{error}.bin').write_bytes(packet);(folder/f'{region}-{error}.json').write_text(json.dumps(info,ensure_ascii=False,indent=2),encoding='utf-8');print(info,flush=True)

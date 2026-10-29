"""粗い描画格子に必要な接合帯まで、保存制御点の曲面評価範囲を広げる。"""
from pathlib import Path
import json,struct,tempfile
import numpy as np
from sibson_aux_surface import AuxiliarySibsonSurface
from sibson_gpu_variable import close_cells
from sibson_gpu_controls import pack
COMPUTED={'bounds','indexBounds','coordinateScale','controls','sourceControls','pointBytes','cellBytes','nodeBytes','referenceBytes','nodeCount','leafCount','maxCandidates','maxCellVertices','unsafeLeaves','buildSeconds','format','noHeightmap','noTriangleMesh'}
def packet_with_padding(folder,info,n):
 side=info['side'];padding=min(float(info['halo']),max(250.,2*side/n))
 if padding<=250:return (folder/'packet.bin').read_bytes()
 source=folder/'control-gradient.npz';cache=folder/f'display-padding-v1-{padding:g}-{source.stat().st_mtime_ns}.bin'
 if cache.exists():return cache.read_bytes()
 with np.load(source) as data:points=data['points'];gradient=data['gradient']
 model=AuxiliarySibsonSurface(points);close_cells(model);bounds=np.asarray([-side/2-padding,-side/2-padding,side/2+padding,side/2+padding]);metadata={k:v for k,v in info.items() if k not in COMPUTED};metadata.update(worldFieldBounds=(bounds+np.tile(info['origin'],2)).tolist(),displayPaddingMeters=padding,displayPaddingOutsideOriginalAudit=padding>250)
 packet,out=pack(points,gradient,bounds,metadata,model=model,max_candidates=48,max_depth=11)
 if out['unsafeLeaves']:raise ValueError('expanded seam packet has unsafe cells')
 temporary=None
 try:
  with tempfile.NamedTemporaryFile(dir=folder,suffix='.tmp',delete=False) as f:f.write(packet);temporary=Path(f.name)
  temporary.replace(cache)
 finally:
  if temporary is not None:temporary.unlink(missing_ok=True)
 return packet

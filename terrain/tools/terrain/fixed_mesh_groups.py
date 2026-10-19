"""階層タイルのメッシュ結合と余白付き法線アトラスを作る。"""
from io import BytesIO
import struct
import zlib

import numpy as np
from PIL import Image



def group_mesh(body, gx, gy, extent=24000, size=10, total=100):
    raw = zlib.decompress(body)
    nv, ni = struct.unpack_from('<II', raw)
    vertices = np.frombuffer(raw, '<f4', nv*3, 8).reshape(-1, 3)
    indices = np.frombuffer(raw, '<u4', ni, 8+nv*12)
    points, faces, uvs = [], [], []
    for y in range(min(size, total-gy*size)):
        for x in range(min(size, total-gx*size)):
            copied = vertices.copy()
            copied[:, 0] += (gx*size+x-(total-1)/2)*extent
            copied[:, 1] += (gy*size+y-(total-1)/2)*extent
            points.append(copied)
            faces.append(indices+(y*min(size,total-gx*size)+x)*nv)
            uv = np.column_stack([vertices[:, 0]/extent+.5, .5-vertices[:, 1]/extent])
            # 2画素の余白を持つ法線アトラス。頂点は元のタイル内UVも保存する。
            uvs.append(np.column_stack([uv, np.full(nv, x), np.full(nv, y)]).astype('<f4'))
    return zlib.compress(struct.pack('<II', len(points)*nv, len(points)*ni)+np.concatenate(points).astype('<f4').tobytes()+
                         np.concatenate(faces).astype('<u4').tobytes()+np.concatenate(uvs).tobytes(), 6)


def atlas_png(body, columns=10, rows=10):
    source=Image.open(BytesIO(body))
    image=np.asarray(source if source.mode=='L' else source.convert('RGB'))
    padded=np.pad(image, ((2,2),(2,2)) if image.ndim==2 else ((2,2),(2,2),(0,0)),mode='edge')
    atlas=np.tile(padded,(rows,columns) if image.ndim==2 else (rows,columns,1))
    output = BytesIO()
    Image.fromarray(atlas).save(output, format='PNG', optimize=True)
    return output.getvalue()

"""描画用配列を little-endian バイナリで転送する。保存形式は変えない。"""
import json
import struct
import numpy as np


def encode_payload(payload):
    metadata=dict(payload)
    blocks=[]
    descriptors=[]
    def array(path,values,dtype):
        data=np.asarray(values,dtype=dtype)
        descriptors.append({'path':path,'count':int(data.size),'type':dtype})
        blocks.append(data.tobytes())
    for key,dtype in [('mesh','<f4'),('indices','<u4'),('landIndices','<u4')]:
        if key in metadata:array([key],metadata.pop(key),dtype)
    if 'overlays' in metadata:
        metadata['overlays']=dict(metadata['overlays'])
        metadata['overlays']['layers']={k:dict(v) for k,v in metadata['overlays']['layers'].items()}
        for key,layer in metadata['overlays']['layers'].items():
            for name in ('triangles','lines'):
                array(['overlays','layers',key,name],layer.pop(name),'<f4')
    header=json.dumps({'metadata':metadata,'arrays':descriptors},ensure_ascii=False,allow_nan=False,separators=(',',':')).encode('utf-8')
    padding=b' ' * (-len(header)%4)
    return struct.pack('<4sI',b'YTB1',len(header)+len(padding))+header+padding+b''.join(blocks)

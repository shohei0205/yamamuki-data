"""ブラウザーの JPEG XL 読み出し結果を djxl の 16 bit 復号と比較する。"""
from pathlib import Path
import json
import numpy as np
from PIL import Image

def compare(root):
 folder=root/'build/native-jxl-probe'
 data=json.loads((folder/'browser.json').read_text(encoding='utf-8'))
 meta=json.loads((folder/'meta.json').read_text(encoding='utf-8'))
 ref=np.asarray(Image.open(folder/'sample.pgm'),dtype=np.float64)
 report=dict(browser=data['ua'],nativeDecoder=data['decoder'],error=data.get('error'),imageDecoderFrame=data.get('imageDecoderFrame'),results={})
 for key,value in data.items():
  if not isinstance(value,dict) or 'pixels' not in value:continue
  q=np.asarray(value['pixels'],dtype=np.float64).reshape(256,256)
  q=q*(65535/255 if value['type']=='Uint8ClampedArray' else 65535)
  delta=(q-ref)*meta['scale']
  report['results'][key]=dict(uniqueValues=len(np.unique(q)),maxMeters=float(abs(delta).max()),rmseMeters=float(np.sqrt(np.mean(delta*delta))))
 (folder/'precision.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
 print(json.dumps(report,ensure_ascii=False,indent=2))
if __name__=='__main__':compare(Path(__file__).resolve().parents[2])

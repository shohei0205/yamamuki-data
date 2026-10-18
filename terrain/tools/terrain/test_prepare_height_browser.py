from contextlib import closing
"""表示用 PNG が 16 bit 標高を保つことを確認する。"""
import json,sqlite3,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
import numpy as np
from PIL import Image
from prepare_height_browser import prepare
class PrepareTest(unittest.TestCase):
 def test_exact_sixteen_bit_channels_and_mask(self):
  with tempfile.TemporaryDirectory(dir='build') as folder:
   root=Path(folder);db=root/'input.sqlite3';q=np.arange(65536,dtype=np.uint16).reshape(256,256)
   with closing(sqlite3.connect(db)) as c:
    c.execute('CREATE TABLE metadata(key,value)');c.execute('INSERT INTO metadata VALUES(?,?)',('config',json.dumps({'normals':False,'heightDistance':.2})))
    c.execute('CREATE TABLE tiles(z,x,y,height_min,height_scale,height_jxl,valid_mask_zstd)');c.execute('INSERT INTO tiles VALUES(7,1,2,-100,.1,?,?)',(b'jxl',b'mask'));c.commit()
   def decode(command,**kwargs):Image.fromarray(q).save(command[2])
   with patch('prepare_height_browser.subprocess.run',side_effect=decode):prepare(db,root/'out',Path('djxl'))
   rgb=np.array(Image.open(root/'out/7/1/2.png'));actual=rgb[:,:,0].astype(np.uint16)+rgb[:,:,1].astype(np.uint16)*256;np.testing.assert_array_equal(actual,q)
   self.assertEqual((root/'out/7/1/2.mask').read_bytes(),b'mask');m=json.loads((root/'out/manifest.json').read_text());self.assertEqual(m['tiles'][0][3:5],[-100,.1])
   with patch('prepare_height_browser.subprocess.run') as run:prepare(db,root/'out',Path('djxl'));run.assert_not_called()
if __name__=='__main__':unittest.main()

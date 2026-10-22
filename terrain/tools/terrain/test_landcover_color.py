"""土地被覆の配色と、生成画像の位置・ライセンス情報を確認する。"""
import io,json,sqlite3,unittest
import numpy as np
from PIL import Image
from generate_landcover_color import colorize,PALETTE,OUTPUT
from national_independent_mesh import project
class ColorTest(unittest.TestCase):
 def test_colors_depend_only_on_class(self):
  classes=np.array([[10,40,50,60,80,255],[10,40,50,60,80,255]],np.uint8);pixels=colorize(classes)
  np.testing.assert_array_equal(pixels[0],pixels[1]);self.assertEqual(tuple(pixels[0,0]),PALETTE[10]);self.assertEqual(tuple(pixels[0,-1]),PALETTE[0]);self.assertGreater(int(pixels[0,0,1]),int(pixels[0,0,0]))
 def test_generated_metadata_and_images(self):
  with sqlite3.connect(OUTPUT) as db:
   meta=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);self.assertFalse(meta['shadeIncluded']);self.assertEqual(meta['license'],'CC BY 4.0');self.assertEqual({(r['region'],r['size']) for r in meta['images']},{(region,size) for region in ['fuji','tsukuba','yari'] for size in [128,256,512,1024,2048,4096]})
   for row in meta['images']:
    image=Image.open(io.BytesIO(db.execute('SELECT data FROM images WHERE region=? AND size=?',(row['region'],row['size'])).fetchone()[0]));self.assertEqual(image.mode,'P');self.assertEqual(image.size,(row['size'],row['size']));self.assertNotIn('0',row['classes'])
 def test_known_summits_match_landcover_location(self):
  with sqlite3.connect(OUTPUT) as db,sqlite3.connect(OUTPUT.parent/'terrain-shared-allocation-error-detail20-direct-e800.sqlite3') as mesh:
   m=json.loads(mesh.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);meta=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
   for region,lon,lat,code in [('fuji',138.7274,35.3606,60),('tsukuba',140.1069,36.2254,10)]:
    area=next(a for a in m['reliefRegions'] if a['id']==region);row=next(a for a in meta['images'] if a['region']==region and a['size']==4096);x,y=project(lon,lat);u=round((x-area['comparisonOffset'][0]-row['origin'][0])/128000*4095);v=round((y-area['comparisonOffset'][1]-row['origin'][1])/128000*4095)
    image=Image.open(io.BytesIO(db.execute('SELECT data FROM images WHERE region=? AND size=4096',(region,)).fetchone()[0])).convert('RGB');self.assertEqual(image.getpixel((u,v)),PALETTE[code])
if __name__=='__main__':unittest.main()

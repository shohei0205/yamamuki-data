"""PNG の標高値・欠損・法線が読み戻せることを確認する。"""
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest

import numpy as np
from PIL import Image
from compact_dem import encode
from heightmap_export import export
from heightmap_levels import build


class HeightmapPNGTest(unittest.TestCase):
    def test_height_and_missing_samples_round_trip(self):
        with tempfile.TemporaryDirectory(dir=Path(__file__).resolve().parents[2]/'build') as folder:
            root=Path(folder);source=root/'source.sqlite3';output=root/'png'
            heights=np.full((256,256),-25.,np.float32)
            heights[100:120,100:120]=3050
            heights[0,0]=np.nan
            with sqlite3.connect(source) as db:
                db.execute('CREATE TABLE tiles(z,x,y,data)')
                db.execute('INSERT INTO tiles VALUES(14,14456,6412,?)',(encode(heights,5,'gzip'),))
            db.close()
            export(source,output)
            meta=json.loads((output/'manifest.json').read_text(encoding='utf-8'))
            pixels=np.asarray(Image.open(output/'height.png'))
            restored=pixels[:,:,0].astype(np.int32)+256*pixels[:,:,1].astype(np.int32)+meta['heightOffset']
            valid=np.isfinite(heights)
            np.testing.assert_array_equal(pixels[:,:,2]>0,valid)
            np.testing.assert_array_equal(restored[valid],heights[valid])
            self.assertEqual(meta['pngMaxHeightErrorMeters'],0)
            with Image.open(output/'normal.png') as image:
                self.assertEqual(image.size,(128,128))
            with Image.open(output/'normal-coarse.png') as image:
                self.assertEqual(image.size,(64,64))
            normal=np.asarray(Image.open(output/'normal.png'))
            np.testing.assert_array_equal(normal[10,10,:2],[128,128])

    def test_levels_are_built_from_original_and_keep_extent(self):
        with tempfile.TemporaryDirectory(dir=Path(__file__).resolve().parents[2]/'build') as folder:
            root=Path(folder);source=root/'source.sqlite3';output=root/'png'
            heights=(np.indices((256,256)).sum(axis=0)*5).astype(np.float32)
            with sqlite3.connect(source) as db:
                db.execute('CREATE TABLE tiles(z,x,y,data)')
                db.execute('INSERT INTO tiles VALUES(14,14456,6412,?)',(encode(heights,5,'gzip'),))
            db.close();export(source,output);report=build(output)
            self.assertEqual([m['level'] for m in report['levels']],list(range(14,3,-1)))
            self.assertEqual([m['size'] for m in report['normalVariants']],[128,64,32,16,8,4])
            for variant in report['normalVariants']:
                with Image.open(output/'normals'/f"{variant['size']}.png") as image:
                    self.assertEqual(image.size,(variant['size'],variant['size']))
            extent=report['levels'][0]['extentMetersX']
            for item in report['levels']:
                self.assertEqual(item['sourceLevel'],14)
                self.assertAlmostEqual(item['meters']*(item['width']-1),extent)
            # L12 の高さを、元配列の 4×4 点の面積平均と直接比較する。
            with Image.open(output/'levels/12/height.png') as image:
                p=np.asarray(image)
            restored=p[:,:,0].astype(np.int32)+256*p[:,:,1].astype(np.int32)
            expected=np.rint(heights.reshape(64,4,64,4).mean(axis=(1,3)))
            np.testing.assert_array_equal(restored,expected)


if __name__=='__main__':
    unittest.main()

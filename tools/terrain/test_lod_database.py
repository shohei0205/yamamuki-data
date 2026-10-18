"""縮尺ごとの平均化、欠損保持、SQLite の読み込みを確認する。"""
from pathlib import Path
from contextlib import closing
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
from compact_dem import encode,decode
from lod_database import reduce_tile,build
from region_server import choose_zoom,region_payload


class LodTests(unittest.TestCase):
    def test_reduction_handles_missing_without_zero_fill(self):
        values=np.full((256,256),np.nan,dtype=np.float32)
        values[:2,:2]=[[100,200],[np.nan,300]]
        reduced=reduce_tile(values)
        self.assertEqual(reduced[0,0],200)
        self.assertTrue(np.isnan(reduced[0,1]))

    def test_database_keeps_detail_and_overview(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);source=root/'source';database=root/'terrain.sqlite3'
            for x,y,h in [(2,2,100),(3,2,200),(3,3,300)]:
                path=source/'14'/str(x)/f'{y}.ymd';path.parent.mkdir(parents=True,exist_ok=True)
                path.write_bytes(encode(np.full((256,256),h),5,'gzip'))
            with patch('builtins.print'):build(source,database,13,2)
            with closing(sqlite3.connect(database)) as db:
                self.assertEqual(db.execute('SELECT z,count(*) FROM tiles GROUP BY z ORDER BY z').fetchall(),[(13,1),(14,3)])
                body=db.execute('SELECT data FROM tiles WHERE z=13 AND x=1 AND y=1').fetchone()[0]
                parent=decode(body)
                self.assertEqual(parent[0,0],100)
                self.assertEqual(parent[0,-1],200)
                self.assertTrue(np.isnan(parent[-1,0]))
                self.assertEqual(parent[-1,-1],300)
            self.assertTrue(Path(str(database)+'.gz').exists())
            with patch('region_server.choose_zoom',return_value=14),patch('region_server.tile_position',side_effect=[(2.99,2.99),(3.01,3.01)]):
                payload=region_payload(root/'no-png',35,138,4,database=database)
            self.assertEqual(payload['coverage']['savedTiles'],3)
            self.assertEqual(payload['coverage']['missingTiles'],1)
            self.assertEqual(payload['coverage']['zoom'],14)

    def test_level_changes_with_display_range(self):
        levels=[choose_zoom(35,138,size) for size in [2,8,100,500,3600]]
        self.assertEqual(levels[0],14)
        self.assertLessEqual(levels[-1],5)
        self.assertEqual(levels,sorted(levels,reverse=True))


if __name__=='__main__':unittest.main()

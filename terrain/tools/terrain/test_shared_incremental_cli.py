"""実際のRustコマンドとの継続生成・終了処理の確認。"""
import json, tempfile, unittest
from contextlib import ExitStack
from pathlib import Path
import numpy as np
from PIL import Image
import compare_shared_allocation as allocation


class IncrementalCliTests(unittest.TestCase):
    def test_incremental_matches_fresh_and_exits(self):
        allocation.compare.national.BINARY = str(allocation.ROOT / 'build/terrain-rust-incremental-target/release/terrain-adaptive-tin.exe')
        with tempfile.TemporaryDirectory(dir=allocation.ROOT/'build', prefix='incremental-check-') as tmp, ExitStack() as stack:
            root = Path(tmp); yy, xx = np.mgrid[:65, :65]
            grid = np.rint((300 + np.sin(xx*.23)*np.cos(yy*.17)*200)*4)/4
            limits = np.full_like(grid, 10.)
            def region(name, incremental):
                folder=root/name;folder.mkdir()
                Image.fromarray((grid*4).astype('uint16')).save(folder/'height.png')
                Image.fromarray((limits*100).astype('uint16')).save(folder/'balanced.png')
                return dict(folder=folder, height_offset=0., seed=[[0,0],[64,0],[64,64],[0,64]], grid=grid, limits=limits, valid=np.ones_like(grid,dtype=bool), incremental=incremental, maximum_budget=1024, stack=stack)
            fresh=allocation.grow(region('fresh',False),'balanced',512)
            r=region('continued',True)
            first=allocation.grow(r,'balanced',128)
            next_mesh=allocation.grow(r,'balanced',512,first)
            np.testing.assert_array_equal(fresh['p'],next_mesh['p'])
            np.testing.assert_array_equal(fresh['f'],next_mesh['f'])
            np.testing.assert_array_equal(first['p'],next_mesh['p'][:len(first['p'])])
            process=r['process']
        self.assertEqual(process.returncode,0)

    def test_fill_budget_continues_after_tolerance(self):
        import subprocess, json
        binary=allocation.ROOT/'build/terrain-rust-incremental-target/release/terrain-adaptive-tin.exe'
        with tempfile.TemporaryDirectory(dir=allocation.ROOT/'build') as tmp:
            root=Path(tmp);yy,xx=np.mgrid[:33,:33]
            Image.fromarray((1000+100*np.sin(xx*.2)*np.cos(yy*.2)).astype('uint16')).save(root/'h.png')
            Image.fromarray(np.full((33,33),60000,dtype='uint16')).save(root/'e.png')
            command=[str(binary),'--input',str(root/'h.png'),'--output',str(root/'m.json'),'--error-limits',str(root/'e.png'),'--incremental','--max-vertices','256']
            subprocess.run(command,input='128\n',text=True,capture_output=True,check=True)
            self.assertLess(len(json.loads((root/'m.json').read_text())['vertices']),128)
            subprocess.run(command+['--fill-budget'],input='128\n256\n',text=True,capture_output=True,check=True)
            self.assertEqual(len(json.loads((root/'m.json').read_text())['vertices']),256)

    def test_mask_matches_renderer_north_to_south_rows(self):
        import zlib
        south_to_north=np.array([[1,0,0],[1,1,0],[0,1,1]],dtype=bool)
        body=allocation.encode_mask(south_to_north)
        texture=np.unpackbits(np.frombuffer(zlib.decompress(body),dtype=np.uint8),bitorder='little')[:9].reshape(3,3)
        for y in range(3):
            for x in range(3):
                self.assertEqual(bool(texture[2-y,x]),bool(south_to_north[y,x]))

    def test_outer_edge_simplification_bound(self):
        axis=np.arange(257); grid=np.tile(np.sin(axis*.14)*100,(257,1))
        seeds=allocation.boundary_seeds(grid,12.5)
        for y in [0,256]:
            xs=sorted(x for x,sy in seeds if sy==y)
            estimate=np.interp(axis,xs,grid[y,xs])
            self.assertLessEqual(np.abs(estimate-grid[y]).max(),12.5+1e-9)


if __name__=='__main__': unittest.main()

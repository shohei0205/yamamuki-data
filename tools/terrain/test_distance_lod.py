"""傾けた視点の距離別レベルと境界の裾を確認する。"""
import json
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import numpy as np
from polygon_region import payload

class DistanceLODTests(unittest.TestCase):
    def make_payload(self,view,levels=(4,6,8),zoom=6):
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'mesh.db'
            with sqlite3.connect(path) as db:
                db.execute('CREATE TABLE metadata(key,value)')
                db.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in {'levels':list(levels),'meshErrorBounds':{str(z):1000/2**z for z in levels}}.items()])
                db.execute('CREATE TABLE tiles(z,x,y,data)')
                for z in levels:
                    db.executemany('INSERT INTO tiles VALUES (?,?,?,?)',[(z,x,y,b'x') for x in range(2**z) for y in range(2**z)])
            db.close()
            points=np.array([[0,0,100],[256,0,100],[256,256,100],[0,256,100]])
            with patch('polygon_region.decode_mesh',return_value=(points,np.array([[0,1,2],[0,2,3]]))):
                return payload(path,35,138,1500,zoom,view)

    def test_tilt_uses_multiple_levels_and_closed_skirts(self):
        result=self.make_payload((80,0,700000))
        self.assertGreater(len(result['coverage']['levels']),1)
        self.assertTrue(np.isfinite(result['mesh']).all())
        self.assertLess(max(result['indices']),len(result['mesh']))
        self.assertGreater(len(result['indices'])//3,result['coverage']['savedTiles']*2)

    def test_top_view_retains_uniform_level(self):
        result=self.make_payload((0,0,700000))
        self.assertEqual(set(result['coverage']['levels']),{'6'})
        self.assertEqual(len(result['indices'])//3,result['coverage']['savedTiles']*2)

    def test_horizontal_near_camera_refines_beyond_base_level(self):
        result=self.make_payload((90,0,1000))
        self.assertIn("8",result["coverage"]["levels"])
        self.assertIn("6",result["coverage"]["levels"])

    def test_uneven_level_intervals_do_not_request_missing_levels(self):
        result=self.make_payload((90,0,1000),levels=(4,6,7,8),zoom=7)
        self.assertIn("8",result["coverage"]["levels"])
        self.assertEqual(result["coverage"]["missingTiles"],0)

    def test_outline_keeps_selected_level_without_loading_other_levels(self):
        from polygon_region import refine_outline
        coarse={'mesh':[[0,0,0],[10,0,10],[0,10,20]],'indices':[0,1,2],'projectionCenter':[138,35,100],'meshStepMeters':1,'coverage':{'zoom':4,'savedTiles':1}}
        with patch('polygon_region.payload') as load:refine_outline(coarse,None,35,138,100)
        load.assert_not_called()
        self.assertEqual(coarse['coverage']['zoom'],4)
        self.assertEqual(coarse['coverage']['outlineZoom'],4)
        self.assertEqual(coarse['indices'],[0,1,2])
        points=np.array(coarse['mesh'])
        np.testing.assert_allclose(points[:,2],points[:,0]+2*points[:,1],atol=1e-6)

    def test_stair_step_outline_is_rounded_without_flipping_faces(self):
        from polygon_region import smooth_outline
        points=[];faces=[];lookup={}
        def vertex(x,y):
            if (x,y) not in lookup:lookup[x,y]=len(points);points.append([x,y,10])
            return lookup[x,y]
        for x in range(6):
            for y in range(6-x):
                a,b,c,d=[vertex(xx,yy) for xx,yy in [(x,y),(x+1,y),(x+1,y+1),(x,y+1)]]
                faces.extend([[a,b,c],[a,c,d]])
        before=np.array(points,dtype=float);after=smooth_outline(points,np.array(faces).ravel(),1)
        self.assertGreater(np.count_nonzero(np.linalg.norm(after[:,:2]-before[:,:2],axis=1)>.01),5)
        self.assertLessEqual(np.linalg.norm(after[:,:2]-before[:,:2],axis=1).max(),.600001)
        a,b,c=after[np.array(faces)].transpose(1,0,2)
        self.assertTrue(np.all((b[:,0]-a[:,0])*(c[:,1]-a[:,1])-(b[:,1]-a[:,1])*(c[:,0]-a[:,0])>0))
        np.testing.assert_array_equal(after[:,2],before[:,2])
        diagonal=[lookup[x,6-x] for x in range(1,6)]
        self.assertTrue(np.any(np.abs(after[diagonal,0]-np.round(after[diagonal,0]))>.01))

    def test_distance_selection_uses_next_available_level(self):
        result=self.make_payload((80,0,700000),levels=(4,6,7,8),zoom=8)
        self.assertIn('7',result['coverage']['levels'])
        self.assertIn('8',result['coverage']['levels'])
        self.assertNotIn('4',result['coverage']['levels'])

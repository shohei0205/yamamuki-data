"""海岸線の湾口・島・内陸欠損と、小数頂点の保存を確認する。"""
import unittest
import numpy as np
import shapely
from shapely.geometry import box,Point
from coast_mesh import rebuild,simplify_land,polygon_parts
from polygon_dem import encode_coast_mesh,decode_mesh


def mesh(polygon,height=100):
    triangles=shapely.get_parts(shapely.constrained_delaunay_triangles(polygon))
    coords=shapely.get_coordinates(triangles).reshape(-1,4,2)[:,:3,:]
    xy,mapping=np.unique(coords.reshape(-1,2),axis=0,return_inverse=True)
    return np.column_stack([xy,np.full(len(xy),height)]),mapping.reshape(-1,3)

class CoastMeshTests(unittest.TestCase):
    def test_bay_remains_open_and_island_is_kept(self):
        points,faces=mesh(box(0,0,256,256))
        gulf=box(100,50,150,260);island=box(110,110,120,120)
        land=box(0,0,256,256).difference(gulf).union(island)
        output,triangles=rebuild(points,faces,land)
        geometry=shapely.union_all(shapely.polygons(output[triangles,:2]))
        self.assertLess(geometry.symmetric_difference(land).area,1e-5)
        self.assertFalse(geometry.covers(Point(130,200)))
        self.assertTrue(geometry.covers(Point(115,115)))
        self.assertTrue(np.isfinite(output).all())
        coast=(np.abs(output[:,0]-100)<1e-5)&(output[:,1]>=50)
        self.assertTrue(np.all(output[coast,2]==0))

    def test_inland_dem_hole_is_not_filled(self):
        original=box(0,0,256,256).difference(box(20,20,40,40))
        points,faces=mesh(original)
        output,triangles=rebuild(points,faces,box(0,0,256,256).difference(box(200,50,300,250)))
        geometry=shapely.union_all(shapely.polygons(output[triangles,:2]))
        self.assertFalse(geometry.covers(Point(30,30)))

    def test_fractional_coast_coordinates_roundtrip(self):
        points=np.array([[0.125,0.25,101.2],[255.75,0.375,201.9],[0.5,255.875,99.1]],dtype=np.float32)
        faces=np.array([[0,1,2]],dtype=np.uint32)
        restored,indices=decode_mesh(encode_coast_mesh(points,faces))
        np.testing.assert_array_equal(restored[:,:2],points[:,:2])
        np.testing.assert_array_equal(indices,faces)
        self.assertLessEqual(np.max(np.abs(restored[:,2]-points[:,2])),2.5)

    def test_simplification_keeps_island_inside_sea_hole(self):
        land=box(0,0,1000,1000).difference(box(100,100,900,900)).union(box(400,400,600,600))
        result=simplify_land(land,250)
        self.assertTrue(result.is_valid)
        self.assertEqual(len(polygon_parts(result)),2)
        self.assertEqual(sum(len(p.interiors) for p in polygon_parts(result)),1)
        self.assertTrue(result.covers(Point(500,500)))

    def test_source_uses_sea_boundary_and_keeps_inland_water(self):
        import mapbox_vector_tile
        from shapely.geometry import mapping
        from coastline_source import tile_land,WORLD
        features=[{'geometry':mapping(box(0,0,2048,4096)),'properties':{'vt_code':5101}},
                  {'geometry':mapping(box(2500,1000,3500,2000)),'properties':{'vt_code':5201}}]
        body=mapbox_vector_tile.encode([{'name':'WA','features':features}],default_options={'y_coord_down':True})
        land=tile_land(body,909,403)
        size=WORLD/1024;left=909*size-WORLD/2;top=WORLD/2-403*size
        self.assertFalse(land.covers(Point(left+.25*size,top-.5*size)))
        self.assertTrue(land.covers(Point(left+.75*size,top-.35*size)))

    def test_shared_coast_height_is_zero_on_both_sides(self):
        import sqlite3
        from coast_mesh import reconcile_boundaries
        from polygon_dem import encode_mesh
        db=sqlite3.connect(':memory:');db.execute('CREATE TABLE tiles(z,x,y,data)')
        points=np.array([[0,0,10],[256,0,10],[256,256,10],[0,256,10]],dtype=np.float32)
        faces=np.array([[0,1,2],[0,2,3]],dtype=np.uint32)
        other=points.copy();other[0,2]=0
        db.executemany('INSERT INTO tiles VALUES (?,?,?,?)',[(10,1,1,encode_mesh(points,faces)),(10,2,1,encode_coast_mesh(other,faces))])
        result=reconcile_boundaries(db,10)
        self.assertEqual(result['sharedCoastVertices'],1)
        restored,_=decode_mesh(db.execute('SELECT data FROM tiles WHERE x=1').fetchone()[0])
        self.assertEqual(restored[1,2],0)
        db.close()

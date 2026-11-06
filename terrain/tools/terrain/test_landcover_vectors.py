import unittest
import numpy as np
import shapely
from shapely.geometry import box
from rasterio.transform import from_bounds
from generate_landcover_vectors import classify,coverage,simplify_level

class VectorCoverageTests(unittest.TestCase):
    def test_four_classes_ignore_cities(self):
        data=np.array([[10,95,60,80,50,40,20,70,90,0]],np.uint8)
        self.assertEqual(classify(data).tolist(),[[1,1,2,3,0,0,0,0,0,0]])
    def test_coast_water_and_shared_lods(self):
        labels=np.zeros((8,8),np.uint8)
        labels[:4,:3]=1;labels[5,1]=2;labels[1:7,4]=3
        land=box(0,0,8,8).difference(box(6,0,8,3))
        faces,codes=coverage(labels,from_bounds(0,0,8,8,8,8),land,[0,0,8,8])
        self.assertTrue(shapely.coverage_is_valid(faces))
        for tolerance in [0,0.2,0.5,1]:
            grouped,report=simplify_level(faces,codes,tolerance,2,box(0,0,8,8))
            self.assertAlmostEqual(report['gapAreaM2'],0)
            self.assertAlmostEqual(report['overlapAreaM2'],0)
            self.assertGreater(grouped[3].area,0)
            self.assertAlmostEqual(grouped[4].area,6)
            self.assertAlmostEqual(grouped[2].area,0)
            self.assertTrue(all(g.is_valid for g in grouped))
    def test_split_edge_is_preserved(self):
        labels=np.zeros((8,8),np.uint8);labels[2:6,2:6]=1;labels[5:7,3:5]=2
        outputs=[]
        for x in [0,4]:
            b=[x,0,x+4,8];domain=box(*b)
            faces,codes=coverage(labels[:,x:x+4],from_bounds(*b,4,8),domain,b)
            groups,_=simplify_level(faces,codes,2,100,domain);outputs.append(groups)
        edge=shapely.LineString([(4,0),(4,8)])
        for code in range(5):
            a=outputs[0][code].intersection(edge);b=outputs[1][code].intersection(edge)
            self.assertLess(a.symmetric_difference(b).length,1e-9)
if __name__=='__main__':unittest.main()

import json
import sqlite3
import struct
import unittest
import numpy as np
from shapely.geometry import Polygon,Point
from inland_vectors import decode_rivers,decode_water,make_packet


def ring(points):
    return struct.pack('<I',len(points))+b''.join(struct.pack('<HH',*p) for p in points)

class InlandTest(unittest.TestCase):
    def test_decode_and_truncation(self):
        data=b'RIV1'+struct.pack('<I',1)+ring([(0,0),(4096,4096)])
        np.testing.assert_equal(decode_rivers(data)[0],[[0,0],[1,1]])
        with self.assertRaises((ValueError,struct.error)):decode_rivers(data[:-1])
        with self.assertRaises(ValueError):decode_rivers(b'BAD1'+data[4:])
    def test_clip_and_preserve_water_hole(self):
        db=sqlite3.connect(':memory:')
        db.execute('CREATE TABLE tiles(layer TEXT,z,x,y,data BLOB)')
        line=b'RIV1'+struct.pack('<I',1)+ring([(0,2048),(4096,2048)])
        water=b'WAT1'+struct.pack('<III',1,0,2)+ring([(0,0),(4096,0),(4096,4096),(0,4096)])+ring([(1638,1638),(2458,1638),(2458,2458),(1638,2458)])
        db.execute('INSERT INTO tiles VALUES(?,?,?,?,?)',('rivers',0,0,0,line))
        db.execute('INSERT INTO tiles VALUES(?,?,?,?,?)',('water',0,0,0,water))
        self.assertEqual(len(decode_water(water)[0][1]),2)
        body,h=make_packet(db,[.25,.25,.5,.5],0)
        size=struct.unpack_from('<I',body,4)[0]
        values=np.frombuffer(body,dtype='<f4',offset=8+size).reshape(-1,2)
        self.assertTrue(np.all(values>=0) and np.all(values<=1))
        self.assertGreater(h['counts'][0],2)
        triangles=values[h['counts'][0]:h['counts'][0]+h['counts'][1]].reshape(-1,3,2)
        self.assertFalse(any(Polygon(t).contains(Point(.5,.5)) for t in triangles))
        self.assertAlmostEqual(sum(Polygon(t).area for t in triangles),.84,delta=.001)
        self.assertEqual(h['counts'][2],0)
        self.assertEqual(json.loads(body[8:8+size])['attribution'],'© OpenStreetMap contributors (ODbL)')
if __name__=='__main__':unittest.main()

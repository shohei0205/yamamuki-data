import json
from pathlib import Path
import sqlite3
import struct
import tempfile
import unittest
from water_tile_relay import relay

class RelayTest(unittest.TestCase):
    def test_only_requested_raw_blobs(self):
        with tempfile.TemporaryDirectory(dir='build') as temp:
            p=Path(temp)/'source.sqlite3'
            with sqlite3.connect(p) as db:
                db.execute('CREATE TABLE tiles(layer TEXT,z,x,y,data BLOB)')
                db.executemany('INSERT INTO tiles VALUES(?,?,?,?,?)',[('sea',12,3600,1600,b'WAT1-original'),('rivers',12,3601,1600,b'other'),('sea',11,3600,1600,b'wrong-level')])
            db.close()
            data=relay(p,{k:[str(v)] for k,v in dict(z=12,x0=3600,x1=3600,y0=1600,y1=1600).items()})
            self.assertEqual(data[:4],b'YSQ1')
            n=struct.unpack_from('<I',data,4)[0];header=json.loads(data[8:8+n]);self.assertEqual(len(header),1)
            self.assertEqual(header[0]['layer'],'sea');self.assertEqual(data[8+n:],b'WAT1-original')
    def test_invalid_bounds(self):
        for values in [dict(z=9,x0=0,x1=1,y0=0,y1=1),dict(z=12,x0=10,x1=1,y0=0,y1=1),dict(z=12,x0=0,x1=99999,y0=0,y1=1),dict(z=12,x0=0,x1=999,y0=0,y1=999)]:
            with self.assertRaises(ValueError):relay(Path('not-used'),{k:[str(v)] for k,v in values.items()})
if __name__=='__main__':unittest.main()

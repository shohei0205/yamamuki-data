"""SQL 中継が BLOB を変更せず、地形を再構成しないことを確認する。"""
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import struct
import tempfile
import threading
import unittest
from unittest.mock import patch
from urllib.request import urlopen
from http.server import ThreadingHTTPServer

from sqlite_relay import relay
from region_server import handler_class


def unpack(body):
    magic,length=struct.unpack_from('<4sI',body)
    assert magic==b'YSQ1'
    rows=json.loads(body[8:8+length]);cursor=8+length
    for row in rows:
        row['body']=body[cursor:cursor+row['bytes']];cursor+=row['bytes']
    assert cursor==len(body)
    return rows


class RelayTests(unittest.TestCase):
    def setup_files(self,root):
        build=root/'build';build.mkdir();mesh=build/'terrain-mesh.sqlite3'
        with closing(sqlite3.connect(mesh)) as db:
            db.executescript('CREATE TABLE metadata(key TEXT,value TEXT);CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,data BLOB);')
            db.execute('INSERT INTO metadata VALUES (?,?)',('schemaFormat','"yamamuki-terrain-mesh"'))
            db.execute('INSERT INTO tiles VALUES (12,3614,1604,?)',(b'YMP2-compressed-unchanged',));db.commit()
        lines=build/'terrain-national-lines.sqlite3'
        with closing(sqlite3.connect(lines)) as db:
            db.executescript('CREATE TABLE line_tiles(id INTEGER,level INTEGER,kind TEXT,x INTEGER,y INTEGER,body BLOB);CREATE TABLE line_bounds(id INTEGER,west REAL,south REAL,east REAL,north REAL);')
            db.execute('INSERT INTO line_tiles VALUES (1,10,"river",1,2,?)',(b'YVL1-original-gzip',))
            db.execute('INSERT INTO line_bounds VALUES (1,137,35,138,37)');db.commit()
        water=build/'terrain-vectors.sqlite3'
        with closing(sqlite3.connect(water)) as db:
            db.execute('CREATE TABLE features(kind TEXT,west REAL,south REAL,east REAL,north REAL,body BLOB)')
            db.execute('INSERT INTO features VALUES ("water",137,35,138,37,?)',(b'original-geojson-gzip',));db.commit()
        return mesh

    def test_overview_returns_every_level4_blob_without_modification(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);mesh=self.setup_files(root)
            with closing(sqlite3.connect(mesh)) as db:
                db.executemany('INSERT INTO tiles VALUES (4,?,?,?)',[(13,5,b'west-original'),(14,6,b'east-original')]);db.execute('INSERT INTO tiles VALUES (8,225,100,?)',(b'relief-preserved-original',));db.commit()
            rows=unpack(relay('/sqlite/overview',{},mesh,root)[0])
            self.assertEqual([(row['z'],row['x'],row['y']) for row in rows],[(4,13,5),(4,14,6)])
            self.assertEqual([row['body'] for row in rows],[b'west-original',b'east-original'])
            detailed=unpack(relay('/sqlite/overview',{'level':['8']},mesh,root)[0])
            self.assertEqual([row['body'] for row in detailed],[b'relief-preserved-original'])
            with self.assertRaises(ValueError):relay('/sqlite/overview',{'level':['14']},mesh,root)

    def test_original_blobs_and_geographic_selection(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);mesh=self.setup_files(root)
            body,kind=relay('/sqlite/mesh',{'tiles':['12,3614,1604;12,3615,1604']},mesh,root)
            rows=unpack(body);self.assertEqual(len(rows),1);self.assertEqual(rows[0]['body'],b'YMP2-compressed-unchanged')
            query={k:[str(v)] for k,v in dict(west=137.1,south=35.1,east=137.9,north=36.9,level=10).items()}
            self.assertEqual(unpack(relay('/sqlite/lines',query,mesh,root)[0])[0]['body'],b'YVL1-original-gzip')
            self.assertEqual(unpack(relay('/sqlite/water',query,mesh,root)[0])[0]['body'],b'original-geojson-gzip')
            query.update(west=['140'],east=['141'])
            self.assertEqual(unpack(relay('/sqlite/lines',query,mesh,root)[0]),[])

    def test_invalid_requests_are_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);mesh=self.setup_files(root)
            for tile in ('3,0,0','14,-1,0','14,0,16384','14,0',';'.join(['14,0,0']*257)):
                with self.subTest(tile=tile),self.assertRaises(ValueError):relay('/sqlite/mesh',{'tiles':[tile]},mesh,root)
            with self.assertRaises(ValueError):relay('/sqlite/lines',{'west':['nan'],'east':['1'],'south':['0'],'north':['1']},mesh,root)

    def test_normalmap_blob_is_relayed_without_decoding(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);mesh=self.setup_files(root)
            body=b'original-gzip-normalmap'
            with closing(sqlite3.connect(root/'build/terrain-normalmaps.sqlite3')) as db:
                db.execute('CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,width INTEGER,height INTEGER,data BLOB)')
                db.execute('INSERT INTO tiles VALUES (4,14,6,1024,1024,?)',(body,));db.commit()
            rows=unpack(relay('/sqlite/normals',{},mesh,root)[0])
            self.assertEqual(rows[0]['body'],body)
            self.assertEqual((rows[0]['z'],rows[0]['width'],rows[0]['height']),(4,1024,1024))

    def test_html_and_sql_route_never_reconstruct_geometry(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);mesh=self.setup_files(root)
            server=ThreadingHTTPServer(('127.0.0.1',0),handler_class(root,root/'coverage.html',database=mesh))
            worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
            try:
                with patch('region_server.region_payload',side_effect=AssertionError('サーバーで地形を作ってはいけません')):
                    base=f'http://127.0.0.1:{server.server_port}'
                    html=urlopen(base+'/terrain?lat=36&lon=138&size=30').read().decode()
                    self.assertIn('"sqliteRelay": true',html);self.assertIn('terrain-client-bridge.js',html)
                    body=urlopen(base+'/sqlite/mesh?tiles=12,3614,1604').read()
                    self.assertEqual(unpack(body)[0]['body'],b'YMP2-compressed-unchanged')
            finally:server.shutdown();server.server_close();worker.join()


if __name__=='__main__':unittest.main()

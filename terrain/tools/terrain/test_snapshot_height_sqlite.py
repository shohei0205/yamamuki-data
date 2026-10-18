import json,sqlite3,tempfile,unittest
from pathlib import Path
from snapshot_height_sqlite import snapshot

class SnapshotTest(unittest.TestCase):
 def test_wal_is_included_and_copy_stays_unchanged(self):
  with tempfile.TemporaryDirectory() as folder:
   root=Path(folder);source=root/'source.sqlite3';output=root/'preview.sqlite3'
   db=sqlite3.connect(source)
   try:
    db.execute('PRAGMA journal_mode=WAL');db.execute('PRAGMA wal_autocheckpoint=0')
    db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)');db.execute('CREATE TABLE tiles(z INTEGER)');db.execute('INSERT INTO tiles VALUES(14)');db.commit()
    result=snapshot(source,output);self.assertEqual(result['levels'],[dict(level=14,tiles=1)]);self.assertTrue(result['incomplete'])
    db.execute('INSERT INTO tiles VALUES(14)');db.commit()
    copy=sqlite3.connect(output)
    try:
     self.assertEqual(copy.execute('SELECT count(*) FROM tiles').fetchone()[0],1)
     self.assertTrue(json.loads(copy.execute("SELECT value FROM metadata WHERE key='snapshot'").fetchone()[0])['incomplete'])
    finally:copy.close()
    self.assertEqual(db.execute('SELECT count(*) FROM tiles').fetchone()[0],2)
    with self.assertRaises(ValueError):snapshot(source,output)
   finally:db.close()

if __name__=='__main__':unittest.main()

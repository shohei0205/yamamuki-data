"""元の比較フォルダーを使わずSQLiteのみから表示パケットを再構築できるか確認する。"""
import json,struct,tempfile
from pathlib import Path
from natural_shared_sqlite_viewer import DatabaseDataset,ROOT
root=ROOT/'build/natural-shared-packed-v1';rows=json.loads((root/'report.json').read_text())['rows'];r=next(r for r in rows if r['depth']==2)
with tempfile.TemporaryDirectory() as temp:
 d=DatabaseDataset(root/'terrain-shared-xyz.sqlite3',Path(temp)/'cache');d.root=Path(temp)/'original-data-does-not-exist'
 assert not d.root.exists();assert len(d.available(r['region']))==8
 data=d.packet(r['region'],r['error'],'sqlite',r['x'],r['y']);n=struct.unpack_from('<I',data)[0];info=json.loads(data[4:4+n]);assert info['source']=='SQLite XYZ / Rust rebuild';assert info['dependencyTiles']>=3;assert 4+n+sum(info[k] for k in ['pointBytes','cellBytes','nodeBytes','referenceBytes'])==len(data)
 d.packet.cache_clear();assert d.packet(r['region'],r['error'],'sqlite',r['x'],r['y'])==data;assert d.cache_hits==1
 print('元生成フォルダーなし・親深度2の復号・Rust補助再生成・ディスクキャッシュ一致：成功',info['dependencyTiles'],info['compressedDependencyBytes'])

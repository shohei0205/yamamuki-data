"""差分保存途中の状態を新しい試験DBに作り、直列・並列の再開結果を照合する。"""
import argparse
import json
import shutil
import sqlite3
import struct
import subprocess
from pathlib import Path
from test_xyz_parallel import fingerprint
from test_xyz_national import check
from verify_xyz_region import references, tfc


def prepare(reference, target):
    assert not target.exists(), '新しい試験先を指定してください'
    target.mkdir(parents=True)
    shutil.copy2(reference / 'national-config.json', target / 'national-config.json')
    shutil.copytree(reference / 'chunks', target / 'chunks')
    for filename in ('terrain.sqlite3', 'national-work.sqlite3'):
        with sqlite3.connect(reference / filename) as source, sqlite3.connect(target / filename) as output:
            source.backup(output)
    with sqlite3.connect(target / 'terrain.sqlite3') as output, sqlite3.connect(target / 'national-work.sqlite3') as work:
        cache = {}

        def vertices(key):
            if key in cache:
                return cache[key]
            data = output.execute('SELECT vertices FROM tiles WHERE z=? AND tile_x=? AND tile_y=?', key).fetchone()[0]
            version, mode, count, ndeps, nrefs, nnew, packed, reserved = struct.unpack_from('<HH6I', data, 4)
            assert version == 4 and not reserved and count == nrefs + nnew
            parents = [vertices(struct.unpack_from('<Iii', data, 32 + 12 * i)) for i in range(ndeps)]
            at = 32 + 12 * ndeps
            refs = references(mode, data[at:at + packed], nrefs, parents)
            result = [parents[r >> 24][r & 0xffffff] for r in refs]
            for i in range(nnew):
                x, y, h = struct.unpack_from('<hhH', data, at + packed + 6 * i)
                span = 1 << (14 - key[0])
                result.append(((key[1] * 16384 + x) * span, (key[2] * 16384 + y) * span, h))
            cache[key] = result
            return result

        # z13の各頂点を復元し、本体に含まれる親参照を先に選ぶ。
        choices = {}
        for x, y, nt, body, data in output.execute('SELECT tile_x,tile_y,triangle_count,body_triangle_count,triangles FROM tiles WHERE z=13 ORDER BY tile_y,tile_x'):
            points = vertices((13, x, y))
            body_bytes = struct.unpack_from('<I', data, 16)[0]
            body_vertices = {i for f in tfc(data[24:24 + body_bytes], body, len(points)) for i in f}
            for i, (px, py, h) in enumerate(points):
                candidate = (i not in body_vertices, y, x, i, h)
                if (px, py) not in choices or candidate[:3] < choices[px, py][:3]:
                    choices[px, py] = candidate
        work.execute('DELETE FROM parent_points')
        for (x, y), (halo, ty, tx, index, h) in choices.items():
            work.execute('INSERT INTO parent_points VALUES(?,?,?,?,?,?,?)', (x, y, h, tx, ty, index, int(not halo)))
        work.execute('DELETE FROM packed WHERE z=14')
        # 保存途中のBLOBと次親索引を残す。再開時に完成形へ上書きできる必要がある。
        tiles = output.execute('SELECT tile_x,tile_y FROM tiles WHERE z=14 ORDER BY tile_y,tile_x').fetchall()
        assert len(tiles) > 1
        output.executemany('DELETE FROM tiles WHERE z=14 AND tile_x=? AND tile_y=?', tiles[len(tiles)//2:])
        output.execute("UPDATE metadata SET value='false' WHERE key='complete'")
        work.execute('CREATE TABLE IF NOT EXISTS next_parent_points(x INTEGER,y INTEGER,h INTEGER,tx INTEGER,ty INTEGER,idx INTEGER,body INTEGER,PRIMARY KEY(x,y)) WITHOUT ROWID')
        work.execute('INSERT OR REPLACE INTO next_parent_points SELECT * FROM parent_points LIMIT 3')
    return json.loads((target / 'national-config.json').read_text())


def main(args):
    expected = fingerprint(args.reference)
    results = []
    for workers in (1, 4):
        root = args.output / f'workers-{workers}'
        config = prepare(args.reference, root)
        command = [str(args.exe.resolve()), '--national', '--national-chunk-cells', str(config['cells']), '--national-pack-workers', str(workers), '--source', str(args.source.resolve()), '--source-sha256', config['source_sha256'], '--output', str(root.resolve())]
        if config['window']:
            command += ['--national-window', *map(str, config['window'])]
        subprocess.run(command, check=True)
        assert fingerprint(root) == expected, '差分保存の再開結果が参照と不一致'
        with sqlite3.connect(root / 'national-work.sqlite3') as work, sqlite3.connect(args.reference / 'national-work.sqlite3') as reference:
            assert work.execute('SELECT * FROM parent_points ORDER BY x,y').fetchall() == reference.execute('SELECT * FROM parent_points ORDER BY x,y').fetchall(), '次レベル用の親索引が不一致'
        check(root)
        results.append(json.loads((root / 'pack-z14.report.json').read_text()))
    (args.output / 'comparison.json').write_text(json.dumps(results, indent=2) + '\n', encoding='utf-8')
    print('差分保存途中からの再開: 直列・並列とも全BLOBと親索引が参照に一致', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    for name in ('exe', 'source', 'reference', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    main(parser.parse_args())

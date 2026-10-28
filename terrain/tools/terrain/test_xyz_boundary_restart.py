"""z9の一部だけ同期済みの同一状態から、逐次と並列で再開して比較する。"""
import argparse
import json
import shutil
import sqlite3
import struct
import subprocess
from pathlib import Path
from test_xyz_parallel import fingerprint
from test_xyz_national import check


def points(file):
    data = file.read_bytes()
    count = struct.unpack_from('<I', data, 4)[0]
    return [struct.unpack_from('<IId', data, 12 + i * 16)[:2] for i in range(count)]


def prepare(reference, target):
    assert not target.exists()
    target.mkdir(parents=True)
    config = json.loads((reference / 'national-config.json').read_text())
    shutil.copy2(reference / 'national-config.json', target / 'national-config.json')
    cells = config['cells']
    with sqlite3.connect(reference / 'terrain.sqlite3') as src, sqlite3.connect(target / 'terrain.sqlite3') as out:
        src.backup(out)
        out.execute('DELETE FROM tiles WHERE z>=9')
        out.execute("UPDATE metadata SET value='false' WHERE key='complete'")
    with sqlite3.connect(target / 'national-work.sqlite3') as work, sqlite3.connect(reference / 'national-work.sqlite3') as src:
        work.executescript('CREATE TABLE valid_tiles(z INTEGER,x INTEGER,y INTEGER,PRIMARY KEY(z,x,y)) WITHOUT ROWID;CREATE TABLE base_done(x INTEGER,y INTEGER,PRIMARY KEY(x,y)) WITHOUT ROWID;CREATE TABLE boundary(z INTEGER,x INTEGER,y INTEGER,PRIMARY KEY(z,x,y)) WITHOUT ROWID;CREATE TABLE synchronized(z INTEGER PRIMARY KEY);CREATE TABLE packed(z INTEGER PRIMARY KEY);')
        work.executemany('INSERT INTO valid_tiles VALUES(?,?,?)', src.execute('SELECT z,x,y FROM valid_tiles'))
        work.executemany('INSERT INTO base_done VALUES(?,?)', config['chunks'])
        work.executemany('INSERT INTO synchronized VALUES(?)', [(6,), (7,), (8,)])
        work.executemany('INSERT INTO packed VALUES(?)', [(6,), (7,), (8,)])
        for index, (cx, cy) in enumerate(config['chunks']):
            name = f'chunk-{cx}-{cy}'
            origin = reference / 'chunks' / name
            destination = target / 'chunks' / name
            destination.mkdir(parents=True)
            for file in origin.glob('z*.mesh'):
                shutil.copy2(file, destination / file.name)
            for z in range(6, 9):
                for suffix in ('mesh', 'audit.json'):
                    shutil.copy2(origin / f'final-z{z}.{suffix}', destination / f'final-z{z}.{suffix}')
            boundary = points(origin / 'z9.mesh')
            if index < 2:
                for suffix in ('mesh', 'audit.json'):
                    shutil.copy2(origin / f'final-z9.{suffix}', destination / f'final-z9.{suffix}')
                boundary += points(origin / 'final-z9.mesh')
            for x, y in boundary:
                if x in (0, cells) or y in (0, cells):
                    work.execute('INSERT OR IGNORE INTO boundary VALUES(9,?,?)', (cx * cells + x, cy * cells + y))
    return config


def main(args):
    results = []
    for workers in (1, 4):
        root = args.output / f'workers-{workers}'
        config = prepare(args.reference, root)
        command = [str(args.exe.resolve()), '--national', '--national-chunk-cells', str(config['cells']), '--national-workers', str(workers), '--national-boundary-workers', str(workers), '--national-pack-workers', str(workers), '--national-window', *map(str, config['window']), '--source', str(args.source.resolve()), '--source-sha256', config['source_sha256'], '--output', str(root.resolve())]
        subprocess.run(command, check=True)
        check(root)
        results.append(fingerprint(root))
    assert results[0] == results[1], '同期途中の再開で逐次版と並列版が異なります'
    print('境界同期途中の再開: 逐次・並列の全形状／全BLOBが一致', results[0])


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    for name in ('exe', 'source', 'reference', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    main(parser.parse_args())

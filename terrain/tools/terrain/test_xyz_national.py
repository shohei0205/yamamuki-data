"""小範囲の全国経路を、独立したBLOB復元と作業境界の照合で検査する。"""
import argparse
import json
import struct
import hashlib
import subprocess
import os
from pathlib import Path
from verify_xyz_region import verify


def shape(path):
    data = path.read_bytes()
    magic, nv, nf = struct.unpack_from('<4sII', data)
    assert magic == b'XRG1' and len(data) == 12 + nv * 16 + nf * 12
    points = [struct.unpack_from('<IId', data, 12 + i * 16) for i in range(nv)]
    return {(x, y): h for x, y, h in points}


def check(root, resume_exe=None, source=None):
    config = json.loads((root / 'national-config.json').read_text())
    cells = config['cells']
    previous = {}
    for z in range(6, 15):
        current = {}
        for x, y in config['chunks']:
            key = (x, y)
            current[key] = shape(root / 'chunks' / f'chunk-{x}-{y}' / f'final-z{z}.mesh')
            assert previous.get(key, {}).keys() <= current[key].keys(), '親の頂点が細レベルから欠落'
        for (x, y), points in current.items():
            if (x + 1, y) in current:
                a = {py: h for (px, py), h in points.items() if px == cells}
                b = {py: h for (px, py), h in current[x + 1, y].items() if px == 0}
                assert a == b, f'z{z} 縦の作業境界が不一致'
            if (x, y + 1) in current:
                a = {px: h for (px, py), h in points.items() if py == cells}
                b = {px: h for (px, py), h in current[x, y + 1].items() if py == 0}
                assert a == b, f'z{z} 横の作業境界が不一致'
        previous = current
    result = verify(root / 'terrain.sqlite3')
    assert result['quantizedGeometryPassed']
    assert all(level['tiles'] for level in result['levels'])
    if resume_exe:
        import sqlite3
        def digest():
            digest = hashlib.sha256()
            with sqlite3.connect(root / 'terrain.sqlite3') as db:
                for row in db.execute('SELECT vertices,triangles FROM tiles ORDER BY z,tile_y,tile_x'):
                    for blob in row:
                        digest.update(struct.pack('<Q', len(blob)))
                        digest.update(blob)
            for path in sorted(root.glob('chunks/*/final-z*.mesh')):
                digest.update(path.read_bytes())
            return digest.digest()
        before = digest()
        command = [str(resume_exe.resolve()), '--national', '--national-chunk-cells', str(cells), '--source', str(source.resolve()), '--output', str(root.resolve()), '--source-sha256', config['source_sha256']]
        if config['window']:
            command += ['--national-window', *map(str, config['window'])]
        if os.name == 'nt':
            with (root / 'generation.lock').open('rb'):
                blocked = subprocess.run(command, capture_output=True)
                assert blocked.returncode != 0, '排他ロック中に二重起動できてしまいました'
            assert digest() == before, '二重起動拒否で形状が変化しました'
        subprocess.run(command, check=True)
        assert digest() == before, '再開で確定形状またはBLOBが変化しました'
    print('全9レベル: 共有境界・親頂点継承・独立BLOB復元に成功')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('directory', type=Path)
    parser.add_argument('--resume-exe', type=Path)
    parser.add_argument('--source', type=Path)
    args = parser.parse_args()
    check(args.directory, args.resume_exe, args.source)

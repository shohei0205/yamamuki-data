"""生成処理を止めず、WAL を含む整合した閲覧用コピーを保存する。"""
import argparse
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import tempfile


def snapshot(source, output):
    if output.exists():
        raise ValueError('既存の閲覧用ファイルは上書きしません')
    output.parent.mkdir(parents=True, exist_ok=True)
    progress = source.parent/'progress.json'
    complete = progress.exists() and json.loads(progress.read_text(encoding='utf-8')).get('state') == 'complete'
    with tempfile.NamedTemporaryFile(dir=output.parent, suffix='.sqlite3', delete=False) as handle:
        temp = Path(handle.name)
    try:
        with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro', uri=True, timeout=30)) as src, closing(sqlite3.connect(temp)) as dst:
            src.backup(dst)
            dst.execute('PRAGMA journal_mode=DELETE')
            rows = dst.execute('SELECT z,count(*) FROM tiles GROUP BY z').fetchall()
            report = dict(incomplete=not complete, levels=[dict(level=z,tiles=n) for z,n in rows])
            dst.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)', ('snapshot', json.dumps(report)))
            dst.commit()
            if dst.execute('PRAGMA quick_check').fetchone()[0] != 'ok':
                raise ValueError('閲覧用コピーの整合性が不正です')
        temp.rename(output)
        return report
    finally:
        if temp.exists():
            temp.unlink()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(snapshot(args.source,args.output), ensure_ascii=False))

"""サーバー不要の単一 HTML を作る。JS と WASM は同梱の固定版を使う。"""
import base64
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]


def read(path):
    return path.read_text(encoding='utf-8')


def js_module(source, bindings='', inherited=''):
    """同梱の固定版だけを IIFE に包む。一般的なモジュール変換器ではない。"""
    names = re.findall(r'^export (?:const|let|class|function) (\w+)', source, re.M)
    source = re.sub(r'^import .*?;\s*$', '', source, flags=re.M)
    source = re.sub(r'^export \* from .*?;\s*$', '', source, flags=re.M)
    source = re.sub(r'^export (?=(const|let|class|function) )', '', source, flags=re.M)
    if re.search(r'^\s*(import|export)\b', source, re.M):
        raise ValueError('同梱モジュールの形式が変わりました')
    return '(()=>{' + bindings + source + '\nreturn {' + inherited + ','.join(names) + '};})()'


def factory(source):
    if source.count('export default Module;') != 1:
        raise ValueError('WASM 読み込みコードの形式が変わりました')
    # wasmBinary を渡すので、この URL からは取得しない。
    source = source.replace('import.meta.url', '"https://terrain.invalid/embedded.js"')
    return '(()=>{' + source.replace('export default Module;', 'return Module;') + '})()'


def bundle_worker(folder):
    vendor = folder / 'vendor'
    sqlite = vendor / 'wa-sqlite-1.0.0'
    jxl = vendor / 'icodec-0.6.0'
    binary = lambda p: 'Uint8Array.from(atob(' + json.dumps(base64.b64encode(p.read_bytes()).decode()) + '),c=>c.charCodeAt(0))'
    source = 'globalThis.terrainWasm={sqlite:' + binary(sqlite/'dist/wa-sqlite.wasm') + ',jxl:' + binary(jxl/'dist/jxl-dec.wasm') + '};\n'
    source += 'const SQLiteConstants=' + js_module(read(sqlite/'src/sqlite-constants.js')) + ';\n'
    source += 'const VFS=' + js_module(read(sqlite/'src/VFS.js'), 'const VFS=SQLiteConstants;', '...SQLiteConstants,') + ';\n'
    source += 'const SQLite=' + js_module(read(sqlite/'src/sqlite-api.js'), 'const SQLite=SQLiteConstants;', '...SQLiteConstants,') + ';\n'
    source += 'const SQLiteFactory=' + factory(read(sqlite/'dist/wa-sqlite.mjs')) + ';\n'
    source += 'const JxlFactory=' + factory(read(jxl/'dist/jxl-dec.js')) + ';\n'
    worker = read(folder/'sqlite-height-worker.js')
    source += re.sub(r'^import .*?;\s*$', '', worker, flags=re.M)
    return source


def build(root=ROOT):
    folder = root/'tools/terrain'
    html = read(folder/'national_height.html')
    worker = bundle_worker(folder)
    bootstrap = '<script>window.terrainStandaloneWorker=' + json.dumps(worker, ensure_ascii=False).replace('</', '<\\/') + ';</script>\n'
    html = html.replace('<script src=', bootstrap + '<script src=', 1)

    def script(match):
        url = match[1]
        path = root/'build/terrain-codec-tools/web/fzstd/umd/index.js' if url == '/codec-vendor/fzstd.js' else folder/url.lstrip('/')
        source = re.sub(r'</script', r'<\\/script', read(path), flags=re.I)
        return '<script>\n' + source + '\n</script>'

    html = re.sub(r'<script src="([^"]+)"></script>', script, html)
    licenses = []
    for package in ('wa-sqlite-1.0.0', 'icodec-0.6.0'):
        for path in sorted((folder/'vendor'/package).glob('*')):
            if path.is_file() and ('LICENSE' in path.name or path.name == 'libjxl-PATENTS'):
                licenses.append(path.name + '\n' + read(path))
    for path in [folder/'vendor/earcut-LICENSE.txt',root/'build/terrain-codec-tools/web/fzstd/LICENSE']:
        licenses.append(path.name+'\n'+read(path))
    html += '\n<!-- 同梱ライセンス\n' + '\n\n'.join(licenses).replace('--', '—') + '\n-->\n'
    output = root/'dist/terrain-viewer.html'
    output.parent.mkdir(exist_ok=True, parents=True)
    output.write_text(html, encoding='utf-8', newline='\n')
    print(f'{output} ({output.stat().st_size:,} bytes)')
    return output


if __name__ == '__main__':
    build()

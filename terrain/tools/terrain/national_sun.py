"""全国の縮小陰影を、指定した太陽位置で Rust 生成器に作らせる。"""
import json
import os
from pathlib import Path
import subprocess
import threading

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT/'build/terrain-national-sun'
JOBS = {}
LOCK = threading.Lock()


def angles(options):
    a, h = options.get('azimuth'), options.get('altitude')
    if type(a) is not int or not 0 <= a <= 360 or type(h) is not int or not 0 <= h <= 90:
        raise ValueError('太陽の方角は 0〜360°、高さは 0〜90°の整数で指定してください')
    return a % 360, h


def folder(a, h):
    if (a,h)==(315,45):
        return ROOT/'build/terrain-national-shading'
    return OUTPUT/f'a{a}-h{h}'


def specification(options):
    if 'datetime' in options:
        from solar_position import ephemeris_jst
        value=options['datetime']
        coefficients=ephemeris_jst(value)
        key='t'+value.replace('-','').replace(':','')
        return dict(datetime=value,key=key,coefficients=coefficients),OUTPUT/key
    a,h=angles(options)
    return dict(azimuth=a,altitude=h,key=f'a{a}-h{h}'),folder(a,h)


def status(options):
    spec,dest=specification(options)
    key=spec['key']
    manifest = dest/'manifest.json'
    result = {k:v for k,v in spec.items() if k!='coefficients'}
    process = JOBS.get(key)
    if manifest.is_file() and (dest/'illumination-levels.json').is_file() and (process is None or process.poll() == 0):
        meta = json.loads(manifest.read_text(encoding='utf-8'))
        if (meta.get('solarDateTime')!=spec['datetime'] if 'datetime' in spec else (meta.get('solarDateTime') is not None or meta['lightAzimuth']%360!=spec['azimuth'] or meta['lightAltitude']!=spec['altitude'])):
            raise ValueError('生成済み陰影の太陽位置が一致しません')
        levels=json.loads((dest/'illumination-levels.json').read_text(encoding='utf-8'))['levels']
        return dict(result, state='complete', levels=levels,imageRoot='/national-shading-data/' if key=='a315-h45' else f'/national-sun/{key}/',bytes=meta['files']['shade.png'],
                    imageURL='/national-shading-data/shade.png' if key=='a315-h45' else f'/national-sun/{key}/shade.png')
    if process is not None and process.poll() is None:
        progress = {}
        try:
            progress = json.loads((dest/'progress.json').read_text(encoding='utf-8'))
        except (FileNotFoundError, json.JSONDecodeError):
            pass
        return dict(result, state='running', completed=progress.get('completed', 0), total=progress.get('total', 109506))
    if process is not None:
        error = (dest/'error.log').read_text(encoding='utf-8', errors='replace')[-1000:]
        return dict(result, state='failed', error=error or '陰影の生成に失敗しました')
    return dict(result, state='missing')


def start(options):
    spec,dest=specification(options)
    with LOCK:
        existing = status(options)
        if existing['state'] in ('running', 'complete'):
            return existing
        if any(p.poll() is None for p in JOBS.values()):
            raise BlockingIOError('別の太陽位置で陰影を生成中です。完了後に再実行してください')
        dest.mkdir(parents=True, exist_ok=True)
        compiler = next((ROOT/'build/rust-tools/llvm-mingw').iterdir())
        env = dict(os.environ)
        env['PATH'] = str(compiler/'bin')+os.pathsep+env['PATH']
        command = [str(ROOT/'build/terrain-rust-target/release/terrain-codec-bench.exe'),
                   '--part', 'national-shading', '--root', str(ROOT/'build/terrain-lod.sqlite3'),
                   '--output', str(dest)]
        if 'datetime' in spec:
            d,h=spec['coefficients']
            command.extend([f'--solar-declination={d}',f'--solar-hour-angle={h}','--solar-datetime',spec['datetime']])
        else:
            command.extend(['--sun-azimuth',str(spec['azimuth']),'--sun-altitude',str(spec['altitude'])])
        with (dest/'output.log').open('wb') as out, (dest/'error.log').open('wb') as err:
            process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=out, stderr=err,
                                       creationflags=subprocess.CREATE_NO_WINDOW)
        JOBS[existing['key']] = process
        (dest/'pid').write_text(str(process.pid), encoding='utf-8')
        return status(options)

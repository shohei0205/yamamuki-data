"""std の公式目録から標高取得候補を作り、tdlmn で取得する。"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import urllib.request
from fetch_national import bounds

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT / 'build/terrain-national'
CATALOG_URL = 'https://cyberjapandata.gsi.go.jp/xyz/std/mokuroku.csv.gz'


def load_missing(path, catalog_sha256):
    missing=set()
    if not path.exists():
        return missing
    with path.open(encoding='utf-8') as lines:
        for line in lines:
            try:
                record=json.loads(line)
            except json.JSONDecodeError:
                # 強制終了で最後の記録が途切れていても、未確認として取り直す。
                continue
            if record.get('catalogSha256')!=catalog_sha256:
                continue
            relative=record.get('url','').partition('/xyz/dem_png/')[2]
            if record.get('status')==404 and relative:
                missing.add(relative)
            elif record.get('status')==200:
                missing.discard(relative)
    return missing


def select_tiles(lines, area, zoom=14):
    x0,y0,x1,y1=area
    for line in lines:
        if not line.startswith(f'{zoom}/'):
            continue
        path=line.split(',',1)[0]
        z,x,filename=path.split('/')
        if not filename.endswith('.png'):
            continue
        y=int(filename[:-4]);x=int(x)
        if x0<=x<=x1 and y0<=y<=y1:
            yield f'{z}/{x}/{y}.png'


def main(args):
    OUTPUT.mkdir(parents=True,exist_ok=True)
    def progress(phase,**extra):
        record={'phase':phase,'sourceCatalog':CATALOG_URL,**extra}
        (OUTPUT/'tdlmn-progress.json').write_text(json.dumps(record,ensure_ascii=False,indent=2),encoding='utf-8')
    source=OUTPUT/'std-mokuroku.csv.gz'
    partial=source.with_suffix('.part')
    if not source.exists():
        progress('downloading_catalog')
        # 元目録をそのまま保存し、未完成のファイルは使用しない。
        with urllib.request.urlopen(CATALOG_URL,timeout=120) as response,partial.open('wb') as target:
            metadata={'url':CATALOG_URL,'lastModified':response.headers.get('Last-Modified'),'etag':response.headers.get('ETag')}
            while chunk:=response.read(1024*1024):
                if shutil.disk_usage(OUTPUT).free<10_000_000_000:
                    raise RuntimeError('空き容量が 10 GB 未満です')
                target.write(chunk)
        partial.replace(source)
        with source.open('rb') as stream:
            metadata['sha256']=hashlib.file_digest(stream,'sha256').hexdigest()
        (OUTPUT/'std-mokuroku-source.json').write_text(json.dumps(metadata,indent=2),encoding='utf-8')
    progress('selecting_candidates')
    tool=ROOT/'build/tdlmn/src'
    candidate=tool/'work/dem_png/mokuroku.csv.gz'
    candidate.parent.mkdir(parents=True,exist_ok=True)
    with source.open('rb') as stream:
        catalog_sha256=hashlib.file_digest(stream,'sha256').hexdigest()
    missing=load_missing(tool/'dem-downloads.jsonl',catalog_sha256)
    count=0
    skipped_missing=0
    with gzip.open(source,'rt',encoding='utf-8') as lines,gzip.open(candidate,'wt',encoding='utf-8',newline='\n') as target:
        for path in select_tiles(lines,bounds(args.bbox,14)):
            # std の日時・サイズ・MD5 を標高のものとして流用しない。
            count+=1
            if path in missing:
                skipped_missing+=1
                continue
            target.write(f'{path},0,0,dem-unverified\n')
    if count==0:
        raise RuntimeError('目録から取得候補を抽出できませんでした')
    commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=tool.parent,text=True).strip()
    for name in ['tdlmn.rb','download_tile.rb']:
        text=subprocess.check_output(['git','show',f'HEAD:src/{name}'],cwd=tool.parent).decode('utf-8').replace('\r\n','\n')
        if name=='tdlmn.rb':
            text=text.replace('$N_DOWNLOAD_THREADS = 8','$N_DOWNLOAD_THREADS = 4')
        else:
            text="require '"+(ROOT/'tools/terrain/tdlmn_dem.rb').as_posix()+"'\n"+text
            text=text.replace('            URI.open(o[:url],','            sleep 0.5\n            URI.open(o[:url],')
            text=text.replace('          buf = nil','          buf = nil\n          http_attempts = 0')
            start=text.index('          rescue OpenURI::HTTPError => e')
            end=text.index('          rescue\n',start)
            text=text[:start]+'''          rescue OpenURI::HTTPError => e
            if record_dem_not_found(e,o[:url],o[:local_path])
              $logger.info("404 : #{o[:url]}")
              $status[:ng] += 1
            elsif e.io.respond_to?(:status) && %w[429 500 502 503 504].include?(e.io.status[0]) && http_attempts < 7
              http_attempts += 1
              sleep [2**http_attempts,120].min
              retry
            else
              raise
            end
'''+text[end:]
            text=text.replace('if o[:md5] != buf_md5','if o[:md5] == "dem-unverified" ? !valid_dem_png?(buf) : o[:md5] != buf_md5')
            text=text.replace('              File.utime(o[:date], o[:date], o[:local_path])','              record_dem_tile(o[:url], o[:local_path], buf, 200)')
            needle='    # /diffモードで既存ファイルとMD5が一致すればダウンロードしない'
            text=text.replace(needle,'    if md5 == "dem-unverified" && File.exist?(local_path) && valid_dem_png?(File.binread(local_path))\n      $status[:skip] += 1\n      return false\n    end\n'+needle)
        (tool/name).write_text(text,encoding='utf-8',newline='\n')
    (OUTPUT/'tdlmn-source.json').write_text(json.dumps({'repository':'https://github.com/gsi-cyberjapan/tdlmn','commit':commit,'adaptations':['DEM の MD5 不明時は PNG の CRC・寸法・展開データを検査','取得後に SHA-256 を記録','既存の正常 PNG を再利用','同時接続4・問い合わせ前に0.5秒待つ','std の更新日時を DEM へ転用しない']},ensure_ascii=False,indent=2),encoding='utf-8')
    ruby=ROOT/'build/rubyinstaller-4.0.7-1-x64/bin/ruby.exe'
    command=[str(ruby),'-r',str(ROOT/'tools/terrain/tdlmn_compat.rb'),'tdlmn.rb','-dt','-diff']
    with (OUTPUT/'tdlmn-output.log').open('wb') as log:
        process=subprocess.Popen(command,cwd=tool,stdout=log,stderr=subprocess.STDOUT,
                                 env={**os.environ,'DEM_CATALOG_SHA256':catalog_sha256})
        progress('downloading_tiles',candidateTiles=count,skippedMissingTiles=skipped_missing,
                 queuedCandidates=count-skipped_missing,pid=process.pid)
        while process.poll() is None:
            if shutil.disk_usage(ROOT/'build/terrain-cache').free<10_000_000_000:
                process.terminate();process.wait()
                progress('disk_space_low',candidateTiles=count)
                return
            time.sleep(2)
    progress('finished_review_log',candidateTiles=count,exitCode=process.returncode)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bbox',nargs=4,type=float,default=[122.,20.,154.,46.])
    try:
        main(parser.parse_args())
    except Exception as error:
        (OUTPUT/'tdlmn-progress.json').write_text(json.dumps({'phase':'failed','error':str(error)},ensure_ascii=False),encoding='utf-8')
        raise

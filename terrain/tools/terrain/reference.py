"""生成済みの稜線候補を地理院の標準地図に重ね、目視で照合する。"""

import argparse
import base64
import hashlib
import io
import json
from pathlib import Path
import time
import urllib.request

import numpy as np
from PIL import Image, ImageDraw

from ridges import tile_position


def run(output, cache, offline):
    with np.load(output / "terrain.npz", allow_pickle=False) as archive:
        metadata = json.loads(str(archive["metadata"]))
    ox, oy = metadata["originPixel"]
    height, width = metadata["shape"]
    zoom = metadata["zoom"]
    xs = range(ox//256, (ox+width-1)//256+1)
    ys = range(oy//256, (oy+height-1)//256+1)
    if not (0 < width and 0 < height and len(xs)*len(ys) <= 64):
        raise ValueError("比較用の地図は 64 タイルまでです")
    base = Image.new("RGB", (width, height), "white")
    sources = []
    for y in ys:
        for x in xs:
            path = cache / str(zoom) / str(x) / f"{y}.png"
            url = f"https://cyberjapandata.gsi.go.jp/xyz/std/{zoom}/{x}/{y}.png"
            if not path.exists():
                if offline:
                    raise ValueError(f"保存されていない標準地図です: {path}")
                request = urllib.request.Request(url, headers={"User-Agent": "yamamuki-terrain-prototype/1"})
                with urllib.request.urlopen(request, timeout=30) as response:
                    data = response.read()
                with Image.open(io.BytesIO(data)) as image:
                    image.load()
                    if image.size != (256, 256):
                        raise ValueError("標準地図の大きさが不正です")
                path.parent.mkdir(parents=True, exist_ok=True)
                temporary = path.with_suffix(".tmp")
                temporary.write_bytes(data)
                temporary.replace(path)
                time.sleep(0.2)
            data = path.read_bytes()
            sources.append({"url": url, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
            with Image.open(io.BytesIO(data)) as tile:
                base.paste(tile.convert("RGB"), (x*256-ox, y*256-oy))
    sections = []
    for method, name in [("watershed", "流域境界"), ("inverse-flow", "反転流路")]:
        image = base.copy()
        draw = ImageDraw.Draw(image)
        collection = json.loads((output / f"{method}.geojson").read_bytes())
        for feature in collection["features"]:
            points = []
            for lon, lat, _ in feature["geometry"]["coordinates"]:
                x, y = tile_position(lon, lat, zoom)
                points.append((x*256-ox, y*256-oy))
            draw.line(points, fill=(225, 25, 55), width=2)
        png = io.BytesIO()
        image.save(png, format="PNG")
        encoded = base64.b64encode(png.getvalue()).decode("ascii")
        sections.append(f'<section><h2>{name}</h2><img alt="{name}の候補を赤線で重ねた地理院の標準地図" src="data:image/png;base64,{encoded}"></section>')
    html = ('<!doctype html><html lang="ja"><meta charset="utf-8"><title>稜線候補の地形図との照合</title>'
            '<style>body{font:14px system-ui;margin:16px}main{display:flex;gap:16px;flex-wrap:wrap}img{max-width:100%;height:auto}</style>'
            '<h1>稜線候補の地形図との照合</h1><p>赤線は自動抽出した候補です。地形図との一致を保証するものではありません。</p>'
            '<p>出典：<a href="https://maps.gsi.go.jp/development/ichiran.html">地理院タイル（標準地図・標高タイル）</a>を加工して山むきが作成。</p>'
            '<main>'+''.join(sections)+'</main></html>')
    (output / "reference.html").write_text(html, encoding="utf-8", newline="\n")
    (output / "reference-sources.json").write_text(json.dumps(sources, ensure_ascii=False, indent=2)+"\n", encoding="utf-8", newline="\n")
    print(f"地形図との比較を保存しました: {output / 'reference.html'}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--cache", type=Path, default=Path("build/terrain-reference-cache"))
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    try:
        run(args.output, args.cache, args.offline)
    except (ValueError, OSError) as error:
        parser.exit(1, f"地形図との比較に失敗しました: {error}\n")

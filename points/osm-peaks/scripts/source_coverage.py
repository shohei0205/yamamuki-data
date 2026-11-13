"""日本 PBF の切り出し境界と、追加抽出範囲から提供範囲を作る。"""

from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from coverage_geometry import parse_poly, union_geometry
from coverage_metadata import validate_coverage

if __package__:
    from .source_regions import SOURCES, REGIONS
    from .download_source import request, save_source_info
else:
    from source_regions import SOURCES, REGIONS
    from download_source import request, save_source_info


def capture_boundary(pbf):
    """取得済みの境界は内容を確認して再利用し、PBF と組にして保存する。"""
    pbf = Path(pbf)
    info_path = Path(str(pbf) + ".source.json")
    source = json.loads(info_path.read_bytes())
    polygon_path = Path(str(pbf) + ".poly")
    url = SOURCES["japan"] + ".poly"
    saved = source.get("boundary")
    if saved is not None:
        content = polygon_path.read_bytes()
        if (saved.get("url") != url or saved.get("sha256") != hashlib.sha256(content).hexdigest()
                or saved.get("sizeBytes") != len(content)):
            raise ValueError("日本 PBF と組になる境界ファイルの取得記録が一致しません")
        return parse_poly(content), saved
    with request(url, timeout=60) as response:
        content = response.read(8 * 1024 * 1024 + 1)
    if len(content) > 8 * 1024 * 1024:
        raise ValueError("日本の境界ファイルが 8 MiB の上限を超えました")
    geometry = parse_poly(content)
    record = {"url": url, "sha256": hashlib.sha256(content).hexdigest(), "sizeBytes": len(content),
              "fetchedAt": datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")}
    with tempfile.TemporaryDirectory(dir=pbf.parent) as directory:
        temporary = Path(directory) / "boundary.poly"
        temporary.write_bytes(content)
        temporary.replace(polygon_path)
    save_source_info(pbf, {**source, "boundary": record})
    return geometry, record


def generated_coverage(pbf, *, additional=False):
    geometry, record = capture_boundary(pbf)
    pieces = [geometry]
    if additional:
        for region in REGIONS:
            west, south, east, north = region["bbox"]
            pieces.append({"type": "Polygon", "coordinates": [
                [[west, south], [east, south], [east, north], [west, north], [west, south]]
            ]})
    coverage = {"regions": ["JP"], "geometry": union_geometry(pieces)}
    validate_coverage(coverage)
    return coverage, record

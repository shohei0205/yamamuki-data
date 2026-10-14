"""全部入り地点 JSON を、少ないメモリーで読み書きする。"""

from collections import Counter
import gzip
import hashlib
import json
import math
from pathlib import Path


def read_points(path, chunk_size=1024 * 1024):
    """通常の JSON 配列を読む。整形・改行の有無や gzip に依存しない。"""
    path = Path(path)
    opener = gzip.open if path.suffix == ".gz" else open
    with opener(path, "rt", encoding="utf-8-sig") as stream:
        buffer, position, eof = "", 0, False
        decoder = json.JSONDecoder()

        def refill():
            nonlocal buffer, position, eof
            chunk = stream.read(chunk_size)
            eof = not chunk
            buffer = buffer[position:] + chunk
            position = 0

        def whitespace():
            nonlocal position
            while True:
                while position < len(buffer) and buffer[position].isspace():
                    position += 1
                if position < len(buffer) or eof:
                    return
                refill()

        whitespace()
        if buffer[position:position+1] != "[":
            raise ValueError("地点 JSON が配列ではありません")
        position += 1
        whitespace()
        if buffer[position:position+1] != "]":
            while True:
                while True:
                    try:
                        point, end = decoder.raw_decode(buffer, position)
                        break
                    except json.JSONDecodeError:
                        if eof:
                            raise ValueError("地点 JSON が途中で終わっているか、不正です") from None
                        refill()
                if not isinstance(point, dict):
                    raise ValueError("地点がオブジェクトではありません")
                yield point
                position = end
                whitespace()
                if buffer[position:position+1] == "]":
                    break
                if buffer[position:position+1] != ",":
                    raise ValueError("地点の区切りが不正です")
                position += 1
                whitespace()
        position += 1
        whitespace()
        if position < len(buffer):
            raise ValueError("地点 JSON の末尾に余分なデータがあります")


def file_hash(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def input_record(path):
    return {"path": str(Path(path).resolve()), "sha256": file_hash(path)}


def write_points(points, output, name):
    """検証しながら JSON と gzip を作り、成功したら置き換える。"""
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    path = output / (name + ".json")
    archive = output / (name + ".json.gz")
    temporary = path.with_suffix(".json.part")
    compressed = archive.with_suffix(".gz.part")
    seen = set()
    counts, sources, heights, classifications = Counter(), Counter(), Counter(), Counter()
    digest = hashlib.sha256()
    size = 0
    try:
        with temporary.open("wb") as raw, compressed.open("wb") as target:
            with gzip.GzipFile(filename="", fileobj=target, mode="wb", mtime=0) as zipped:
                def write(payload):
                    nonlocal size
                    raw.write(payload)
                    zipped.write(payload)
                    digest.update(payload)
                    size += len(payload)

                write(b"[\n")
                for point in points:
                    identifier = point.get("id")
                    if not isinstance(identifier, str) or not identifier.strip() or identifier in seen:
                        raise ValueError(f"地点 ID が不正または重複しています: {identifier}")
                    seen.add(identifier)
                    if not isinstance(point.get("name"), str) or not point["name"].strip():
                        raise ValueError("地点名がありません")
                    for key, limit in (("latitude", 90), ("longitude", 180)):
                        value = point.get(key)
                        if type(value) not in (int, float) or not math.isfinite(value) or abs(value) > limit:
                            raise ValueError("地点の座標が不正です")
                    elevation = point.get("elevationM")
                    if elevation is not None and (type(elevation) not in (int, float) or not math.isfinite(elevation)):
                        raise ValueError("標高が不正です")
                    info = point.get("provenance")
                    if info is None:
                        info = {}
                    if not isinstance(info, dict) or set(info) - {"dataSource", "classification", "elevationSource"} or any(not isinstance(v, str) or not v.strip() or v.strip() != v for v in info.values()):
                        raise ValueError("地点の出典情報が不正です")
                    if counts["points"]:
                        write(b",\n")
                    write(json.dumps(point, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8"))
                    counts["points"] += 1
                    counts["peaks"] += point.get("type") == "peak"
                    counts["elevations"] += elevation is not None
                    sources[info.get("dataSource", "unknown")] += 1
                    heights[info.get("elevationSource", "unknown")] += 1
                    classifications[info.get("classification", "unknown")] += 1
                if not counts["points"]:
                    raise ValueError("出力する地点がありません")
                write(b"\n]\n")
        temporary.replace(path)
        compressed.replace(archive)
    finally:
        temporary.unlink(missing_ok=True)
        compressed.unlink(missing_ok=True)
    return {"pointCount": counts["points"], "peakCount": counts["peaks"],
            "annotationCount": counts["points"] - counts["peaks"],
            "elevationCount": counts["elevations"], "missingElevationCount": counts["points"] - counts["elevations"],
            "sha256": digest.hexdigest(), "sizeBytes": size, "gzipSizeBytes": archive.stat().st_size,
            "sources": dict(sources), "elevationSources": dict(heights), "classifications": dict(classifications)}


def write_report(output, report):
    path = Path(output) / "report.json"
    temporary = path.with_suffix(".json.part")
    temporary.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    temporary.replace(path)

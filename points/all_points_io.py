"""全部入り地点 JSON を、少ないメモリーで読み書きする。"""

from collections import Counter
import gzip
import hashlib
import json
import math
from pathlib import Path

from document_metadata import FIELDS, validate_metadata, ordered_point, alias_objects


def read_points(path, chunk_size=1024 * 1024, metadata=None):
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

        def value():
            nonlocal position
            whitespace()
            while True:
                try:
                    item, end = decoder.raw_decode(buffer, position)
                    if end == len(buffer) and not eof:
                        refill()
                        continue
                    position = end
                    return item
                except json.JSONDecodeError:
                    if eof:
                        raise ValueError("地点文書の項目が不正です") from None
                    refill()

        whitespace()
        wrapped = buffer[position:position+1] == "{"
        keys = set()
        tags = {}
        document_metadata = {}
        def field():
            nonlocal position, tags
            key = value()
            if key not in ("metadata", "schemaVersion", "points", "tags", "sourceUrl", "license", "attribution", "supplementSources", "pointCount", "latestPointTimestamp") or key in keys:
                raise ValueError("地点文書の項目が不正または重複しています")
            keys.add(key)
            whitespace()
            if buffer[position:position+1] != ":":
                raise ValueError("地点文書の区切りが不正です")
            position += 1
            if key == "metadata":
                item = value()
                if not isinstance(item,dict) or "schemaVersion" in item:
                    raise ValueError("metadata の構造が不正です")
                validate_metadata(item)
                tags=item.get("tags",{})
                document_metadata.update({k:v for k,v in item.items() if k != "tags"})
            elif key != "points":
                item = value()
                validate_metadata({key:item})
                if key == "tags":
                    tags = item
                else:
                    document_metadata[key] = item
            return key
        if wrapped:
            position += 1
            while field() != "points":
                whitespace()
                if buffer[position:position+1] != ",":
                    raise ValueError("地点文書に points がありません")
                position += 1
            whitespace()
        if buffer[position:position+1] != "[":
            raise ValueError("地点 JSON が配列ではありません")
        position += 1
        whitespace()
        point_count = 0
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
                if wrapped:
                    if "aliasReadings" in point:
                        raise ValueError("版6では aliasReadings を使えません")
                    alias_objects(point.get("aliases"), legacy=False)
                point_count += 1
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
        if wrapped:
            while buffer[position:position+1] == ",":
                position += 1
                if field() == "points":
                    raise ValueError("points が重複しています")
                whitespace()
            if buffer[position:position+1] != "}":
                raise ValueError("地点文書が閉じられていません")
            position += 1
            whitespace()
        if wrapped:
            if "schemaVersion" not in document_metadata:
                raise ValueError("版6の地点 JSON には schemaVersion が必要です")
            validate_metadata({**document_metadata, "tags":tags}, point_count)
        if metadata is not None:
            metadata.update(document_metadata, tags=tags)
        if position < len(buffer):
            raise ValueError("地点 JSON の末尾に余分なデータがあります")


def file_hash(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def input_record(path):
    return {"path": str(Path(path).resolve()), "sha256": file_hash(path)}


def write_points(points, output, name, tags=None, metadata=None):
    """検証しながら JSON と gzip を作り、成功したら置き換える。"""
    info = {} if metadata is None else metadata
    if not isinstance(info, dict) or "tags" in info:
        raise ValueError("文書の出典情報が不正です")
    validate_metadata({**info,"tags":{} if tags is None else tags})
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

                write(b'{"schemaVersion":6,"points":[\n')
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
                    alias_objects(point.get("aliases"), point.get("aliasReadings"))
                    info = point.get("provenance")
                    if info is None:
                        info = {}
                    if not isinstance(info, dict) or set(info) - {"dataSource", "classification", "elevationSource"} or any(not isinstance(v, str) or not v.strip() or v.strip() != v for v in info.values()):
                        raise ValueError("地点の出典情報が不正です")
                    if counts["points"]:
                        write(b",\n")
                    write(json.dumps(ordered_point(point), ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8"))
                    counts["points"] += 1
                    counts["peaks"] += point.get("type") == "peak"
                    counts["elevations"] += elevation is not None
                    sources[info.get("dataSource", "unknown")] += 1
                    heights[info.get("elevationSource", "unknown")] += 1
                    classifications[info.get("classification", "unknown")] += 1
                if not counts["points"]:
                    raise ValueError("出力する地点がありません")
                info={**{k:v for k,v in (metadata or {}).items() if k not in ("schemaVersion","pointCount")},"tags":tags or {},"pointCount":counts["points"]}
                write(('\n],"metadata":'+json.dumps(info,ensure_ascii=False,separators=(",",":"))+'}\n').encode("utf-8"))
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

"""地点文書の共通情報を、通常読み込み・逐次処理で同じ規則で検証する。"""

from datetime import datetime
import re

FIELDS = {"schemaVersion","sources", "pointCount", "tags"}


def text(value):
    return isinstance(value, str) and bool(value) and value == value.strip()


def validate_metadata(data, point_count=None):
    if not isinstance(data, dict) or set(data) - FIELDS:
        raise ValueError("地点文書の共通情報が不正です")
    if "schemaVersion" in data and (type(data["schemaVersion"]) is not int or data["schemaVersion"] != 6):
        raise ValueError("地点 JSON の schemaVersion は6にしてください")
    if "tags" in data:
        tags = data["tags"]
        if not isinstance(tags, dict) or any(not text(k) or not isinstance(v, dict) or set(v)-{"reading"}
                or ("reading" in v and not text(v["reading"])) for k,v in tags.items()):
            raise ValueError("タグ情報が不正です")
    if "sources" in data:
        sources = data["sources"]
        if not isinstance(sources, dict) or any(not text(k) or not isinstance(v, dict)
                or set(v)-{"source","license","attribution","changes","lastestTimeStamp"}
                or any(not isinstance(item,str) for item in v.values()) for k,v in sources.items()):
            raise ValueError("出典情報が不正です")
    if "pointCount" in data:
        count = data["pointCount"]
        if type(count) is not int or count < 1 or (point_count is not None and count != point_count):
            raise ValueError("pointCount は実際の地点数と等しい正の整数にしてください")
    if point_count is not None and point_count < 1:
        raise ValueError("地点配列には1地点以上が必要です")
    for entry in data.get("sources", {}).values():
        if "lastestTimeStamp" in entry:
            validate_timestamp(entry["lastestTimeStamp"])
    return data




def validate_timestamp(stamp):
    field = "lastestTimeStamp"
    if not isinstance(stamp, str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:[Zz]|[+-]\d{2}:\d{2})",stamp):
        raise ValueError(f"{field} はタイムゾーン付き日時にしてください")
    if stamp[-1] not in "Zz" and (int(stamp[-5:-3]) > 23 or int(stamp[-2:]) > 59):
        raise ValueError(f"{field} の時差が不正です")
    try:
        datetime.fromisoformat(stamp.upper().replace("Z","+00:00"))
    except ValueError as error:
        raise ValueError(f"{field} の日時が不正です") from error

def source_entries(identifier, source=None, license=None, attribution=None, supplements=None, lastestTimeStamp=None):
    validate_metadata({"sources":{} if supplements is None else supplements})
    result = {k:dict(v) for k,v in (supplements or {}).items()}
    entry = {k:v for k,v in {"source":source,"license":license,"attribution":attribution}.items() if v is not None}
    if lastestTimeStamp is not None:
        entry["lastestTimeStamp"] = lastestTimeStamp
    elif identifier in result and "lastestTimeStamp" in result[identifier]:
        entry["lastestTimeStamp"] = result[identifier]["lastestTimeStamp"]
    if entry:
        if identifier in result and result[identifier] != entry:
            raise ValueError("元データと補足の出典 ID が競合しています: "+identifier)
        result[identifier] = entry
    validate_metadata({"sources":result})
    return result


def alias_objects(aliases, readings=None, legacy=True):
    """別名を名前と読みの object にそろえる。旧版の文字列も入力できる。"""
    if aliases is None:
        aliases = []
    if not isinstance(aliases, list):
        raise ValueError("aliases は配列にしてください")
    result, seen = [], set()
    for value in aliases:
        item = {"name":value} if legacy and isinstance(value,str) else dict(value) if isinstance(value,dict) else None
        if item is None or set(item)-{"name","nameReading"} or not text(item.get("name")):
            raise ValueError("別名は name と任意の nameReading を持つ object にしてください")
        if "nameReading" in item and item["nameReading"] is not None and not text(item["nameReading"]):
            raise ValueError("別名の読みは空でない文字列または null にしてください")
        if item["name"] in seen:
            raise ValueError("別名の name が重複しています")
        seen.add(item["name"])
        result.append(item)
    if readings is not None:
        if not isinstance(readings,dict) or any(k not in seen or not text(v) for k,v in readings.items()):
            raise ValueError("旧別名よみがなの対応が不正です")
        for item in result:
            if item["name"] in readings:
                item["nameReading"] = readings[item["name"]]
    return result

def ordered_point(point):
    """表示名と読みを隣に並べ、ほかの項目の順序と値は保つ。"""
    point = dict(point)
    if point.get("aliases") is not None or "aliasReadings" in point:
        point["aliases"] = alias_objects(point.get("aliases"), point.pop("aliasReadings", None))
    if "name" not in point or "nameReading" not in point:
        return dict(point)
    result = {}
    for key, value in point.items():
        if key == "nameReading":
            continue
        result[key] = value
        if key == "name":
            result["nameReading"] = point["nameReading"]
    return result


def manifest_metadata(manifest):
    """版6の共通情報と、旧 manifest の直下の共通項目を読む。"""
    return manifest.get("metadata", {}) if manifest.get("schemaVersion", 0) >= 6 else manifest


def copy_manifest_metadata(manifest, metadata):
    """データ本体の共通情報をそのままコピーし、直下との重複を取り除く。"""
    from copy import deepcopy
    validate_metadata(metadata)
    for key in FIELDS - {"schemaVersion"}:
        manifest.pop(key, None)
    manifest.pop("sourceTimestamp", None)
    manifest.pop("latestPointTimestamp", None)
    manifest["metadata"] = deepcopy(metadata)
    return manifest

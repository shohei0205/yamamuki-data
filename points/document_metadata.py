"""地点文書の共通情報を、通常読み込み・逐次処理で同じ規則で検証する。"""

from datetime import datetime
import re

FIELDS = {"schemaVersion","sources", "pointCount", "tags"}


def text(value):
    return isinstance(value, str) and bool(value) and value == value.strip()


MAX_OSM_ID = 9007199254740991


def validate_point_id(value):
    if not text(value):
        raise ValueError("id は前後に空白のない空でない文字列にしてください")


def validate_osm_id(value):
    if type(value) is not int or not 0 < value <= MAX_OSM_ID:
        raise ValueError("osmId は1以上9007199254740991以下の整数にしてください")


def osm_node_id(point):
    """旧 osmId と、グローバルな OSM ノード ID を読む。"""
    if point.get("osmId") is not None:
        return point["osmId"]
    identifier = point.get("id", "")
    if isinstance(identifier, str) and re.fullmatch(r"osm:[1-9][0-9]*", identifier):
        value = int(identifier[4:])
        validate_osm_id(value)
        return value
    return None


def validate_related_ids(values, identifier=None):
    if not isinstance(values, list):
        raise ValueError("relatedIds は地点 ID の配列にしてください")
    seen = set()
    for value in values:
        validate_point_id(value)
        if value in seen or value == identifier:
            raise ValueError("relatedIds に重複または自分自身の ID は指定できません")
        seen.add(value)


def validate_point_identifiers(point):
    validate_point_id(point.get("id"))
    if point.get("osmId") is not None:
        validate_osm_id(point["osmId"])
    osm_node_id(point)
    if "relatedIds" in point:
        validate_related_ids(point["relatedIds"],point.get("id"))


def distributed_point(point):
    """地点版6では旧 osmId を関連地点 ID に変換する。"""
    result = ordered_point(point)
    osm_id = result.pop("osmId", None)
    if osm_id is not None:
        validate_osm_id(osm_id)
        related = f"osm:{osm_id}"
        if result.get("id") == str(osm_id):
            result["id"] = related
        if result.get("id") != related:
            result["relatedIds"] = list(dict.fromkeys([*result.get("relatedIds", []),related]))
    if "relatedIds" in result:
        validate_related_ids(result["relatedIds"],result.get("id"))
    return result


def metadata_maps(info):
    """配布用の配列を、照合用の内部対応表に変換する。"""
    result = dict(info)
    for field, key in (("sources", "name"), ("tags", "name")):
        if field not in result or isinstance(result[field], dict):
            continue
        values = result[field]
        if not isinstance(values, list):
            raise ValueError(field + " は配列にしてください")
        entries = {}
        for item in values:
            if not isinstance(item, dict) or not text(item.get(key)) or item[key] in entries:
                raise ValueError(field + " の識別子が不正または重複しています")
            entries[item[key]] = {k:v for k,v in item.items() if k != key}
        result[field] = entries
    return result


def metadata_arrays(info):
    """共通情報を配布用の配列にそろえる。"""
    result = metadata_maps(info)
    validate_metadata(result)
    for field, key in (("sources", "name"), ("tags", "name")):
        if field in result:
            result[field] = [{key: identifier, **value} for identifier, value in result[field].items()]
    return result


def validate_metadata(data, point_count=None):
    data = metadata_maps(data) if isinstance(data, dict) else data
    if not isinstance(data, dict):
        raise ValueError("地点文書の共通情報が不正です")
    if "schemaVersion" in data and (type(data["schemaVersion"]) is not int or data["schemaVersion"] != 6):
        raise ValueError("地点 JSON の schemaVersion は6にしてください")
    if "tags" in data:
        tags = data["tags"]
        if not isinstance(tags, dict) or any(not text(k) or not isinstance(v, dict)
                or ("nameReading" in v and not text(v["nameReading"])) for k,v in tags.items()):
            raise ValueError("タグ情報が不正です")
    if "sources" in data:
        sources = data["sources"]
        if not isinstance(sources, dict) or any(not text(k) or not isinstance(v, dict)
                or any(not isinstance(item,str) for k,item in v.items() if k in {"source","license","attribution","changes","lastestTimeStamp"}) for k,v in sources.items()):
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
        if item is None or not text(item.get("name")):
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
    return metadata_maps(manifest.get("metadata", {})) if manifest.get("schemaVersion", 0) >= 6 else manifest


def copy_manifest_metadata(manifest, metadata):
    """データ本体の共通情報をそのままコピーし、直下との重複を取り除く。"""
    from copy import deepcopy
    validate_metadata(metadata)
    for key in FIELDS - {"schemaVersion"}:
        manifest.pop(key, None)
    manifest.pop("sourceTimestamp", None)
    manifest.pop("latestPointTimestamp", None)
    manifest["schemaFormat"] = "yamamuki-manifest"
    manifest["dataSchemaFormat"] = "yamamuki-points"
    manifest["metadata"] = metadata_arrays(metadata)
    ordered = {"schemaFormat": manifest["schemaFormat"], "schemaVersion": manifest["schemaVersion"], **manifest}
    manifest.clear()
    manifest.update(ordered)
    return manifest


def point_document_json(data):
    """通常の JSON を保ち、points の各要素を1行ずつ出力する。"""
    import json
    def encode(value):
        return json.dumps(value, ensure_ascii=False, separators=(",", ":"), allow_nan=False)
    header = json.dumps({key:value for key,value in data.items() if key != "points"}, ensure_ascii=False, indent=2, allow_nan=False)
    lines = ["  " + encode(point) for point in data["points"]]
    return header[:-2] + ',\n  "points": [\n' + ",\n".join(lines) + '\n  ]\n}\n'

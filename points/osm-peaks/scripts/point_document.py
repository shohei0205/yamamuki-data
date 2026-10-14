"""版5以前の地点配列と、版6の地点文書を扱う。"""

from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from document_metadata import FIELDS, validate_metadata, ordered_point, alias_objects, source_entries


def document(points, tags=None, **metadata):
    info = {"schemaVersion":6, **metadata, "pointCount": len(points), "tags": {} if tags is None else tags}
    validate_metadata(info, len(points))
    return {"schemaVersion":6, "metadata":{k:v for k,v in info.items() if k != "schemaVersion"}, "points":[ordered_point(point) for point in points]}


def metadata_of(data):
    if not isinstance(data, dict):
        return {}
    return data.get("metadata", {k:v for k,v in data.items() if k not in ("points","schemaVersion")})


def rows(data, version=None):
    if version is not None and (type(version) is not int or version not in (1,2,3,4,5,6)):
        raise ValueError("未対応の地点データの版です")
    if isinstance(data, list) and version != 6:
        return data
    if isinstance(data, dict) and version not in (1, 2, 3, 4, 5):
        if "metadata" in data:
            if set(data)-{"schemaVersion","metadata","points"} or not isinstance(data["metadata"],dict) or "schemaVersion" in data["metadata"]:
                raise ValueError("地点 JSON の metadata の構造が不正です")
            data={**data["metadata"],"schemaVersion":data.get("schemaVersion"),"points":data.get("points")}

        if set(data) - {"tags", "points", *FIELDS} or not isinstance(data.get("points"), list):
            raise ValueError("版6の地点 JSON は points 配列を持つ object にしてください")
        if "schemaVersion" not in data:
            raise ValueError("版6の地点 JSON には schemaVersion が必要です")
        validate_metadata({k:v for k,v in data.items() if k != "points"}, len(data["points"]))
        for point in data["points"]:
            if not isinstance(point, dict) or "aliasReadings" in point:
                raise ValueError("版6では aliasReadings を使わず aliases に読みを格納してください")
            alias_objects(point.get("aliases"), legacy=False)
        return data["points"]
    raise ValueError("地点 JSON の版と構造が一致しません")

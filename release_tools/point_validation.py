"""出典に依存しない地点データの検証を行う。"""

import math
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "points/osm-peaks"))
sys.path.insert(0, str(ROOT / "points"))
from scripts.graphics import validate_graphics
from scripts.point_tags import validate_tags, validate_alias_readings
from document_metadata import validate_point_identifiers


def validate_rows(rows):
    if not isinstance(rows, list) or not rows:
        raise ValueError("地点がありません")
    seen = set()
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("地点がオブジェクトではありません")
        validate_point_identifiers(row)
        identifier = row.get("id")
        if not isinstance(identifier, str) or not identifier.strip() or identifier in seen:
            raise ValueError("地点 ID が不正または重複しています")
        seen.add(identifier)
        if not isinstance(row.get("name"), str) or not row["name"].strip():
            raise ValueError("地点名がありません")
        if row.get("type") is not None and (not isinstance(row["type"], str) or not re.fullmatch(r"[a-z][a-z0-9_]*", row["type"])):
            raise ValueError("地点の種別が不正です")
        provenance = row.get("provenance")
        if provenance is not None:
            keys = {"dataSource", "classification", "elevationSource"}
            if not isinstance(provenance, dict) or not set(provenance) <= keys or any(not isinstance(v, str) or not v.strip() or v.strip() != v for v in provenance.values()):
                raise ValueError("地点の出典情報が不正です")
        for key, limit in (("latitude", 90), ("longitude", 180)):
            value = row.get(key)
            if type(value) not in (int, float) or not math.isfinite(value) or abs(value) > limit:
                raise ValueError("座標が不正です")
    validate_tags(rows)
    validate_alias_readings(rows)
    validate_graphics(rows)


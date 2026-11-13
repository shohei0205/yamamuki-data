"""通信せずに PBF と組の境界記録を作る、テスト用の仮の範囲。"""

import hashlib
import json
from pathlib import Path

POLY = b"fixture\n1\n 122 20\n 146 20\n 146 46\n 122 46\n 122 20\nEND\nEND\n"


def save_boundary(pbf):
    path = Path(str(pbf) + ".source.json")
    source = json.loads(path.read_bytes())
    source["boundary"] = {"url": "https://download.geofabrik.de/asia/japan.poly",
                          "sha256": hashlib.sha256(POLY).hexdigest(), "sizeBytes": len(POLY),
                          "fetchedAt": "2026-11-13T00:00:00Z"}
    Path(str(pbf) + ".poly").write_bytes(POLY)
    path.write_text(json.dumps(source) + "\n", encoding="utf-8", newline="\n")


def fixture_coverage():
    from coverage_geometry import parse_poly
    return {"regions":["JP"], "geometry":parse_poly(POLY)}

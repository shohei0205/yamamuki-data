"""国土地理院の地点に共通形式の補足を適用する。"""

from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "osm-peaks"))
from scripts.supplements import apply_supplements, read_supplements

DEFAULT_SUPPLEMENTS = Path(__file__).resolve().parents[1] / "supplements.json"


def supplement_points(points, path=DEFAULT_SUPPLEMENTS):
    data = read_supplements(path)
    return apply_supplements(points, data), data.get("sources", {})

"""追加の取得元と、山頂を補う範囲。行政区分ではなく座標で選ぶ。"""

import json
from pathlib import Path

SOURCES = {
    "japan": "https://download.geofabrik.de/asia/japan",
    "far-eastern-fed-district": "https://download.geofabrik.de/russia/far-eastern-fed-district",
    "south-korea": "https://download.geofabrik.de/asia/south-korea",
}
REGIONS = json.loads((Path(__file__).resolve().parent.parent / "regions.json").read_text(encoding="utf-8"))


def region_for(latitude, longitude):
    for region in REGIONS:
        west, south, east, north = region["bbox"]
        if west <= longitude <= east and south <= latitude <= north:
            return region
    return None


def in_source_region(source, latitude, longitude):
    region = region_for(latitude, longitude)
    return region is not None and region["source"] == source

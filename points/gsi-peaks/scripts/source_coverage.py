"""海上保安庁の公式概念図をトレースし、GSI の概略提供範囲を記録する。"""

import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from coverage_geometry import geometry_json, check_points, union_geometry
from coverage_metadata import validate_coverage

DATA = Path(__file__).resolve().parents[1]
TRACE = DATA / "coverage-trace.json"
SOURCE_NAME = "海上保安庁（日本の領海等概念図）"
CONTROL_SOURCE_NAME = "国土地理院（地球地図日本・提供範囲の位置合わせ）"


def trace_coverage(trace):
    """手動で選んだ画素座標を、島の基準点で位置合わせして経緯度にする。"""
    from shapely.geometry import Polygon
    from shapely import union_all
    if trace.get("schemaVersion") != 1 or trace.get("projection") != "spherical-mercator-axis-aligned-least-squares":
        raise ValueError("概念図のトレース形式が不正です")
    width, height = trace["sourceImage"]["width"], trace["sourceImage"]["height"]
    def pixel(point):
        if (len(point) != 2 or any(type(v) not in (int, float) or not math.isfinite(v) for v in point)
                or not 0 <= point[0] < width or not 0 <= point[1] < height):
            raise ValueError("概念図の画素座標が不正です")
        return point
    controls = trace["controlPoints"]
    if len(controls) < 3:
        raise ValueError("概念図の基準点が足りません")
    for control in controls:
        pixel(control["pixel"])
        if not (-180 <= control["longitude"] <= 180 and -85 < control["latitude"] < 85):
            raise ValueError("基準点の経緯度が不正です")
    longitude = [p["longitude"] for p in controls]
    mercator = [math.asinh(math.tan(math.radians(p["latitude"]))) for p in controls]
    def fit(values, pixels):
        mean = math.fsum(values)/len(values)
        pixel_mean = math.fsum(pixels)/len(pixels)
        spread = math.fsum((v-mean)**2 for v in values)
        if spread == 0:
            raise ValueError("基準点が同じ経度または緯度に並んでいます")
        slope = math.fsum((v-mean)*(p-pixel_mean) for v,p in zip(values,pixels))/spread
        return [slope, pixel_mean-slope*mean]
    x = fit(longitude, [p["pixel"][0] for p in controls])
    y = fit(mercator, [p["pixel"][1] for p in controls])
    if x[0] <= 0 or y[0] >= 0:
        raise ValueError("概念図の軸方向が不正です")
    residuals = [math.dist(p["pixel"], [x[0]*p["longitude"]+x[1], y[0]*m+y[1]]) for p,m in zip(controls,mercator)]
    if max(residuals) > 4:
        raise ValueError("基準点の位置合わせの誤差が 4 画素を超えています")
    def lonlat(point):
        px, py = pixel(point)
        return [round((px-x[1])/x[0], 6), round(math.degrees(math.atan(math.sinh((py-y[1])/y[0]))), 6)]
    def ring(points):
        if len(points) < 4 or points[0] != points[-1]:
            raise ValueError("トレースのリングを閉じてください")
        return [lonlat(p) for p in points]
    polygons = []
    for polygon in trace["polygons"]:
        if "ellipsePixels" in polygon:
            ellipse = polygon["ellipsePixels"]
            cx, cy = pixel(ellipse["center"])
            rx, ry = ellipse["radius"]
            count = ellipse["segments"]
            if rx <= 0 or ry <= 0 or type(count) is not int or not 32 <= count <= 1024:
                raise ValueError("トレースの円が不正です")
            points = [lonlat([cx+rx*math.cos(i*2*math.pi/count), cy+ry*math.sin(i*2*math.pi/count)]) for i in range(count)]
            polygons.append(Polygon(points))
        else:
            polygons.append(Polygon(ring(polygon["exteriorPixels"]), [ring(r) for r in polygon.get("holesPixels", [])]))
    if not polygons or any(not p.is_valid or p.area <= 0 for p in polygons):
        raise ValueError("トレースの面が不正です")
    geometry = geometry_json(union_all(polygons))
    coverage = {"regions":["JP"], "geometry":geometry}
    validate_coverage(coverage)
    return coverage, {"xCoefficients":[round(v,9) for v in x], "yCoefficients":[round(v,9) for v in y], "maxControlResidualPixels":round(max(residuals),6)}


def coverage_sources(trace):
    return [
        {"name":SOURCE_NAME, "source":trace["sourcePage"],
         "license":"海洋情報部ホームページ利用ルール（政府標準利用規約第2.0版準拠） https://www1.kaiho.mlit.go.jp/copyright.html",
         "attribution":"出典：日本の領海等概念図（海上保安庁海洋情報部）を加工して作成",
         "changes":"陸域・内水・領海・EEZ を含む概略範囲を手動トレースし、延長大陸棚だけの海域を除外。法的境界・航海用の情報ではない。未画定海域は元図の便宜上の図示に従う。"},
        {"name":CONTROL_SOURCE_NAME, "source":"https://www.gsi.go.jp/kankyochiri/gm_jpn.html",
         "license":"国土地理院コンテンツ利用規約 https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html",
         "attribution":"出典：地球地図日本 第2.1版（国土地理院）を加工して作成",
         "changes":"6島の陸域ポリゴンの重心（ITRF94 経緯度）を概念図の概略位置合わせに使用。境界そのものの合成には使用しない。"}
    ]


def generated_coverage(extra_geometries=(), data=DATA):
    raw = (data / "coverage.geojson").read_bytes()
    record = json.loads((data / "coverage-source.json").read_bytes())
    if hashlib.sha256(raw).hexdigest() != record["geometrySha256"]:
        raise ValueError("保存済みの概略提供範囲の SHA-256 が一致しません")
    if hashlib.sha256((data / "coverage-trace.json").read_bytes()).hexdigest() != record["traceSha256"]:
        raise ValueError("トレースが変更されています。概略提供範囲を再生成してください")
    geometry = json.loads(raw)
    if extra_geometries:
        geometry = union_geometry([geometry, *extra_geometries])
        record["additionalGeometries"] = list(extra_geometries)
    coverage = {"regions":["JP"], "geometry":geometry}
    validate_coverage(coverage)
    return coverage, record


def default_coverage():
    return generated_coverage()[0]


def record_source(directory=DATA, *, trace_path=TRACE, source_image=None):
    directory = Path(directory)
    path = directory / "gsi-peaks-source.json"
    body = json.loads(path.read_bytes())
    snapshot_path = directory / "snapshot.json"
    snapshot = json.loads(snapshot_path.read_bytes())
    if hashlib.sha256(path.read_bytes()).hexdigest() != snapshot["inputSha256"]:
        raise ValueError("元 JSON の SHA-256 が snapshot と一致しません")
    trace_raw = Path(trace_path).read_bytes()
    trace = json.loads(trace_raw)
    if source_image is not None:
        image = Path(source_image).read_bytes()
        if len(image) != trace["sourceImage"]["sizeBytes"] or hashlib.sha256(image).hexdigest() != trace["sourceImage"]["sha256"]:
            raise ValueError("概念図の元画像が記録と一致しません")
    coverage, alignment = trace_coverage(trace)
    check_points(coverage["geometry"], body["points"])
    geometry_raw = (json.dumps(coverage["geometry"], ensure_ascii=False, separators=(",", ":"))+"\n").encode()
    record = {"kind":"jcg-concept-map-trace", "approximate":True, "sourcePage":trace["sourcePage"],
              "sourceImage":trace["sourceImage"], "controlPointSource":trace["controlPointSource"],
              "method":trace["method"], "projection":trace["projection"], "alignment":alignment,
              "traceSha256":hashlib.sha256(trace_raw).hexdigest(), "geometrySha256":hashlib.sha256(geometry_raw).hexdigest(),
              "notes":trace["notes"], "metadataSources":coverage_sources(trace)}
    body["metadata"]["coverage"] = coverage
    body["metadata"]["language"] = "ja"
    from document_metadata import point_document_json, metadata_maps, metadata_arrays
    metadata = metadata_maps(body["metadata"])
    for entry in record["metadataSources"]:
        metadata.setdefault("sources", {})[entry["name"]] = {k:v for k,v in entry.items() if k != "name"}
    body["metadata"] = metadata_arrays(metadata)
    raw = point_document_json(body).encode("utf-8")
    snapshot["inputSha256"] = hashlib.sha256(raw).hexdigest()
    snapshot["coverageSource"] = record
    snapshot["sources"] = body["metadata"]["sources"]
    (directory / "coverage.geojson").write_bytes(geometry_raw)
    (directory / "coverage-source.json").write_text(json.dumps(record, ensure_ascii=False, indent=2)+"\n",encoding="utf-8",newline="\n")
    if Path(trace_path).resolve() != (directory / "coverage-trace.json").resolve():
        (directory / "coverage-trace.json").write_bytes(trace_raw)
    path.write_bytes(raw)
    snapshot_path.write_text(json.dumps(snapshot, ensure_ascii=False, indent=2)+"\n",encoding="utf-8",newline="\n")
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source",type=Path,default=DATA)
    parser.add_argument("--trace",type=Path,default=TRACE)
    parser.add_argument("--source-image",type=Path,help="任意の原画像照合用。画像はリポジトリへ複製しない")
    args=parser.parse_args()
    print(json.dumps(record_source(args.source,trace_path=args.trace,source_image=args.source_image),ensure_ascii=False,indent=2))


if __name__ == "__main__":
    main()

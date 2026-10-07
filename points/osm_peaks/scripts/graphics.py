"""地点 JSON に埋め込む SVG と表示倍率を検査する。"""

import math
import re
import xml.etree.ElementTree as ET


MAX_SVG_BYTES = 100_000
ASSET_ID = r"[a-z][a-z0-9_-]*"
ELEMENTS = {"svg", "g", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon",
            "defs", "linearGradient", "radialGradient", "stop", "clipPath", "title", "desc"}
ATTRIBUTES = {"id", "viewBox", "width", "height", "x", "y", "x1", "y1", "x2", "y2",
              "cx", "cy", "r", "rx", "ry", "d", "points", "transform", "fill", "stroke",
              "stroke-width", "stroke-linecap", "stroke-linejoin", "stroke-miterlimit",
              "fill-rule", "clip-rule", "clip-path", "opacity", "fill-opacity", "stroke-opacity",
              "offset", "stop-color", "stop-opacity", "gradientUnits", "gradientTransform",
              "spreadMethod", "fx", "fy", "fr", "clipPathUnits", "preserveAspectRatio"}


def validate_svg(contents):
    if not 0 < len(contents) <= MAX_SVG_BYTES:
        raise ValueError("SVG のサイズが許容範囲外です")
    text = contents.decode("utf-8")
    if "<!DOCTYPE" in text or "<!ENTITY" in text:
        raise ValueError("SVG に DTD や実体参照は使用できません")
    root = ET.fromstring(text)
    namespace = "{http://www.w3.org/2000/svg}"
    if root.tag != namespace + "svg":
        raise ValueError("SVG のルートと名前空間が不正です")
    values = root.get("viewBox", "").replace(",", " ").split()
    try:
        import math
        box = [float(value) for value in values]
        if len(box) != 4 or not all(math.isfinite(value) for value in box) or box[2] <= 0 or box[3] <= 0:
            raise ValueError()
    except ValueError as exc:
        raise ValueError("SVG に有効な viewBox が必要です") from exc
    for element in root.iter():
        if not element.tag.startswith(namespace) or element.tag[len(namespace):] not in ELEMENTS:
            raise ValueError("SVG に未対応の要素があります")
        for key, value in element.attrib.items():
            if key not in ATTRIBUTES:
                raise ValueError(f"SVG に未対応の属性があります: {key}")
            if "url" in value.lower() and not re.fullmatch(r"url\(#[A-Za-z_][A-Za-z0-9_.-]*\)", value):
                raise ValueError("SVG の外部参照は使用できません")


def validate_graphics(rows):
    for row in rows:
        if row.get("graphic") is None:
            continue
        graphic = row["graphic"]
        if (not isinstance(graphic, dict) or "svg" not in graphic
                or not set(graphic) <= {"svg", "scale"} or not isinstance(graphic["svg"], str)):
            raise ValueError("graphic は SVG 文字列を持つオブジェクトで指定してください")
        validate_svg(graphic["svg"].encode("utf-8"))
        scale = graphic.get("scale")
        if scale is None:
            scale = 1.0
        if type(scale) not in (int, float) or not math.isfinite(scale) or scale <= 0:
            raise ValueError("graphic.scale は有限の正の数で指定してください")

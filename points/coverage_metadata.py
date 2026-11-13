"""共通 metadata の対象範囲と言語を検証する。外部サービスには問い合わせない。"""

import math
import re


# RFC 5646 の構文。登録済み言語・地域の一覧との照合は行わない。
LANGUAGE_PATTERN = (
    r"^(?:[A-Za-z]{2,3}(?:-[A-Za-z]{3}){0,3}|[A-Za-z]{4}|[A-Za-z]{5,8})"
    r"(?:-[A-Za-z]{4})?(?:-[A-Za-z]{2}|-[0-9]{3})?"
    r"(?:-[A-Za-z0-9]{5,8}|-[0-9][A-Za-z0-9]{3})*"
    r"(?:-[0-9A-WY-Za-wy-z](?:-[A-Za-z0-9]{2,8})+)*"
    r"(?:-[xX](?:-[A-Za-z0-9]{1,8})+)?$"
)
GRANDFATHERED = set("en-gb-oed i-ami i-bnn i-default i-enochian i-hak i-klingon "
                    "i-lux i-mingo i-navajo i-pwn i-tao i-tay i-tsu sgn-be-fr "
                    "sgn-be-nl sgn-ch-de art-lojban cel-gaulish no-bok no-nyn "
                    "zh-guoyu zh-hakka zh-min zh-min-nan zh-xiang".split())


def validate_language(value):
    if not isinstance(value, str):
        raise ValueError("language は BCP 47 の言語タグにしてください")
    lower = value.lower()
    if lower in GRANDFATHERED or re.fullmatch(r"x(?:-[a-z0-9]{1,8})+", lower):
        return
    if not re.fullmatch(LANGUAGE_PATTERN, value):
        raise ValueError("language は BCP 47 の言語タグにしてください")
    # variant と拡張の singleton の重複も拒否する。私用部分には適用しない。
    variants, singletons = set(), set()
    extension = False
    for part in lower.split("-")[1:]:
        if part == "x":
            break
        if len(part) == 1:
            if part in singletons:
                raise ValueError("language の拡張が重複しています")
            singletons.add(part)
            extension = True
        elif not extension and (len(part) >= 5 or (len(part) == 4 and part[0].isdigit())):
            if part in variants:
                raise ValueError("language の variant が重複しています")
            variants.add(part)


def geometry_polygons(value):
    if not isinstance(value, dict) or value.get("type") not in ("Polygon", "MultiPolygon"):
        raise ValueError("geometry は GeoJSON の Polygon または MultiPolygon にしてください")
    coordinates = value.get("coordinates")
    polygons = [coordinates] if value["type"] == "Polygon" else coordinates
    if not isinstance(polygons, list) or not polygons:
        raise ValueError("geometry に空の領域は指定できません")
    for polygon in polygons:
        if not isinstance(polygon, list) or not polygon:
            raise ValueError("Polygon には外周が必要です")
        for ring in polygon:
            if not isinstance(ring, list) or len(ring) < 4:
                raise ValueError("リングには4座標以上が必要です")
            for point in ring:
                if (not isinstance(point, list) or len(point) != 2
                        or any(type(v) not in (int, float) or (type(v) is float and not math.isfinite(v)) for v in point)
                        or not -180 <= point[0] <= 180 or not -90 <= point[1] <= 90):
                    raise ValueError("座標は WGS 84 の [経度, 緯度] にしてください")
            if ring[0] != ring[-1] or len({tuple(p) for p in ring[:-1]}) != len(ring) - 1:
                raise ValueError("リングは始終点を一致させ、途中の座標を重複させないでください")
            if any(abs(a[0] - b[0]) > 180 for a, b in zip(ring, ring[1:])):
                raise ValueError("日付変更線をまたぐ面は経度 ±180 度で分割してください")
    from shapely.geometry import shape
    from shapely.validation import explain_validity
    area = shape(value)
    if not area.is_valid or area.area <= 0:
        raise ValueError("提供範囲の面が不正です: " + explain_validity(area))
    return polygons


def validate_coverage(value, *, surface=False):
    if not isinstance(value, dict):
        raise ValueError("coverage は object にしてください")
    if "regions" in value:
        regions = value["regions"]
        if (not isinstance(regions, list) or not regions
                or any(not isinstance(r, str) or not re.fullmatch(r"[A-Z]{2}", r) for r in regions)
                or len(set(regions)) != len(regions)):
            raise ValueError("regions は重複のない国・地域コードの配列にしてください")
    outer = geometry_polygons(value["geometry"]) if "geometry" in value else None
    if "finest" in value:
        if not surface:
            raise ValueError("地点データには coverage.finest を指定できません")
        finest = value["finest"]
        if not isinstance(finest, dict) or "geometry" not in finest or outer is None:
            raise ValueError("finest には全体と最も精細な範囲の geometry が必要です")
        inner = geometry_polygons(finest["geometry"])
        from shapely.geometry import shape
        if not shape(value["geometry"]).covers(shape(finest["geometry"])):
            raise ValueError("finest の範囲が全体の提供範囲からはみ出しています")

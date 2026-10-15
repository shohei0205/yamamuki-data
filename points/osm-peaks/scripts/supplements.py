"""単一の地点補足 JSON を検査し、元データを変更せずに適用する。"""

from copy import deepcopy
import gzip
import hashlib
import json
from pathlib import Path

if __package__:
    from .graphics import validate_graphics
    from .point_tags import validate_tag_metadata
else:
    from graphics import validate_graphics
    from point_tags import validate_tag_metadata

from pathlib import Path as _Path
import sys as _sys
_sys.path.insert(0, str(_Path(__file__).resolve().parents[2]))
from document_metadata import osm_node_id, validate_related_ids, validate_point_id, validate_osm_id, alias_objects, metadata_arrays, metadata_maps

DEFAULT_SUPPLEMENTS = Path(__file__).resolve().parents[1] / "supplements.json"
FIELDS = {"name", "nameReading", "aliases", "aliasesRemove", "tags", "tagsRemove", "exclude", "graphic", "relatedIds"}
METADATA = {"source", "license", "attribution", "changes", "lastestTimeStamp"}
COMPANION_FILES = ("osm-peaks-source.json.gz", "supplements.json")
LEGACY_COMPANION_FILE = "supplement-manifest.json"


def text(value):
    return isinstance(value, str) and bool(value) and value == value.strip()


def validate_supplement_sources(sources):
    if not isinstance(sources, dict):
        raise ValueError("supplementSources は出典 ID と出典情報の対応にしてください")
    for identifier, metadata in sources.items():
        if not text(identifier) or not isinstance(metadata, dict) or any(not isinstance(value, str) for k,value in metadata.items() if k in METADATA):
            raise ValueError("supplementSources の出典 ID または出典情報が不正です")
    from document_metadata import validate_metadata
    validate_metadata({"sources": sources})
    return sources


def supplement_document(data):
    data=validate_supplements(data)
    return {"schemaFormat":"yamamuki-point-supplements","schemaVersion":2,"metadata":metadata_arrays({"sources":data.get("sources",{}),"tags":data.get("tags",{})}),"points":[{**{k:v for k,v in row.items() if k != "osmId"}, **({"id":f"osm:{row['osmId']}"} if "osmId" in row else {}), **({"aliases":alias_objects(row["aliases"])} if "aliases" in row else {})} for row in data["points"]]}


def validate_supplements(data):
    if isinstance(data, dict) and data.get("schemaVersion") == 2 and data.get("schemaFormat") != "yamamuki-point-supplements":
        raise ValueError("補足 JSON の schemaFormat が不正です")
    if isinstance(data,dict) and "metadata" in data:
        if not isinstance(data["metadata"],dict) or "pointCount" in data["metadata"]:
            raise ValueError("補足 JSON の metadata の構造が不正です")
        data={**({"schemaFormat":data["schemaFormat"]} if "schemaFormat" in data else {}),"schemaVersion":data.get("schemaVersion"),"points":data.get("points"),**metadata_maps(data["metadata"])}

    if not isinstance(data, dict) or type(data.get("schemaVersion")) is not int or data["schemaVersion"] not in (1, 2):
        raise ValueError("地点補足 JSON の schemaVersion は 1 または 2 にしてください")
    if "tags" in data:
        validate_tag_metadata(data["tags"])
    sources = data.get("sources", {})
    if not isinstance(sources, dict):
        raise ValueError("sources は出典 ID と出典情報の対応にしてください")
    for key, source in sources.items():
        if not text(key) or not isinstance(source, dict) or any(not isinstance(v, str) for k,v in source.items() if k in METADATA):
            raise ValueError("出典 ID または出典情報が不正です")
    if not isinstance(data.get("points"), list):
        raise ValueError("points は配列にしてください")
    seen = set()
    for row in data["points"]:
        if not isinstance(row, dict):
            raise ValueError("地点補足の項目が不正です")
        if data["schemaVersion"] == 2 and ("id" not in row or "osmId" in row):
            raise ValueError("補足 JSON 版2は id で対象を指定してください")
        if ("id" in row) == ("osmId" in row):
            raise ValueError("id または osmId のどちらか一方を指定してください")
        key = "id" if "id" in row else "osmId"
        identifier = row[key]
        if key == "id":
            validate_point_id(identifier)
        else:
            validate_osm_id(identifier)
        if (key, identifier) in seen:
            raise ValueError(f"{key} が重複しています")
        seen.add((key, identifier))
        if not set(row) & FIELDS:
            raise ValueError("地点補足には補足・補正する項目が必要です")
        for key in ("note", "reason"):
            if key in row and not isinstance(row[key], str):
                raise ValueError(f"{key} は文字列にしてください")
        if "name" in row and not text(row["name"]):
            raise ValueError("補正する地点名は空でない文字列にしてください")
        if "nameReading" in row and row["nameReading"] is not None and not text(row["nameReading"]):
            raise ValueError("よみがなは空でない文字列または null にしてください")
        if "relatedIds" in row:
            validate_related_ids(row["relatedIds"],row.get("id"))
        if "aliases" in row:
            if not isinstance(row["aliases"],list):
                raise ValueError("補足の aliases は配列にしてください")
            alias_objects(row["aliases"])
        if "graphic" in row:
            validate_graphics([row])
        if "exclude" in row and type(row["exclude"]) is not bool:
            raise ValueError("exclude は真偽値にしてください")
        for key in ("tags", "tagsRemove", "aliasesRemove"):
            if key in row:
                values = row[key]
                if not isinstance(values, list) or any(not text(v) for v in values) or len(set(values)) != len(values):
                    raise ValueError(f"{key} は重複のない空でない文字列の配列にしてください")
        if set(row.get("tags", [])) & set(row.get("tagsRemove", [])):
            raise ValueError("同じタグを追加と削除の両方に指定できません")
        if {v["name"] for v in alias_objects(row.get("aliases", []))} & set(row.get("aliasesRemove", [])):
            raise ValueError("同じ別名を追加と削除の両方に指定できません")
        expected = row.get("expected", {})
        if not isinstance(expected, dict):
            raise ValueError("expected は補正前の地点名・よみがな・別名を指定してください")
        for key, value in expected.items():
            if key == "aliases":
                alias_objects(value)
            elif key == "relatedIds":
                validate_related_ids(value)
            elif key in {"name","nameReading"} and value is not None and not isinstance(value, str):
                raise ValueError("expected の名前・よみがなが不正です")
    return data


def read_supplements(path=DEFAULT_SUPPLEMENTS):
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    if isinstance(data, dict) and data.get("schemaVersion") == 2:
        for row in data.get("points", []):
            if isinstance(row, dict) and "aliases" in row:
                alias_objects(row["aliases"], legacy=False)
    return validate_supplements(data)


def apply_supplements(points, data):
    validate_supplements(data)
    result = deepcopy(points)
    indexes = {"id": {}, "osmId": {}}
    for index, point in enumerate(result):
        for key in indexes:
            identifier = osm_node_id(point) if key == "osmId" else point.get(key)
            if identifier is None:
                continue
            if identifier in indexes[key]:
                raise ValueError(f"地点データの {key} が重複しています")
            indexes[key][identifier] = index
    excluded, applied = set(), set()
    for row in data["points"]:
        ref_key = "id" if "id" in row else "osmId"
        identifier = row[ref_key]
        if identifier not in indexes[ref_key]:
            raise ValueError(f"{ref_key} {identifier} が地点データにありません")
        index = indexes[ref_key][identifier]
        if index in applied:
            raise ValueError("同じ地点に複数の補足が指定されています")
        applied.add(index)
        point = result[index]
        for key, expected in row.get("expected", {}).items():
            if key not in {"name","nameReading","aliases","relatedIds"}:
                continue
            actual = point.get(key, [] if key in {"aliases","relatedIds"} else None)
            if key == "aliases":
                entries = alias_objects(actual)
                actual = [v["name"] for v in entries] if all(isinstance(v,str) for v in expected) else entries
            if actual != expected:
                raise ValueError(f"{ref_key} {identifier} の {key} が想定値と異なります。補正内容を確認してください")
        if "relatedIds" in row:
            validate_related_ids(row["relatedIds"],point.get("id"))
        for key in ("name", "nameReading", "graphic", "relatedIds"):
            if key in row:
                if row[key] is None:
                    point.pop(key, None)
                else:
                    point[key] = deepcopy(row[key])
        if "tags" in row or "tagsRemove" in row:
            point["tags"] = list(dict.fromkeys([v for v in (point.get("tags") or []) if v not in row.get("tagsRemove", [])] + row.get("tags", [])))
        if "aliases" in row or "aliasesRemove" in row:
            removed = set(row.get("aliasesRemove", []))
            entries = {v["name"]:v for v in alias_objects(point.get("aliases"),point.pop("aliasReadings",None)) if v["name"] not in removed}
            for item in alias_objects(row.get("aliases", [])):
                name = item["name"]
                entries[name] = {**entries.get(name,{}), **item}
            # 旧文字列だけの補足では従来の内部形式を維持する。書き出し時は object にそろえる。
            if all(isinstance(v,str) for v in point.get("aliases",[])) and all(isinstance(v,str) for v in row.get("aliases",[])) and not any("nameReading" in v for v in entries.values()):
                point["aliases"] = list(entries)
            else:
                point["aliases"] = list(entries.values())
        if row.get("exclude"):
            excluded.add(index)
    return [point for index, point in enumerate(result) if index not in excluded]


def validate_companions(directory, manifest, integrated, tags=None):
    directory = Path(directory)
    present = [(directory / name).exists() for name in COMPANION_FILES]
    legacy_path = directory / LEGACY_COMPANION_FILE
    if manifest.get("schemaVersion", 0) >= 6:
        if any(k in manifest for k in ("base", "supplements")):
            raise ValueError("地点 manifest 版6では補足ファイルを別添しません")
        return
    if not any(present) and not legacy_path.exists():
        return
    if not all(present) or not legacy_path.exists():
        raise ValueError("元データ・補足・補足 manifest がそろっていません")
    companion = json.loads(legacy_path.read_text(encoding="utf-8"))
    version = companion.get("schemaVersion")
    expected = {"schemaVersion", "version", "dataSchemaVersion", "distributionSha256", "base", "supplements", "sources", "license", "attribution"}
    if type(version) is not int or version != 1 or set(companion) != expected or companion.get("version") != manifest["version"] or companion.get("dataSchemaVersion") != manifest["dataSchemaVersion"] or companion.get("distributionSha256") != manifest["sha256"] or any(companion.get(k) != manifest.get(k) for k in ("license", "attribution")):
        raise ValueError("補足 manifest と地点 manifest の版・出典・ハッシュが一致しません")
    for key, name in zip(("base", "supplements"), COMPANION_FILES):
        descriptor = companion.get(key, {})
        payload = (directory / name).read_bytes()
        url = manifest["downloadUrl"].rsplit("/", 1)[0] + "/" + name
        if descriptor != {"fileName": name, "downloadUrl": url, "sizeBytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}:
            raise ValueError("補足ファイルの取得先・サイズ・ハッシュが一致しません")
    data = read_supplements(directory / COMPANION_FILES[1])
    if tags is not None and tags != data.get("tags", {}):
        raise ValueError("manifest と補足 JSON の tags が一致しません")
    if (version == 1 and companion.get("sources") != data.get("sources", {})) or (version == 2 and any(companion.get("sources", {}).get(k) != v for k, v in data.get("sources", {}).items())):
        raise ValueError("補足の出典が manifest と一致しません")
    with gzip.open(directory / COMPANION_FILES[0], "rt", encoding="utf-8") as stream:
        if __package__:
            from .point_document import rows
        else:
            from point_document import rows
        original = rows(json.load(stream))
    if isinstance(integrated, dict):
        if integrated.get("tags", {}) != data.get("tags", {}):
            raise ValueError("地点 JSON と補足 JSON のタグ情報が一致しません")
        integrated = rows(integrated, 6)
    from document_metadata import distributed_point, ordered_point
    if [distributed_point(p) for p in apply_supplements(original, data)] != [distributed_point(p) for p in integrated]:
        raise ValueError("元データと補足から配布データを再現できません")


def complete_supplement(input_path, points_path, output_path):
    """欠けた osmId をメモ中の山名から補い、入力を上書きせず保存する。"""
    input_path, points_path, output_path = map(Path, (input_path, points_path, output_path))
    if output_path.resolve() in (input_path.resolve(), points_path.resolve()):
        raise ValueError("入力と出力は別のファイルにしてください")
    data = json.loads(input_path.read_text(encoding="utf-8"))
    from scripts.tag_json import complete_rows, read_points
    rows = [{**row, "osmId": str(row.get("osmId", "")), "note": row.get("note", "")} for row in data["points"] if "id" not in row]
    id_rows = [row for row in data["points"] if "id" in row]
    points = read_points(points_path)
    apply_supplements(points, {"schemaVersion": 1, "points": id_rows})
    rows, report, unresolved = complete_rows(rows, [p for p in points if osm_node_id(p) is not None])
    completed = iter([{**row, **({"osmId": int(row["osmId"])} if row["osmId"] else {})} for row in rows])
    data["points"] = [row if "id" in row else next(completed) for row in data["points"]]
    for row in data["points"]:
        if row.get("osmId") == "":
            del row["osmId"]
    if not unresolved:
        validate_supplements(data)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(supplement_document(data), ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return report, unresolved


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--points", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=Path("build/supplements.json"))
    args = parser.parse_args()
    try:
        report, unresolved = complete_supplement(args.input, args.points, args.output)
    except (ValueError, OSError, KeyError, TypeError) as error:
        parser.exit(2, f"エラー: {error}\n")
    for row, status, candidates in report:
        print(f"{row['osmId']},{row['note']}: {status}")
    if unresolved:
        parser.exit(1, "未解決の地点があります。\n")

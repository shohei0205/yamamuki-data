"""単一の地点補足 JSON を検査し、元データを変更せずに適用する。"""

from copy import deepcopy
import gzip
import hashlib
import json
from pathlib import Path

DEFAULT_SUPPLEMENTS = Path(__file__).resolve().parents[1] / "supplements.json"
FIELDS = {"name", "nameReading", "aliases", "aliasesRemove", "tags", "tagsRemove", "exclude"}
METADATA = {"source", "license", "attribution", "changes"}
COMPANION_FILES = ("osm-peaks-source.json.gz", "supplements.json", "supplement-manifest.json")


def text(value):
    return isinstance(value, str) and bool(value) and value == value.strip()


def validate_supplement_sources(sources):
    if not isinstance(sources, dict):
        raise ValueError("supplementSources は出典 ID と出典情報の対応にしてください")
    for identifier, metadata in sources.items():
        if not text(identifier) or not isinstance(metadata, dict) or set(metadata) - METADATA or any(not isinstance(value, str) for value in metadata.values()):
            raise ValueError("supplementSources の出典 ID または出典情報が不正です")
    return sources


def validate_supplements(data):
    if not isinstance(data, dict) or set(data) - {"schemaVersion", "sources", "points"} or type(data.get("schemaVersion")) is not int or data["schemaVersion"] != 1:
        raise ValueError("地点補足 JSON の schemaVersion は 1 にしてください")
    sources = data.get("sources", {})
    if not isinstance(sources, dict):
        raise ValueError("sources は出典 ID と出典情報の対応にしてください")
    for key, source in sources.items():
        if not text(key) or not isinstance(source, dict) or set(source) - METADATA or any(not isinstance(v, str) for v in source.values()):
            raise ValueError("出典 ID または出典情報が不正です")
    if not isinstance(data.get("points"), list):
        raise ValueError("points は配列にしてください")
    seen = set()
    for row in data["points"]:
        if not isinstance(row, dict) or set(row) - {"osmId", "note", "reason", "expected", *FIELDS}:
            raise ValueError("地点補足の項目が不正です")
        identifier = row.get("osmId")
        if type(identifier) is not int or not 0 < identifier <= 9007199254740991 or identifier in seen:
            raise ValueError("osmId は重複のない正の整数にしてください")
        seen.add(identifier)
        if not set(row) & FIELDS:
            raise ValueError("地点補足には補足・補正する項目が必要です")
        for key in ("note", "reason"):
            if key in row and not isinstance(row[key], str):
                raise ValueError(f"{key} は文字列にしてください")
        if "name" in row and not text(row["name"]):
            raise ValueError("補正する地点名は空でない文字列にしてください")
        if "nameReading" in row and row["nameReading"] is not None and not text(row["nameReading"]):
            raise ValueError("よみがなは空でない文字列または null にしてください")
        if "exclude" in row and type(row["exclude"]) is not bool:
            raise ValueError("exclude は真偽値にしてください")
        for key in ("tags", "tagsRemove", "aliases", "aliasesRemove"):
            if key in row:
                values = row[key]
                if not isinstance(values, list) or any(not text(v) for v in values) or len(set(values)) != len(values):
                    raise ValueError(f"{key} は重複のない空でない文字列の配列にしてください")
        if set(row.get("tags", [])) & set(row.get("tagsRemove", [])):
            raise ValueError("同じタグを追加と削除の両方に指定できません")
        if set(row.get("aliases", [])) & set(row.get("aliasesRemove", [])):
            raise ValueError("同じ別名を追加と削除の両方に指定できません")
        if ("name" in row or row.get("exclude") or ("nameReading" in row and row["nameReading"] is None) or row.get("aliasesRemove")) and not text(row.get("reason")):
            raise ValueError("地点名の補正・除外・削除には reason に理由を書いてください")
        expected = row.get("expected", {})
        if not isinstance(expected, dict) or set(expected) - {"name", "nameReading", "aliases"}:
            raise ValueError("expected は補正前の地点名・よみがな・別名を指定してください")
        for key, value in expected.items():
            if key == "aliases":
                if not isinstance(value, list) or any(not isinstance(v, str) for v in value):
                    raise ValueError("expected.aliases は文字列の配列にしてください")
            elif value is not None and not isinstance(value, str):
                raise ValueError("expected の名前・よみがなが不正です")
    return data


def read_supplements(path=DEFAULT_SUPPLEMENTS):
    return validate_supplements(json.loads(Path(path).read_text(encoding="utf-8")))


def apply_supplements(points, data):
    validate_supplements(data)
    result = deepcopy(points)
    by_id = {}
    for point in result:
        identifier = point.get("osmId")
        if identifier in by_id:
            raise ValueError("地点データの osmId が重複しています")
        by_id[identifier] = point
    excluded = set()
    for row in data["points"]:
        identifier = row["osmId"]
        if identifier not in by_id:
            raise ValueError(f"osmId {identifier} が地点データにありません")
        point = by_id[identifier]
        for key, expected in row.get("expected", {}).items():
            actual = point.get(key, [] if key == "aliases" else None)
            if actual != expected:
                raise ValueError(f"osmId {identifier} の {key} が想定値と異なります。補正内容を確認してください")
        for key in ("name", "nameReading"):
            if key in row:
                if row[key] is None:
                    point.pop(key, None)
                else:
                    point[key] = row[key]
        if "tags" in row or "tagsRemove" in row:
            point["tags"] = list(dict.fromkeys([v for v in (point.get("tags") or []) if v not in row.get("tagsRemove", [])] + row.get("tags", [])))
        if "aliases" in row or "aliasesRemove" in row:
            removed = set(row.get("aliasesRemove", []))
            point["aliases"] = list(dict.fromkeys([v for v in (point.get("aliases") or []) if v not in removed] + row.get("aliases", [])))
        if row.get("exclude"):
            excluded.add(identifier)
    return [point for point in result if point.get("osmId") not in excluded]


def write_companions(directory, original, data, manifest):
    """既存 manifest と配布 gzip を保ち、編集用の元データと補足を同じ版に固定する。"""
    directory = Path(directory)
    raw = (json.dumps(original, ensure_ascii=False, separators=(",", ":"), allow_nan=False) + "\n").encode("utf-8")
    (directory / COMPANION_FILES[0]).write_bytes(gzip.compress(raw, mtime=0))
    (directory / COMPANION_FILES[1]).write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    def descriptor(name):
        payload = (directory / name).read_bytes()
        return {"fileName": name, "downloadUrl": manifest["downloadUrl"].rsplit("/", 1)[0] + "/" + name,
                "sizeBytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}
    companion = {"schemaVersion": 1, "version": manifest["version"], "dataSchemaVersion": manifest["dataSchemaVersion"],
                 "distributionSha256": manifest["sha256"], "base": descriptor(COMPANION_FILES[0]),
                 "supplements": descriptor(COMPANION_FILES[1]), "sources": data.get("sources", {}),
                 "license": manifest["license"], "attribution": manifest["attribution"]}
    (directory / COMPANION_FILES[2]).write_text(json.dumps(companion, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return companion


def validate_companions(directory, manifest, integrated):
    directory = Path(directory)
    present = [(directory / name).exists() for name in COMPANION_FILES]
    if not any(present):
        return
    if not all(present):
        raise ValueError("元データ・補足・補足 manifest がそろっていません")
    companion = json.loads((directory / COMPANION_FILES[2]).read_text(encoding="utf-8"))
    if type(companion.get("schemaVersion")) is not int or companion.get("schemaVersion") != 1 or set(companion) != {"schemaVersion", "version", "dataSchemaVersion", "distributionSha256", "base", "supplements", "sources", "license", "attribution"} or companion.get("version") != manifest["version"] or companion.get("dataSchemaVersion") != manifest["dataSchemaVersion"] or companion.get("distributionSha256") != manifest["sha256"] or any(companion.get(key) != manifest[key] for key in ("license", "attribution")):
        raise ValueError("補足 manifest と地点 manifest の版・出典・ハッシュが一致しません")
    for key, name in zip(("base", "supplements"), COMPANION_FILES):
        descriptor = companion.get(key, {})
        payload = (directory / name).read_bytes()
        url = manifest["downloadUrl"].rsplit("/", 1)[0] + "/" + name
        if descriptor != {"fileName": name, "downloadUrl": url, "sizeBytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}:
            raise ValueError("補足ファイルの取得先・サイズ・ハッシュが一致しません")
    data = read_supplements(directory / COMPANION_FILES[1])
    if companion.get("sources") != data.get("sources", {}):
        raise ValueError("補足の出典が manifest と一致しません")
    with gzip.open(directory / COMPANION_FILES[0], "rt", encoding="utf-8") as stream:
        original = json.load(stream)
    if apply_supplements(original, data) != integrated:
        raise ValueError("元データと補足から配布データを再現できません")


def complete_supplement(input_path, points_path, output_path):
    """欠けた osmId をメモ中の山名から補い、入力を上書きせず保存する。"""
    input_path, points_path, output_path = map(Path, (input_path, points_path, output_path))
    if output_path.resolve() in (input_path.resolve(), points_path.resolve()):
        raise ValueError("入力と出力は別のファイルにしてください")
    data = json.loads(input_path.read_text(encoding="utf-8"))
    from scripts.tag_json import complete_rows, read_points
    rows = [{**row, "osmId": str(row.get("osmId", "")), "note": row.get("note", "")} for row in data["points"]]
    rows, report, unresolved = complete_rows(rows, read_points(points_path))
    data["points"] = [{**row, **({"osmId": int(row["osmId"])} if row["osmId"] else {})} for row in rows]
    for row in data["points"]:
        if row.get("osmId") == "":
            del row["osmId"]
    if not unresolved:
        validate_supplements(data)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
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

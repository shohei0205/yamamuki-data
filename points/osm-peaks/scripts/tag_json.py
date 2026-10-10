"""地点補足 JSON の検査・適用と osmId / 山名の双方向補完。"""

import argparse
import gzip
import json
from pathlib import Path

DEFAULT_TAGS_DIRECTORY = Path(__file__).resolve().parents[1] / "tags"
DEFAULT_OUTPUT_DIRECTORY = Path(__file__).resolve().parents[1] / "build" / "tags"


METADATA_KEYS = ("source", "license", "attribution", "changes")


def validate_legacy_sources(sources):
    if not isinstance(sources, list):
        raise ValueError("旧形式の出典一覧 は配列にしてください")
    seen = set()
    for source in sources:
        if not isinstance(source, dict) or set(source) - {"tag", "name", *METADATA_KEYS}:
            raise ValueError("旧形式の出典一覧 の項目が不正です")
        if ("tag" in source) == ("name" in source):
            raise ValueError("旧形式の出典一覧 は tag と name のいずれか一方を指定してください")
        tag = source.get("tag", source.get("name"))
        if not isinstance(tag, str) or not tag.strip() or tag != tag.strip() or tag in seen:
            raise ValueError("旧形式の出典一覧 の識別名が不正または重複しています")
        seen.add(tag)
        if any(not isinstance(source[key], str) for key in METADATA_KEYS if key in source):
            raise ValueError("旧形式の出典一覧 の出典・ライセンス等は文字列にしてください")


def read_tag_json(path, *, complete=False):
    path = Path(path)
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict) or set(data) - {"tag", "points", *METADATA_KEYS}:
        raise ValueError(f"{path}: 地点補足 JSON の項目が不正です")
    tag = data.get("tag")
    if "tag" in data and (not isinstance(tag, str) or not tag.strip() or tag != tag.strip() or tag != path.stem):
        raise ValueError(f"{path}: tag は拡張子を除いたファイル名に合わせてください")
    metadata = {key: data[key] for key in METADATA_KEYS if key in data}
    if any(not isinstance(value, str) for value in metadata.values()):
        raise ValueError(f"{path}: 出典・ライセンス等は文字列にしてください")
    if not isinstance(data.get("points"), list):
        raise ValueError(f"{path}: points は配列にしてください")
    result, seen = [], set()
    for i, row in enumerate(data["points"], 1):
        if not isinstance(row, dict) or set(row) - {"osmId", "note", "nameReading", "aliases"}:
            raise ValueError(f"{path}: {i} 件目の項目が不正です")
        identifier, note = row.get("osmId"), row.get("note", "")
        if identifier is not None and (type(identifier) is not int or not 0 < identifier <= 9007199254740991):
            raise ValueError(f"{path}: osmId は正の整数にしてください")
        if not isinstance(note, str):
            raise ValueError(f"{path}: note は文字列にしてください")
        if (identifier is None and not note) or (complete and identifier is None):
            raise ValueError(f"{path}: osmId または山名が空です")
        if identifier in seen:
            raise ValueError(f"{path}: osmId {identifier} が重複しています")
        if identifier is not None:
            seen.add(identifier)
        extra = {key: row[key] for key in ("nameReading", "aliases") if key in row}
        if "nameReading" in extra and (not isinstance(extra["nameReading"], str) or not extra["nameReading"] or extra["nameReading"] != extra["nameReading"].strip()):
            raise ValueError(f"{path}: よみがなは空でない文字列にしてください")
        if "aliases" in extra:
            aliases = extra["aliases"]
            if not isinstance(aliases, list) or any(not isinstance(alias, str) or not alias or alias != alias.strip() for alias in aliases) or len(set(aliases)) != len(aliases):
                raise ValueError(f"{path}: 別名は重複のない空でない文字列の配列にしてください")
        if tag is None and not extra:
            raise ValueError(f"{path}: タグなしの地点には nameReading または aliases が必要です")
        result.append({"osmId": str(identifier) if identifier is not None else "", "note": note, **extra})
    return tag, result, metadata


def index_points(points):
    by_id, by_name = {}, {}
    for point in points:
        identifier = point.get("osmId")
        if identifier is None:
            continue
        if type(identifier) is not int or identifier <= 0 or identifier in by_id:
            raise ValueError("地点データの osmId が不正、または重複しています")
        by_id[identifier] = point
        for name in set([point["name"], *(point.get("aliases") or [])]):
            by_name.setdefault(name, []).append(point)
    return by_id, by_name


def apply_tag_jsons(points, directory=DEFAULT_TAGS_DIRECTORY):
    """全件の検査後にタグ・別名を追記し、指定されたよみがなを適用する。"""
    directory = Path(directory)
    if not directory.is_dir():
        raise ValueError(f"地点補足のフォルダがありません: {directory}")
    by_id, _ = index_points(points)
    additions, sources, readings = [], [], {}
    for path in sorted(directory.glob("*.json")):
        tag, rows, metadata = read_tag_json(path, complete=True)
        sources.append({**({"tag": tag} if tag is not None else {"name": path.stem}), **metadata})
        for row in rows:
            identifier = int(row["osmId"])
            if identifier not in by_id:
                raise ValueError(f"{path}: osmId {identifier} が山頂データにありません")
            point = by_id[identifier]
            if "nameReading" in row:
                if identifier in readings and readings[identifier] != row["nameReading"]:
                    raise ValueError(f"{path}: osmId {identifier} のよみがなが競合しています")
                readings[identifier] = row["nameReading"]
            additions.append((point, tag, row))
    for point, tag, row in additions:
        tags = point.get("tags") or []
        if tag is not None and tag not in tags:
            point["tags"] = [*tags, tag]
        if "nameReading" in row:
            point["nameReading"] = row["nameReading"]
        if "aliases" in row:
            point["aliases"] = list(dict.fromkeys([*(point.get("aliases") or []), *row["aliases"]]))

    return sources

def complete_rows(rows, points):
    by_id, by_name = index_points(points)
    completed, report, used = [], [], set()
    unresolved = False
    for row in rows:
        row = dict(row)
        candidates = []
        status = "確認済み"
        if row["osmId"]:
            point = by_id.get(int(row["osmId"]))
            if point is None:
                status, unresolved = "ID が見つかりません", True
            else:
                candidates = [point]
                if not row["note"]:
                    row["note"] = point["name"]
                    status = "山名を補完"
        else:
            candidates = by_name.get(row["note"], [])
            if len(candidates) == 1:
                row["osmId"] = str(candidates[0]["osmId"])
                status = "osmId を補完"
            else:
                status = "候補が複数あります" if candidates else "山名が見つかりません"
                unresolved = True
        if row["osmId"]:
            if row["osmId"] in used:
                raise ValueError(f"補完後の osmId {row['osmId']} が重複しています")
            used.add(row["osmId"])
        completed.append(row)
        report.append((row, status, candidates))
    return completed, report, unresolved


def read_points(points_path):
    points_path = Path(points_path)
    opener = gzip.open if points_path.suffix == ".gz" else open
    with opener(points_path, "rt", encoding="utf-8") as stream:
        points = json.load(stream)
    if not isinstance(points, list):
        raise ValueError("地点データは JSON 配列にしてください")
    return points


def process_json(tag_path, points_path, output_directory=DEFAULT_OUTPUT_DIRECTORY):
    tag_path, points_path = Path(tag_path), Path(points_path)
    output = Path(output_directory) / tag_path.name
    if output.resolve() in (tag_path.resolve(), points_path.resolve()):
        raise ValueError("入力ファイルと出力先を同じ場所にしないでください")
    tag, input_rows, metadata = read_tag_json(tag_path)
    rows, report, unresolved = complete_rows(input_rows, read_points(points_path))
    output_rows = []
    for row in rows:
        output_row = dict(row)
        if row["osmId"]:
            output_row["osmId"] = int(row["osmId"])
        else:
            output_row.pop("osmId")
        output_rows.append(output_row)
    data = {**({"tag": tag} if tag is not None else {}), **metadata, "points": output_rows}
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return output, report, unresolved


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag_json", type=Path)
    parser.add_argument("--points", type=Path, required=True, help="展開済み JSON または gzip")
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT_DIRECTORY)
    args = parser.parse_args()
    try:
        output, report, unresolved = process_json(args.tag_json, args.points, args.output_dir)
    except (ValueError, OSError) as error:
        parser.exit(2, f"エラー: {error}\n")
    print(f"補完 JSON: {output}")
    for row, status, candidates in report:
        print(f"{row['osmId']},{row['note']}: {status}")
        for point in candidates:
            print(f"  {point['osmId']},{point['name']}")
    if unresolved:
        parser.exit(1, "未解決の行があります。照合結果を確認してください。\n")


if __name__ == "__main__":
    main()

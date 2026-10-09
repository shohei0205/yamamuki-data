"""タグ別 JSON の検査と双方向補完。"""

import argparse
import gzip
import json
import logging
from pathlib import Path

DEFAULT_TAGS_DIRECTORY = Path(__file__).resolve().parents[1] / "tags"
DEFAULT_OUTPUT_DIRECTORY = Path(__file__).resolve().parents[1] / "build" / "tags"


METADATA_KEYS = ("source", "license", "attribution", "changes")


def validate_tag_sources(sources):
    if not isinstance(sources, list):
        raise ValueError("tagSources は配列にしてください")
    seen = set()
    for source in sources:
        if not isinstance(source, dict) or set(source) - {"tag", *METADATA_KEYS}:
            raise ValueError("tagSources の項目が不正です")
        tag = source.get("tag")
        if not isinstance(tag, str) or not tag.strip() or tag != tag.strip() or tag in seen:
            raise ValueError("tagSources のタグ名が不正または重複しています")
        seen.add(tag)
        if any(not isinstance(source[key], str) for key in METADATA_KEYS if key in source):
            raise ValueError("tagSources の出典・ライセンス等は文字列にしてください")


def read_tag_json(path, *, complete=False):
    path = Path(path)
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict) or set(data) - {"tag", "points", *METADATA_KEYS}:
        raise ValueError(f"{path}: タグ JSON の項目が不正です")
    tag = data.get("tag")
    if not isinstance(tag, str) or not tag.strip() or tag != tag.strip() or tag != path.stem:
        raise ValueError(f"{path}: tag は拡張子を除いたファイル名に合わせてください")
    metadata = {key: data[key] for key in METADATA_KEYS if key in data}
    if any(not isinstance(value, str) for value in metadata.values()):
        raise ValueError(f"{path}: 出典・ライセンス等は文字列にしてください")
    if not isinstance(data.get("points"), list):
        raise ValueError(f"{path}: points は配列にしてください")
    result, seen = [], set()
    for i, row in enumerate(data["points"], 1):
        if not isinstance(row, dict) or set(row) - {"osmId", "name"}:
            raise ValueError(f"{path}: {i} 件目の項目が不正です")
        identifier, name = row.get("osmId"), row.get("name", "")
        if identifier is not None and (type(identifier) is not int or not 0 < identifier <= 9007199254740991):
            raise ValueError(f"{path}: osmId は正の整数にしてください")
        if not isinstance(name, str) or name != name.strip():
            raise ValueError(f"{path}: 山名の形式が不正です")
        if (identifier is None and not name) or (complete and (identifier is None or not name)):
            raise ValueError(f"{path}: osmId または山名が空です")
        if identifier in seen:
            raise ValueError(f"{path}: osmId {identifier} が重複しています")
        if identifier is not None:
            seen.add(identifier)
        result.append({"osmId": str(identifier) if identifier is not None else "", "name": name})
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
    """検査をすべて通したあと、既存タグを保って追加する。"""
    directory = Path(directory)
    if not directory.is_dir():
        raise ValueError(f"タグのフォルダがありません: {directory}")
    by_id, _ = index_points(points)
    additions, sources = [], []
    for path in sorted(directory.glob("*.json")):
        tag, rows, metadata = read_tag_json(path, complete=True)
        sources.append({"tag": tag, **metadata})
        for row in rows:
            identifier = int(row["osmId"])
            if identifier not in by_id:
                raise ValueError(f"{path}: osmId {identifier} が山頂データにありません")
            point = by_id[identifier]
            if row["name"] not in [point["name"], *(point.get("aliases") or [])]:
                logging.warning("%s: osmId %s の山名が異なります: %s / %s", path, identifier, row["name"], point["name"])
            additions.append((point, tag))
    for point, tag in additions:
        tags = point.get("tags") or []
        if tag not in tags:
            point["tags"] = [*tags, tag]

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
                if not row["name"]:
                    row["name"] = point["name"]
                    status = "山名を補完"
                elif row["name"] not in [point["name"], *(point.get("aliases") or [])]:
                    status = "山名が異なります（要確認）"
        else:
            candidates = by_name.get(row["name"], [])
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
    data = {"tag": tag, **metadata, "points": [
        {**({"osmId": int(row["osmId"])} if row["osmId"] else {}), "name": row["name"]} for row in rows
    ]}
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
        print(f"{row['osmId']},{row['name']}: {status}")
        for point in candidates:
            print(f"  {point['osmId']},{point['name']}")
    if unresolved:
        parser.exit(1, "未解決の行があります。照合結果を確認してください。\n")


if __name__ == "__main__":
    main()

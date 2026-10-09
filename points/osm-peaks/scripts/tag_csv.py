"""タグ別 CSV の検査・双方向補完と、確認用 HTML の生成。"""

import argparse
import csv
import gzip
import html
import json
import logging
from pathlib import Path
import re

DEFAULT_TAGS_DIRECTORY = Path(__file__).resolve().parents[1] / "tags"
DEFAULT_OUTPUT_DIRECTORY = Path(__file__).resolve().parents[1] / "build" / "tags"


def read_csv(path, *, complete=False):
    path = Path(path)
    if not path.stem or path.stem != path.stem.strip():
        raise ValueError(f"{path}: ファイル名の前後に空白を入れないでください")
    with path.open(encoding="utf-8-sig", newline="") as stream:
        reader = csv.DictReader(stream, strict=True)
        columns = reader.fieldnames
        if columns and columns[0].startswith("#"):
            columns[0] = columns[0][1:]
        if (not columns or len(columns) != len(set(columns))
                or not set(columns) <= {"osmId", "name"}
                or (complete and set(columns) != {"osmId", "name"})):
            raise ValueError(f"{path}: 列は osmId,name にしてください（補完時は片方の列だけでも可）")
        result, seen = [], set()
        for row in reader:
            context = f"{path}:{reader.line_num}"
            if None in row or any(value is None for value in row.values()):
                raise ValueError(f"{context}: 列数が一致しません")
            identifier, name = row.get("osmId", ""), row.get("name", "")
            if identifier and not re.fullmatch(r"[1-9][0-9]*", identifier):
                raise ValueError(f"{context}: osmId は正の整数にしてください")
            if name != name.strip():
                raise ValueError(f"{context}: 山名の前後に空白を入れないでください")
            if not identifier and not name or complete and (not identifier or not name):
                raise ValueError(f"{context}: osmId または山名が空です")
            if identifier:
                if identifier in seen:
                    raise ValueError(f"{context}: osmId {identifier} が重複しています")
                seen.add(identifier)
            result.append({"osmId": identifier, "name": name})
        return result


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


def apply_tag_csvs(points, directory=DEFAULT_TAGS_DIRECTORY):
    """検査をすべて通したあと、既存タグを保って追加する。"""
    directory = Path(directory)
    if not directory.is_dir():
        raise ValueError(f"タグのフォルダがありません: {directory}")
    by_id, _ = index_points(points)
    additions = []
    for path in sorted(directory.glob("*.csv")):
        for row in read_csv(path, complete=True):
            identifier = int(row["osmId"])
            if identifier not in by_id:
                raise ValueError(f"{path}: osmId {identifier} が山頂データにありません")
            point = by_id[identifier]
            if row["name"] not in [point["name"], *(point.get("aliases") or [])]:
                logging.warning("%s: osmId %s の山名が異なります: %s / %s", path, identifier, row["name"], point["name"])
            additions.append((point, path.stem))
    for point, tag in additions:
        tags = point.get("tags") or []
        if tag not in tags:
            point["tags"] = [*tags, tag]


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


def make_html(tag, report):
    entries, table_rows = [], []
    for row, status, candidates in report:
        for point in candidates or [None]:
            identifier = str(point["osmId"]) if point else row["osmId"]
            url = f"https://www.openstreetmap.org/node/{identifier}" if identifier else ""
            entry = {
                "osmId": identifier, "name": row["name"], "status": status,
                "pointName": point["name"] if point else "",
                "latitude": point["latitude"] if point else None,
                "longitude": point["longitude"] if point else None,
                "elevation": point.get("elevationM") if point else None,
                "source": point, "tags": (point.get("tags") or []) if point else [],
                "url": url, "review": len(candidates) > 1 or "要確認" in status,
            }
            entries.append(entry)
            escape = lambda value: html.escape(str(value), quote=True)
            name = escape(row["name"])
            if point:
                name = f'<button type="button" class="point-link">{name}</button>'
            coordinates = f"{point['latitude']}, {point['longitude']}" if point else ""
            elevation = entry["elevation"] if entry["elevation"] is not None else ""
            values = [elevation, "", coordinates]
            cells = "".join(f"<td>{escape(value)}</td>" for value in values)
            link = f'<a href="{escape(url)}" target="_blank" rel="noopener noreferrer">{escape(identifier)}</a>' if url else ""
            table_rows.append(f"<tr><td>{name}</td>{cells}<td>{link or escape(identifier)}</td></tr>")
    template = (Path(__file__).resolve().parents[2] / "viewer.html").read_text(encoding="utf-8")
    template = template.replace("<title>地点データビューア", "<title>__TITLE__").replace("<h1>地点データビューア", "<h1>__TITLE__")
    template = template.replace("<tbody></tbody>", "<tbody>__ROWS__</tbody>")
    template = template.replace("let points=[];", "let points=JSON.parse(document.getElementById('points').textContent);")
    template = template.replace("<script>\n", '<script id="points" type="application/json">__DATA__</script>\n<script>\n', 1)
    template = template.replace("renderView(points);", "renderView(points,false);")
    payload = json.dumps(entries, ensure_ascii=False, allow_nan=False).replace("&", "\\u0026").replace("<", "\\u003c").replace(">", "\\u003e")
    # 入力に置換用の文字列があっても再解釈しない。
    replacements = {"__TITLE__": html.escape(tag), "__ROWS__": "\n".join(table_rows), "__DATA__": payload}
    return re.sub(r"__TITLE__|__ROWS__|__DATA__", lambda match: replacements[match[0]], template)


def read_points(points_path):
    points_path = Path(points_path)
    opener = gzip.open if points_path.suffix == ".gz" else open
    with opener(points_path, "rt", encoding="utf-8") as stream:
        points = json.load(stream)
    if not isinstance(points, list):
        raise ValueError("地点データは JSON 配列にしてください")
    return points


def process_all_points(points_path, output_directory=DEFAULT_OUTPUT_DIRECTORY):
    """CSV を使わず、山頂データ全件の確認用 HTML を生成する。"""
    output = Path(output_directory) / "osm-peaks.html"
    if output.resolve() == Path(points_path).resolve():
        raise ValueError("地点データを出力先にしないでください")
    points = read_points(points_path)
    index_points(points)
    if any(point.get("osmId") is None for point in points):
        raise ValueError("全地点表示には全地点の osmId が必要です")
    report = [({"osmId": str(point["osmId"]), "name": point["name"]}, "データ収録地点", [point]) for point in points]
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(make_html("osm-peaks 全地点", report), encoding="utf-8", newline="\n")
    return output


def process_csv(csv_path, points_path, output_directory=DEFAULT_OUTPUT_DIRECTORY):
    csv_path, points_path = Path(csv_path), Path(points_path)
    output_directory = Path(output_directory)
    output_csv = output_directory / csv_path.name
    output_html = output_directory / (csv_path.stem + ".html")
    if output_csv.resolve() == csv_path.resolve():
        raise ValueError("入力 CSV と出力先を同じ場所にしないでください")
    if points_path.resolve() in (output_csv.resolve(), output_html.resolve()):
        raise ValueError("地点データを出力先にしないでください")
    points = read_points(points_path)
    rows, report, unresolved = complete_rows(read_csv(csv_path), points)
    output_directory.mkdir(parents=True, exist_ok=True)
    with output_csv.open("w", encoding="utf-8-sig", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=["osmId", "name"], lineterminator="\n")
        stream.write("#")
        writer.writeheader()
        writer.writerows(rows)
    output_html.write_text(make_html(csv_path.stem, report), encoding="utf-8", newline="\n")
    return output_csv, output_html, unresolved


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("csv", type=Path, nargs="?")
    parser.add_argument("--all", action="store_true", help="CSV を使わず全地点の HTML を生成")
    parser.add_argument("--viewer", action="store_true", help="ブラウザでファイルを選択する汎用ビューアを生成")
    parser.add_argument("--points", type=Path, help="展開済み JSON または gzip")
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT_DIRECTORY)
    args = parser.parse_args()
    if sum((args.all, args.viewer, args.csv is not None)) != 1:
        parser.error("CSV・--all・--viewer のいずれか一つを指定してください")
    if not args.viewer and args.points is None:
        parser.error("--points を指定してください")
    if args.viewer and args.points is not None:
        parser.error("--viewer では --points を指定しないでください")
    try:
        if args.viewer:
            output = args.output_dir / "point-viewer.html"
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text((Path(__file__).resolve().parents[2] / "viewer.html").read_text(encoding="utf-8"), encoding="utf-8", newline="\n")
            print(f"地点データビューア: {output}")
            return
        if args.all:
            output = process_all_points(args.points, args.output_dir)
            print(f"全地点 HTML: {output}")
            return
        output_csv, output_html, unresolved = process_csv(args.csv, args.points, args.output_dir)
    except (ValueError, OSError, csv.Error) as error:
        parser.exit(2, f"エラー: {error}\n")
    print(f"補完 CSV: {output_csv}\n確認用 HTML: {output_html}")
    if unresolved:
        parser.exit(1, "未解決の行があります。HTML を確認してください。\n")


if __name__ == "__main__":
    main()

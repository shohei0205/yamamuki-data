"""公開前の山頂 Draft について、削除直前の状態を確認して整理する。"""

import argparse
from datetime import datetime, timedelta, timezone
import json
import os
import re

from release_tools import release_data as site


DATA_REPO = "shohei0205/yamamuki-data"
TAG = re.compile(r"(?:osm-peaks|gsi-peaks)-\d{8}T\d{6}Z-\d+-\d+")
JST = timezone(timedelta(hours=9))
RETENTION = timedelta(days=7)


def draft_date(release, now):
    """作成から 7 日以上経った公開前の Draft なら作成日を返す。"""
    if (release.get("draft") is not True or "published_at" not in release
            or release["published_at"] is not None
            or site.TEMPORARY_DRAFT_MARKER not in (release.get("body") or "").splitlines()
            or not TAG.fullmatch(release.get("tag_name", ""))):
        return None
    try:
        created = datetime.fromisoformat(release["created_at"])
        if created.tzinfo is None:
            return None
    except (KeyError, TypeError, ValueError):
        return None
    if now - created >= RETENTION:
        return created.astimezone(JST).date()
    return None


def list_candidates(now):
    eligible = [(created, release["tag_name"]) for release in site.releases()
                if (created := draft_date(release, now)) is not None]
    candidates = sorted(eligible)
    lines = ["## Draft の削除候補", ""]
    lines += [f"- `{tag}`（作成日 {created}）" for created, tag in candidates] or ["候補はありません。"]
    site.append_summary("\n".join(lines) + "\n")


def delete_candidate(tag, now):
    if (os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("GITHUB_REF") != "refs/heads/main"
            or os.environ.get("GH_REPO") != DATA_REPO):
        raise ValueError("削除はデータリポジトリの main の Action から実行してください")
    if not TAG.fullmatch(tag):
        raise ValueError("山頂データの Draft タグを 1 件指定してください")
    matches = [release for release in site.releases() if release.get("tag_name") == tag]
    if len(matches) != 1 or draft_date(matches[0], now) is None:
        raise ValueError("指定した Draft は削除条件を満たしていません")
    release_id = matches[0]["id"]
    if type(release_id) is not int or release_id <= 0:
        raise ValueError("Release ID が不正です")
    release = json.loads(site.gh("api", site.endpoint(f"releases/{release_id}")))
    if release.get("id") != release_id or release.get("tag_name") != tag or draft_date(release, now) is None:
        raise ValueError("指定した Release は削除条件を満たしていません")
    site.gh("api", "--method", "DELETE", site.endpoint(f"releases/{release_id}"))
    site.append_summary(f"## Draft の削除\n\n- 削除した Release: `{tag}`\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("list")
    commands.add_parser("delete-old")
    delete = commands.add_parser("delete")
    delete.add_argument("--tag", required=True)
    args = parser.parse_args()
    now = datetime.now(timezone.utc)
    if args.command == "list":
        list_candidates(now)
    elif args.command == "delete":
        delete_candidate(args.tag, now)
    else:
        tags = [release["tag_name"] for release in site.releases() if draft_date(release, now) is not None]
        for tag in tags:
            delete_candidate(tag, now)
        if not tags:
            site.append_summary("## Draft の整理\n\n対象はありません。\n")


if __name__ == "__main__":
    main()

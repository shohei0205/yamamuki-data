"""ビューワの配布名を生成日時と Actions の実行情報から決める。"""

from datetime import datetime, timezone
import os


def plan(now, run_id, attempt):
    utc = now.astimezone(timezone.utc)
    identifier = f"{utc:%Y%m%dT%H%M%SZ}-{run_id}-{attempt}"
    return {
        "tag": f"viewer-{identifier}",
        "title": f"YamamukiDataViewer-{identifier}",
        "version": f"{utc.year}.{utc.month}.{utc.day}",
    }


if __name__ == "__main__":
    values = plan(datetime.now(timezone.utc), os.environ["GITHUB_RUN_ID"],
                  os.environ["GITHUB_RUN_ATTEMPT"])
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        for key, value in values.items():
            print(f"{key}={value}", file=output)

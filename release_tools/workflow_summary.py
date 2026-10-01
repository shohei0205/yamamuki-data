"""実行パラメーターをデータの処理前に Actions Summary へ記録する。"""

import argparse
import html
import json
import os
import re


def cell(value):
    # 自由記入の理由に Markdown や改行が含まれても表を壊さない。
    value = re.sub(r"([\\`*_{}\[\]()#+.!~])", r"\\\1", str(value))
    value = html.escape(value, quote=True).replace("|", "&#124;")
    return value.replace("\r\n", "\n").replace("\r", "\n").replace("\n", "<br>")


def render(kind, inputs, environment):
    rows = [
        ("実行ブランチ", environment.get("GITHUB_REF_NAME", "不明")),
        ("実行イベント", environment.get("GITHUB_EVENT_NAME", "不明")),
        ("実行者", environment.get("GITHUB_ACTOR", "不明")),
        ("実行回数（再実行を含む）", environment.get("GITHUB_RUN_ATTEMPT", "不明")),
        ("配布先", environment.get("RELEASE_CHANNEL", "不明")),
    ]
    if kind == "generate":
        rows.append(("取得対象日（source_date）", inputs.get("source_date") or "未指定（latest を取得）"))
    elif kind == "publish":
        rows.extend([
            ("対象の URL またはタグ（tag）", inputs.get("tag") or "未指定"),
            ("指定した SHA-256（sha256）", inputs.get("sha256") or "未指定（検査結果から自動取得）"),
            ("確認内容・公開理由（reason）", inputs.get("reason") or "未記入"),
        ])
    else:
        raise ValueError("未対応のワークフローです")
    lines = ["## 実行パラメーター", "", "| 項目 | 値 |", "|---|---|"]
    lines.extend(f"| {cell(key)} | {cell(value)} |" for key, value in rows)
    return "\n".join(lines) + "\n\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("generate", "publish"))
    args = parser.parse_args()
    contents = render(args.kind, json.loads(os.environ.get("WORKFLOW_INPUTS", "{}")), os.environ)
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as stream:
        stream.write(contents)


if __name__ == "__main__":
    main()

"""地点に付ける任意の分類ラベルを検証する。"""


def validate_tags(rows):
    for row in rows:
        tags = row.get("tags", [])
        if (not isinstance(tags, list)
                or any(not isinstance(tag, str) or not tag or tag != tag.strip() for tag in tags)):
            raise ValueError("tags は前後の空白がない空でない文字列の配列にしてください")
        if len(tags) != len(set(tags)):
            raise ValueError("tags に重複があります")

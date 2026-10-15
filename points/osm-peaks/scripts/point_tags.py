"""地点に付ける任意の分類ラベルを検証する。"""


def validate_tags(rows):
    for row in rows:
        tags = row.get("tags")
        if tags is None:
            tags = []
        if (not isinstance(tags, list)
                or any(not isinstance(tag, str) or not tag or tag != tag.strip() for tag in tags)):
            raise ValueError("tags は前後の空白がない空でない文字列の配列にしてください")
        if len(tags) != len(set(tags)):
            raise ValueError("tags に重複があります")


def validate_alias_readings(rows):
    from pathlib import Path
    import sys
    sys.path.insert(0,str(Path(__file__).resolve().parents[2]))
    from document_metadata import alias_objects
    for row in rows:
        alias_objects(row.get("aliases"), row.get("aliasReadings"))


def validate_tag_metadata(info):
    if not isinstance(info, dict):
        raise ValueError("tags はタグ名と情報の対応にしてください")
    for tag, value in info.items():
        if (not isinstance(tag, str) or not tag or tag != tag.strip()
                or not isinstance(value, dict)):
            raise ValueError("tags のタグ名または項目が不正です")
        if "nameReading" in value and (not isinstance(value["nameReading"], str)
                or not value["nameReading"] or value["nameReading"] != value["nameReading"].strip()):
            raise ValueError("tags.nameReading は前後に空白のない空でない文字列にしてください")
    return info

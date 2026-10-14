"""配布地点のオリジナルと補足を、それぞれ共通 manifest 付きで配る。"""

from copy import deepcopy
import gzip
import hashlib
import json
from pathlib import Path

from scripts.point_document import document, rows
from scripts.supplements import apply_supplements, supplement_document
from document_metadata import ordered_point, validate_metadata


def filenames(dataset):
    return (dataset + "-source.json.gz", dataset + "-source-manifest.json", "supplements.json.gz", "supplements-manifest.json")


def write_parts(directory, original, supplements, manifest):
    directory = Path(directory)
    dataset = manifest["fileName"].removesuffix(".json.gz")
    original_rows = rows(original) if isinstance(original, dict) else original
    source_info = deepcopy(original.get("metadata", {})) if isinstance(original, dict) else {"sources": {k:v for k,v in manifest["metadata"].get("sources", {}).items() if k not in supplements.get("sources", {})}}
    source_info.pop("pointCount", None)
    tags = source_info.pop("tags", {})
    source_document = document(original_rows, tags, **source_info)
    supplement = supplement_document(supplements)
    if [ordered_point(p) for p in apply_supplements(original_rows, supplements)] != rows(json.loads(gzip.decompress((directory / manifest["fileName"]).read_bytes())), 6):
        raise ValueError("オリジナルと補足から配布地点を再現できません")
    names = filenames(dataset)
    for body, data_name, manifest_name, label in ((source_document, names[0], names[1], "（補足適用前）"), (supplement, names[2], names[3], "（補足）")):
        raw = (json.dumps(body, ensure_ascii=False, separators=(",", ":"), allow_nan=False) + "\n").encode("utf-8")
        payload = gzip.compress(raw, mtime=0)
        part = {"schemaVersion": 6, "dataSchemaVersion": body["schemaVersion"], "name": manifest["name"] + label, "version": manifest["version"], "dataTimestamp": manifest["dataTimestamp"], "fileName": data_name, "downloadUrl": manifest["downloadUrl"].rsplit("/", 1)[0] + "/" + data_name, "sha256": hashlib.sha256(payload).hexdigest(), "sizeBytes": len(payload), "uncompressedSizeBytes": len(raw), "metadata": deepcopy(body["metadata"])}
        (directory / data_name).write_bytes(payload)
        (directory / manifest_name).write_text(json.dumps(part, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return names


def validate_parts(directory, manifest, integrated):
    directory = Path(directory)
    names = filenames(manifest["fileName"].removesuffix(".json.gz"))
    present = [(directory / name).exists() for name in names]
    if not any(present):
        return
    if not all(present):
        raise ValueError("オリジナル・補足とそれぞれの manifest がそろっていません")
    documents = []
    for data_name, manifest_name in ((names[0], names[1]), (names[2], names[3])):
        part = json.loads((directory / manifest_name).read_bytes())
        payload = (directory / data_name).read_bytes()
        raw = gzip.decompress(payload)
        body = json.loads(raw)
        validate_metadata(part.get("metadata"))
        expected = {"schemaVersion":6, "dataSchemaVersion":body["schemaVersion"], "version":manifest["version"], "dataTimestamp":manifest["dataTimestamp"], "fileName":data_name, "downloadUrl":manifest["downloadUrl"].rsplit("/",1)[0]+"/"+data_name, "sha256":hashlib.sha256(payload).hexdigest(), "sizeBytes":len(payload), "uncompressedSizeBytes":len(raw), "metadata":body["metadata"]}
        if any(part.get(k) != v for k,v in expected.items()):
            raise ValueError("別配布 JSON と manifest が一致しません")
        documents.append(body)
    for body in documents:
        if any(manifest["metadata"].get("sources", {}).get(k) != v for k,v in body["metadata"].get("sources", {}).items()):
            raise ValueError("各配布データの出典がマージ済み manifest に保持されていません")
    source = rows(documents[0], 6)
    # 補足の通常読み込みと同じ検証を通す。
    from scripts.supplements import validate_supplements
    supplement = validate_supplements(documents[1])
    if [ordered_point(p) for p in apply_supplements(source, supplement)] != [ordered_point(p) for p in integrated]:
        raise ValueError("オリジナルと補足から配布地点を再現できません")

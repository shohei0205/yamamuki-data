"""最適化ベクトルタイルの PMTiles 元ファイルを、途中から再開して保存する。"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import time
from urllib.request import Request, urlopen

URL = "https://cyberjapandata.gsi.go.jp/xyz/optimal_bvmap-v1/optimal_bvmap-v1.pmtiles"


def save(path, data):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def download(directory):
    directory.mkdir(parents=True, exist_ok=True)
    archive = directory / "optimal_bvmap-v1.pmtiles"
    partial = directory / "optimal_bvmap-v1.pmtiles.part"
    info = directory / "optimal_bvmap-source.json"
    if partial.exists() and not info.exists():
        raise ValueError("途中ファイルの取得記録がありません。版を確認できないため再開しません")
    if archive.exists() and info.exists():
        metadata = json.loads(info.read_bytes())
        with archive.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        if metadata["url"] != URL or metadata["sizeBytes"] != archive.stat().st_size or metadata.get("sha256") != digest:
            raise ValueError("保存済み PMTiles のハッシュまたは記録が一致しません")
        return archive
    with urlopen(Request(URL, method="HEAD"), timeout=60) as response:
        metadata = {"url": URL, "sizeBytes": int(response.headers["Content-Length"]),
                    "etag": response.headers.get("ETag"), "lastModified": response.headers.get("Last-Modified")}
    if info.exists():
        previous = json.loads(info.read_bytes())
        if any(previous.get(k) != metadata[k] for k in ("url", "etag", "lastModified", "sizeBytes")):
            raise ValueError("配信元が更新されています。前回の途中ファイルは残し、別の保存先で取得してください")
        metadata["fetchedAt"] = previous["fetchedAt"]
    else:
        metadata["fetchedAt"] = datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
    save(info, metadata)
    if archive.exists():
        if archive.stat().st_size != metadata["sizeBytes"]:
            raise ValueError("PMTiles の長さが配信元と異なります")
    else:
        for attempt in range(4):
            offset = partial.stat().st_size if partial.exists() else 0
            if offset == metadata["sizeBytes"]:
                break
            if offset > metadata["sizeBytes"]:
                raise ValueError("途中ファイルが配信元より大きくなっています")
            headers = {"Range": f"bytes={offset}-", "If-Match": metadata["etag"]} if offset else {}
            if not headers.get("If-Match"):
                headers.pop("If-Match", None)
            try:
                with urlopen(Request(URL, headers=headers), timeout=60) as response:
                    expected = 206 if offset else 200
                    if response.status != expected or response.headers.get("ETag") != metadata["etag"]:
                        raise ValueError("HTTP 応答またはデータの版が変わりました")
                    if offset and response.headers.get("Content-Range") != f"bytes {offset}-{metadata['sizeBytes'] - 1}/{metadata['sizeBytes']}":
                        raise ValueError("再開位置が一致しません")
                    tick = time.monotonic()
                    with partial.open("ab") as target:
                        while data := response.read(8 * 1024 * 1024):
                            target.write(data)
                            if time.monotonic() - tick >= 10:
                                print(f"PMTiles 取得: {target.tell()}/{metadata['sizeBytes']} バイト", flush=True)
                                tick = time.monotonic()
                if partial.stat().st_size != metadata["sizeBytes"]:
                    raise OSError("取得が途中で終了しました")
                break
            except OSError:
                if attempt == 3:
                    raise
                time.sleep(2 ** attempt)
        partial.replace(archive)
    with archive.open("rb") as source:
        metadata["sha256"] = hashlib.file_digest(source, "sha256").hexdigest()
    save(info, metadata)
    print(f"PMTiles 保存完了: {archive}（{metadata['sizeBytes']} バイト）", flush=True)
    return archive


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=Path("build/gsi-vector-cache"))
    download(parser.parse_args().cache)

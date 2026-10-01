"""全国 PBF を日付付き URL から取得し、中断時は続きから再開する。"""

import argparse
import hashlib
import http.client
import json
import logging
from pathlib import Path
import re
import time
import urllib.error
import urllib.parse
import urllib.request


SOURCE_URL = "https://download.geofabrik.de/asia/japan-latest.osm.pbf"
RETRY_ERRORS = (OSError, urllib.error.URLError, http.client.HTTPException)


def request(url, *, method="GET", headers=None, timeout=60):
    return urllib.request.urlopen(
        urllib.request.Request(url, method=method, headers=headers or {}), timeout=timeout,
    )


def resolve_source():
    logging.info("元データの日付付き URL とサイズを確認しています")
    with request(SOURCE_URL, method="HEAD") as response:
        # latest の更新をまたいでも、異なる版のデータをつなげない。
        url = response.url.rstrip("/")
        parsed = urllib.parse.urlparse(url)
        if (parsed.scheme != "https" or parsed.netloc != "download.geofabrik.de"
                or not re.fullmatch(r"/asia/japan-\d{6}\.osm\.pbf", parsed.path)):
            raise ValueError(f"日付付きの全国 PBF に転送されませんでした: {url}")
        size = int(response.headers["Content-Length"])
        if size <= 0:
            raise ValueError("元データのサイズが不正です")
    logging.info("配布元の MD5 を取得しています: %s", url + ".md5")
    with request(url + ".md5") as response:
        fields = response.read(4096).decode("ascii").split()
        if not fields or not re.fullmatch(r"[0-9a-fA-F]{32}", fields[0]):
            raise ValueError("元データの MD5 が不正です")
        checksum = fields[0].lower()
    return {"url": url, "sizeBytes": size, "md5": checksum}


def matches(path, source):
    if not path.exists() or path.stat().st_size != source["sizeBytes"]:
        return False
    with path.open("rb") as stream:
        logging.info("MD5 を照合しています: %s (%s バイト)", path, format(source["sizeBytes"], ","))
        matched = hashlib.file_digest(stream, "md5").hexdigest() == source["md5"]
        logging.info("MD5 照合: %s", "一致" if matched else "不一致")
        return matched


def transfer(source, partial, *, timeout=60, deadline=None):
    size = source["sizeBytes"]
    offset = partial.stat().st_size if partial.exists() else 0
    if offset == size:
        return
    if offset > size:
        raise ValueError("途中ファイルが元データより大きくなっています")
    headers = {"Range": f"bytes={offset}-"} if offset else {}
    logging.info("接続中: %s（取得済み %s / %s バイト、通信待ち上限 %.0f 秒）",
                 source["url"], format(offset, ","), format(size, ","), timeout)
    with request(source["url"], headers=headers, timeout=timeout) as response:
        mode = "ab"
        if response.status == 206:
            expected = f"bytes {offset}-{size - 1}/{size}"
            if response.headers.get("Content-Range") != expected:
                raise ValueError("再開位置または全体サイズが応答と一致しません")
        elif response.status == 200:
            # Range を無視するサーバーでは、全体を上書きで取り直す。
            if offset:
                logging.info("再開位置が応答に反映されなかったため、先頭から取り直します")
            mode, offset = "wb", 0
        else:
            raise ValueError(f"予期しない応答です: {response.status}")
        if int(response.headers.get("Content-Length", size - offset)) != size - offset:
            raise ValueError("応答のサイズが元データと一致しません")
        logging.info("受信開始: HTTP %s、開始位置 %s バイト", response.status, format(offset, ","))
        started = last_report = time.monotonic()
        start_offset = offset
        with partial.open(mode) as stream:
            # 少量ずつしか届かない場合も、読み取りの合間に全体の時間を確認する。
            while chunk := response.read1(1024 * 1024):
                if deadline is not None and time.monotonic() >= deadline:
                    raise TimeoutError("ダウンロード全体の制限時間に達しました")
                if offset + len(chunk) > size:
                    raise ValueError("元データのサイズを超える応答です")
                stream.write(chunk)
                offset += len(chunk)
                now = time.monotonic()
                if now - last_report >= 15:
                    speed = (offset - start_offset) / (now - started) / 1_000_000
                    logging.info("取得済み: %s / %s バイト (%.1f%%)、平均 %.2f MB/秒、経過 %.0f 秒",
                                 format(offset, ","), format(size, ","), offset / size * 100, speed, now - started)
                    last_report = now
    if offset != size:
        raise OSError(f"転送が途中で終了しました: {offset:,} / {size:,} バイト")
    logging.info("受信完了: %s バイト", format(offset, ","))


def download(output, *, attempts=6, retry_delay=15, max_seconds=3600):
    logging.info("全国データの取得を開始します（全体の上限 %s 秒）", max_seconds)
    deadline = time.monotonic() + max_seconds
    output = Path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    source = None
    for attempt in range(attempts):
        try:
            logging.info("取得の試行 %s/%s", attempt + 1, attempts)
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("ダウンロード全体の制限時間に達しました")
            if source is None:
                source = resolve_source()
                logging.info("取得元: %s", json.dumps(source, ensure_ascii=False))
            if matches(output, source):
                logging.info("取得済みの同じデータを再利用します: %s", output)
                return source
            # 日付とチェックサムで途中ファイルを区別し、別の版を再利用しない。
            partial = output.with_name(f"{output.name}.{source['md5']}.part")
            if partial.exists() and partial.stat().st_size > source["sizeBytes"]:
                partial.unlink()
            transfer(source, partial, timeout=min(60, max(1, remaining)), deadline=deadline)
            if not matches(partial, source):
                partial.unlink()
                raise OSError("元データの MD5 が一致しないため、取り直します")
            partial.replace(output)
            logging.info("取得完了・MD5 照合成功: %s (%s バイト)", output, format(source["sizeBytes"], ","))
            return source
        except RETRY_ERRORS as exc:
            if attempt == attempts - 1 or time.monotonic() >= deadline:
                raise
            logging.warning("取得失敗: %s。最大 %s 秒待って再試行します (%s/%s)", exc, retry_delay, attempt + 2, attempts)
            time.sleep(min(retry_delay, max(0, deadline - time.monotonic())))
    raise RuntimeError("元データを取得できませんでした")


def main():
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(message)s", datefmt="%H:%M:%S")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("build/japan-latest.osm.pbf"))
    parser.add_argument("--max-seconds", type=int, default=3600)
    args = parser.parse_args()
    if args.max_seconds <= 0:
        parser.error("--max-seconds は正の整数で指定してください")
    download(args.output, max_seconds=args.max_seconds)


if __name__ == "__main__":
    main()

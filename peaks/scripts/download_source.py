"""全国 PBF を日付付き URL から取得し、中断時は続きから再開する。"""

import argparse
from contextlib import contextmanager
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


def response_headers(headers):
    # 接続やキャッシュの診断に使う項目だけを記録する。
    names = ("Location", "Content-Length", "Content-Range", "Content-Type", "Retry-After",
             "Date", "Age", "Cache-Control", "Cache-Status", "Via", "Server")
    return json.dumps({name: headers.get(name) for name in names if headers and headers.get(name) is not None},
                      ensure_ascii=False)


class LoggingRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        logging.info("HTTP 転送: %s %s → %s（HTTP %s、応答ヘッダー %s）",
                     req.get_method(), req.full_url, newurl, code, response_headers(headers))
        redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
        if redirected is not None:
            logging.info("HTTP 転送後の要求: %s %s", redirected.get_method(), redirected.full_url)
        return redirected


@contextmanager
def request(url, *, method="GET", headers=None, timeout=60):
    started = time.monotonic()
    response = None
    logging.info("HTTP 接続開始: %s %s（Range=%s、通信待ち上限 %.0f 秒）",
                 method, url, (headers or {}).get("Range", "なし"), timeout)
    try:
        opener = urllib.request.build_opener(LoggingRedirectHandler())
        with opener.open(urllib.request.Request(url, method=method, headers=headers or {}), timeout=timeout) as response:
            logging.info("HTTP 応答: %s %s → %s（HTTP %s、経過 %.1f 秒、応答ヘッダー %s）",
                         method, url, response.url, response.status, time.monotonic() - started,
                         response_headers(response.headers))
            yield response
    except urllib.error.HTTPError as exc:
        logging.error("HTTP エラー: %s %s → %s（HTTP %s、理由 %s、経過 %.1f 秒、応答ヘッダー %s）",
                      method, url, exc.url, exc.code, exc.reason, time.monotonic() - started,
                      response_headers(exc.headers))
        exc.close()
        raise
    except (*RETRY_ERRORS, ValueError) as exc:
        logging.error("通信・応答の処理に失敗: %s %s（応答URL=%s、HTTP=%s、%s: %s、経過 %.1f 秒）",
                      method, url, response.url if response is not None else "未取得",
                      response.status if response is not None else "未取得",
                      type(exc).__name__, exc, time.monotonic() - started)
        raise


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
        stage = "開始・時間制限の確認"
        try:
            logging.info("取得の試行 %s/%s", attempt + 1, attempts)
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("ダウンロード全体の制限時間に達しました")
            if source is None:
                stage = "取得先の日付・サイズ・MD5 の確認"
                source = resolve_source()
                logging.info("取得元: %s", json.dumps(source, ensure_ascii=False))
            stage = "保存済みファイルの確認"
            if matches(output, source):
                logging.info("取得済みの同じデータを再利用します: %s", output)
                return source
            # 日付とチェックサムで途中ファイルを区別し、別の版を再利用しない。
            partial = output.with_name(f"{output.name}.{source['md5']}.part")
            if partial.exists() and partial.stat().st_size > source["sizeBytes"]:
                partial.unlink()
            stage = "PBF の転送"
            transfer(source, partial, timeout=min(60, max(1, remaining)), deadline=deadline)
            stage = "取得ファイルの MD5 照合"
            if not matches(partial, source):
                partial.unlink()
                raise OSError("元データの MD5 が一致しないため、取り直します")
            stage = "取得ファイルの保存"
            partial.replace(output)
            logging.info("取得完了・MD5 照合成功: %s (%s バイト)", output, format(source["sizeBytes"], ","))
            return source
        except (*RETRY_ERRORS, ValueError) as exc:
            remaining = max(0, deadline - time.monotonic())
            retry = isinstance(exc, RETRY_ERRORS) and attempt < attempts - 1 and remaining > 0
            log = logging.warning if retry else logging.error
            log("取得失敗: 段階=%s、試行=%s/%s、URL=%s、保存先=%s、%s: %s、残り %.1f 秒",
                stage, attempt + 1, attempts, source["url"] if source else SOURCE_URL, output,
                type(exc).__name__, exc, remaining)
            if not retry:
                reason = "再試行対象外" if not isinstance(exc, RETRY_ERRORS) else (
                    "時間制限" if remaining <= 0 else "試行回数の上限")
                logging.error("全国データの取得を終了します（%s）", reason)
                raise
            delay = min(retry_delay, remaining)
            logging.warning("%.1f 秒待って再試行します (%s/%s)", delay, attempt + 2, attempts)
            time.sleep(delay)
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

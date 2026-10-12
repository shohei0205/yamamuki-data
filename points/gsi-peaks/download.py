"""国土地理院の取得済みファイルを、ハッシュと応答情報付きで保存する。"""

from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import ssl
import subprocess
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import Request, urlopen


class Cache:
    def __init__(self, directory, refresh=False, offline=False):
        self.directory = Path(directory)
        self.directory.mkdir(parents=True, exist_ok=True)
        self.refresh = refresh
        self.offline = offline
        self.used = {}

    def get(self, url, missing_ok=False):
        parsed = urlparse(url)
        if parsed.scheme != "https" or parsed.hostname not in {"www.gsi.go.jp", "cyberjapandata.gsi.go.jp"}:
            raise ValueError("国土地理院の HTTPS URL だけを取得します")
        key = hashlib.sha256(url.encode()).hexdigest()
        body, info = self.directory / (key + ".data"), self.directory / (key + ".json")
        if info.exists() and not self.refresh:
            metadata = json.loads(info.read_text(encoding="utf-8"))
            if metadata["url"] != url:
                raise ValueError("キャッシュの URL が一致しません")
            if metadata["status"] == 404 and missing_ok:
                self.used[url] = metadata
                return None
            payload = body.read_bytes()
            if metadata["status"] != 200 or hashlib.sha256(payload).hexdigest() != metadata["sha256"]:
                raise ValueError("取得済みファイルのハッシュが一致しません")
            self.used[url] = metadata
            return payload
        if self.offline:
            raise FileNotFoundError(f"再生成に必要な元データが保存されていません: {url}")
        metadata = {"url": url, "fetchedAt": datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")}
        for attempt in range(3):
            try:
                with urlopen(Request(url, headers={"User-Agent": "yamamuki-data-gsi-prototype/1"}), timeout=30) as response:
                    if response.status != 200 or urlparse(response.url).hostname not in {"www.gsi.go.jp", "cyberjapandata.gsi.go.jp"}:
                        raise ValueError("取得先または HTTP 応答が不正です")
                    payload = response.read()
                    metadata.update(status=200, lastModified=response.headers.get("Last-Modified"), etag=response.headers.get("ETag"))
                break
            except HTTPError as error:
                if error.code == 404 and missing_ok:
                    metadata.update(status=404, sha256=None, sizeBytes=0)
                    info.write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
                    self.used[url] = metadata
                    return None
                if error.code not in {429, 500, 502, 503, 504} or attempt == 2:
                    raise
                time.sleep(2 ** attempt)
            except URLError as error:
                # 一部の Windows 環境では、公式 CSV の TLS を OpenSSL が扱えない。
                # 証明書検証を無効にせず、Windows 標準の curl で取得する。
                if isinstance(error.reason, ssl.SSLError) and "UNSAFE_LEGACY_RENEGOTIATION" in str(error) and shutil.which("curl.exe"):
                    temporary = self.directory / (key + ".part")
                    subprocess.run(["curl.exe", "--fail", "--silent", "--show-error", "--max-time", "60", "--output", str(temporary), url], check=True)
                    payload = temporary.read_bytes()
                    temporary.unlink()
                    metadata.update(status=200, lastModified=None, etag=None, transport="Windows curl")
                    break
                if attempt == 2:
                    raise
                time.sleep(2 ** attempt)
        metadata.update(sha256=hashlib.sha256(payload).hexdigest(), sizeBytes=len(payload))
        temporary = body.with_suffix(".part")
        temporary.write_bytes(payload)
        temporary.replace(body)
        info.write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
        self.used[url] = metadata
        return payload

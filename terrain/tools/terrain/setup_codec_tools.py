"""比較用の公式配布物を、版とハッシュを固定して build/ に用意する。"""
import base64
import hashlib
import io
import json
from pathlib import Path
import tarfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2] / "build/terrain-codec-tools"
ARCHIVES = {
    "jxl-0.12.0": (
        "https://github.com/libjxl/libjxl/releases/download/v0.12.0/jxl-x64-windows-static.zip",
        "3025d7e308390796d20492322e606bc92decaee7b6bc99d3f7547870ae5db7de",
    ),
    "astc": (
        "https://github.com/ARM-software/astc-encoder/releases/download/5.7.0/astcenc-5.7.0-windows-x64.zip",
        "cbde7c78adcc90d0d90ecf10f32d03f3340fbe1720cb6d6c892147d625e74d2d",
    ),
    "lerc": (
        "https://files.pythonhosted.org/packages/a2/26/8342587ebbb4d5302c2813e6db859aced1b7a99c3de268d59d377ef91ef3/pylerc-4.2.0-py3-none-win_amd64.whl",
        "8998766c4d371e60b42c4cf1e3c38aca7e81a977eed5349505acef65e2681067",
    ),
}
PACKAGES = {
    "meshoptimizer": ("1.3.0", "2oZqMAcEEiCtXWUjIGtZ71oPx1B0ZxYGTqIKsXB1wzWFhleNsCFxQuUJzgCV706bRcwPWtMGf1odmQvAcqKkIw=="),
    "fzstd": ("0.1.1", "dkuVSOKKwh3eas5VkJy1AW1vFpet8TA/fGmVA5krThl8YcOVE/8ZIoEA1+U1vEn5ckxxhLirSdY837azmbaNHA=="),
    "lerc": ("4.2.0", "5TeQPWq2xnHjaxkTrskOG+Ove9pWRGFMKlofBx2hPORFHmIMhyoYWC5zMUJ9mibq0e5tqTZR/dvyAMZ3F6yCqw=="),
}


def download(url, algorithm, expected):
    cache = ROOT / "downloads" / url.rsplit("/", 1)[1]
    cache.parent.mkdir(parents=True, exist_ok=True)
    data = cache.read_bytes() if cache.exists() else urllib.request.urlopen(url, timeout=120).read()
    actual = hashlib.new(algorithm, data).digest()
    if actual != expected:
        raise ValueError(f"配布物のハッシュが一致しません: {url}")
    cache.write_bytes(data)
    return data


def destination(root, name):
    path = (root / name).resolve()
    if not path.is_relative_to(root.resolve()):
        raise ValueError("展開先が作業ディレクトリの外です")
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


def main():
    for name, (url, digest) in ARCHIVES.items():
        data = download(url, "sha256", bytes.fromhex(digest))
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            for item in archive.infolist():
                if not item.is_dir():
                    destination(ROOT / name, item.filename).write_bytes(archive.read(item))
    for name, (version, digest) in PACKAGES.items():
        url = f"https://registry.npmjs.org/{name}/-/{name}-{version}.tgz"
        data = download(url, "sha512", base64.b64decode(digest))
        folder = ROOT / "web" / name
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            for item in archive:
                if item.isfile():
                    relative = Path(item.name).relative_to("package")
                    destination(folder, relative).write_bytes(archive.extractfile(item).read())
        destination(folder, "source.json").write_text(json.dumps({
            "version": version, "url": url, "integrity": "sha512-" + digest,
        }, indent=2), encoding="utf-8")
    print(f"比較用ツールを用意しました: {ROOT}")


if __name__ == "__main__":
    main()

"""配布データの作成者と、Release に添付する利用条件のファイルを扱う。"""

from pathlib import Path
import shutil

LICENSE_FILE = "LICENSE.txt"
ODBL_FILE = "ODbL-1.0.txt"
ODBL_TEXT = Path(__file__).resolve().parents[1] / "LICENSES" / ODBL_FILE
ODBL_URL = "https://opendatacommons.org/licenses/odbl/1-0/"
AUTHORS_URL = "https://github.com/shohei0205/yamamuki-data/blob/main/AUTHORS.md"
AUTHORS = ({"name": "The yamamuki Authors", "role": "配布データの編集・加工", "url": AUTHORS_URL},)


def authors():
    """metadata.authors に入れる作成者の一覧。元データの作者は sources に残す。"""
    return [dict(author) for author in AUTHORS]


def license_url(download_url):
    """データ本体と同じ Release に添付する利用条件のファイルの URL。"""
    return download_url.rsplit("/", 1)[0] + "/" + LICENSE_FILE


def license_text(manifest, *, odbl=False):
    from document_metadata import metadata_maps
    metadata = metadata_maps(manifest["metadata"])
    lines = ["yamamuki-data 配布データの利用条件", "",
             f"データ: {manifest['name']}（{manifest['fileName']}）", f"版: {manifest['version']}", "",
             "## データ全体のライセンス", ""]
    if odbl:
        lines += ["このデータは OpenStreetMap のデータから作った派生データベースで、",
                  "Open Data Commons Open Database License (ODbL) 1.0 で提供する。",
                  f"本文は同じ Release に添付した {ODBL_FILE}（{ODBL_URL}）を参照する。",
                  "再配布するときは、このファイルと下の帰属表示を残す。"]
    else:
        lines += ["データ全体に一つのライセンスは定めていない。下の出典ごとの利用条件に従う。"]
    lines += ["", "## 出典と元データの帰属表示", ""]
    for name, source in metadata.get("sources", {}).items():
        lines.append(f"- {name}")
        for key, label in (("source", "資料"), ("license", "利用条件"), ("attribution", "帰属表示"), ("changes", "加工")):
            if source.get(key):
                lines.append(f"  {label}: {source[key]}")
    lines += ["", "## 編集・加工した者", ""]
    for author in metadata.get("authors", []):
        lines.append(f"- {author['name']}（{author.get('role', '')}）" + (f" {author['url']}" if author.get("url") else ""))
    lines += ["", "上の出典の資料を選び、照合・補正して、このデータを作った。元データの作者ではない。",
              "", "## 表示のお願い（任意）", "",
              "元データのライセンスが求める表示に加えて、できれば次のように作成者も表示してください。",
              "この表示は義務ではありません。", "",
              "  山データ：yamamuki-data（The yamamuki Authors）"]
    return "\n".join(lines) + "\n"


def write_license_files(directory, manifest, *, odbl=False):
    """利用条件のファイルを書き、添付するファイル名を返す。"""
    directory = Path(directory)
    (directory / LICENSE_FILE).write_text(license_text(manifest, odbl=odbl), encoding="utf-8", newline="\n")
    if not odbl:
        return (LICENSE_FILE,)
    shutil.copyfile(ODBL_TEXT, directory / ODBL_FILE)
    return (LICENSE_FILE, ODBL_FILE)

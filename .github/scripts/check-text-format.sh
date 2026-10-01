#!/usr/bin/env bash
# 改行コードと BOM が .gitattributes / .editorconfig の指定どおりかを確かめる。
# - リポジトリに入っているテキストファイルは改行が LF(.bat もリポジトリ内は LF で、チェックアウト時に CRLF になる)
# - Markdown は BOM 付き。ただしスキルの SKILL.md は BOM なし
# - それ以外のテキストファイルは BOM なし
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

failed=0

# git ls-files --eol の i/ はリポジトリ(インデックス)に入っている形。crlf や mixed なら LF にそろっていない。
while IFS=$'\t' read -r info path; do
  case "$info" in
    i/crlf*|i/mixed*)
      echo "::error file=$path::改行に CRLF が入っています。LF にしてください。"
      failed=1 ;;
  esac
done < <(git ls-files --eol)

bom=$'\xef\xbb\xbf'
while IFS= read -r path; do
  # バイナリ(i/-text)は対象外
  [[ "$(git ls-files --eol -- "$path")" == i/-text* ]] && continue
  # 大きなファイルは head が先に閉じて git が SIGPIPE で終わるので、その失敗は無視する。
  head3=$(git cat-file blob ":$path" | head -c 3 || true)
  case "$path" in
    SKILL.md|*/SKILL.md)
      [[ "$head3" == "$bom" ]] && { echo "::error file=$path::SKILL.md には BOM を付けないでください。"; failed=1; } ;;
    *.md)
      [[ "$head3" != "$bom" ]] && { echo "::error file=$path::Markdown は UTF-8 BOM 付きで保存してください。"; failed=1; } ;;
    *)
      [[ "$head3" == "$bom" ]] && { echo "::error file=$path::Markdown 以外には BOM を付けないでください。"; failed=1; } ;;
  esac
done < <(git ls-files)

if [[ $failed -ne 0 ]]; then
  echo "改行コードか BOM が AGENTS.md のルールと合わないファイルがあります。"
  exit 1
fi
echo "改行コードと BOM はすべてルールどおりです。"

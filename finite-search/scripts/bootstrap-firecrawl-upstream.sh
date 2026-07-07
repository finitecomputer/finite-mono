#!/usr/bin/env bash
set -euo pipefail

target="${1:-vendor/firecrawl}"
ref="${FIRECRAWL_REF:-main}"

if [ -d "$target/.git" ]; then
  git -C "$target" fetch --depth 1 origin "$ref"
  git -C "$target" checkout FETCH_HEAD
else
  mkdir -p "$(dirname "$target")"
  git clone --depth 1 --branch "$ref" https://github.com/firecrawl/firecrawl.git "$target"
fi

printf "firecrawl upstream ready at %s\n" "$target"


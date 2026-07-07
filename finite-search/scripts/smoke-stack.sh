#!/usr/bin/env bash
set -euo pipefail

"$(dirname "${BASH_SOURCE[0]}")/smoke-searxng.sh"
"$(dirname "${BASH_SOURCE[0]}")/smoke-firecrawl.sh"

echo "finite-search stack smoke ok"


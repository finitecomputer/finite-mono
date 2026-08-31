#!/usr/bin/env bash
# linecount reporter: count the repo's tracked lines, push one data point to
# the linecount fragment's inbox. Runs wherever the repo is checked out —
# a GitHub Action (see workflow.yml next to this file) or a laptop.
#
# Pure line count by design: comments, blanks and tests all count; lockfiles
# and other binaries don't — a dependency bump would swing the "score" by
# tens of thousands of machine-generated lines, and tracked images/fonts
# by however many newlines live in their bytes. Edit EXCLUDE if your repo
# has other generated files worth ignoring — but freeze the rule before the
# first data point: changing the ruler mid-drive poisons the series.
#
# Env:
#   FRAGMENT_INBOX_URL  required — the fragment's webhook URL
#                       (https://<host>/api/f/linecount/inbox?t=<token>)
#                       Unset = no-op success, so the workflow can ship
#                       before the secret exists. A failed push is also a
#                       soft failure: the reporter runs on merged PRs and
#                       must never redden one over a flaky webhook.
#   MERGE_SHA           commit being counted (default: HEAD)
#   PR_NUMBER/TITLE/AUTHOR/URL  PR metadata (absent for a manual seed run)
set -euo pipefail

if [ -z "${FRAGMENT_INBOX_URL:-}" ]; then
  echo "linecount: FRAGMENT_INBOX_URL not set — skipping"
  exit 0
fi

EXCLUDE='(^|/)(package-lock\.json|npm-shrinkwrap\.json|yarn\.lock|pnpm-lock\.yaml|bun\.lockb|Cargo\.lock|poetry\.lock|Pipfile\.lock|Gemfile\.lock|composer\.lock|go\.sum|flake\.lock)$'

MERGE_SHA="${MERGE_SHA:-$(git rev-parse HEAD)}"

# Text files only: grep -I skips binaries, and the empty pattern matches
# every line, so -l prints each text file's name and stops reading it. The
# (|| :) is load-bearing — grep exits 1 when a batch contains no text files
# at all, and an images-only PR must not kill the reporter. Then cat|wc
# (not wc's per-file lines) so xargs batching can't double-count: every
# batch's bytes stream through the one wc. The tr round-trip filters
# NUL-separated names portably (BSD grep -z doesn't split on NUL — it ate
# the whole list as one line); the trade is filenames with embedded
# newlines, which we declare out of scope. An empty list is safe: cat
# inherits the EOF pipe and exits, wc counts 0.
total=$(git ls-files -z | tr '\0' '\n' | grep -vE "$EXCLUDE" | tr '\n' '\0' | (xargs -0 grep -Il '' || :) | tr '\n' '\0' | xargs -0 cat | wc -l | tr -d '[:space:]')

payload=$(jq -n \
  --arg sha "$MERGE_SHA" \
  --argjson pr "${PR_NUMBER:-null}" \
  --arg title "${PR_TITLE:-baseline}" \
  --arg author "${PR_AUTHOR:-manual}" \
  --arg url "${PR_URL:-}" \
  --argjson total "$total" \
  '{source: "linecount-reporter", payload: {sha: $sha, pr: $pr, title: $title, author: $author, url: $url, total: $total}}')

echo "linecount: $total tracked lines at $MERGE_SHA"
# A lost data point is recoverable — re-run the job once the fragment is
# back. The Idempotency-Key makes the retry safe even if the first POST
# landed but the response was lost.
if ! curl -sf --max-time 20 -X POST "$FRAGMENT_INBOX_URL" \
  -H 'content-type: application/json' \
  -H "Idempotency-Key: $MERGE_SHA" \
  -d "$payload"; then
  echo "linecount: push failed — $MERGE_SHA not recorded; re-run this job to retry" >&2
fi
echo

#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: scripts/deploy-staging.sh [TAG]

Deploy or relaunch the SearXNG Tinfoil staging container.

Environment:
  TINFOIL_API_KEY          Optional. Used by tinfoil CLI if not already logged in.
  TINFOIL_BIN              Optional. Defaults to tinfoil or ~/.local/bin/tinfoil.
  TINFOIL_CONTAINER_NAME   Optional. Default: finite-searxng.
  TINFOIL_CONFIG_REPO      Optional. Default: finitecomputer/finite-searxng-tinfoil.
  TINFOIL_CONFIG_TAG       Optional. Default: first positional arg or v0.0.5.
  TINFOIL_STAGING          Optional. true/false. Default: true.
  SEARXNG_SECRET_NAME      Optional. Default: SEARXNG_SECRET.
  SEARXNG_SECRET_VALUE     Optional. If set, creates or updates the Tinfoil secret.
  FINITE_SEARCH_TOKEN_NAME Optional. Default: FINITE_SEARCH_TOKEN. Set empty to skip.
  FINITE_SEARCH_TOKEN_VALUE Optional. If set, creates or updates the auth token secret.
  TINFOIL_PROXY_PORT       Optional. Default: 3301, used only in printed next step.

If a secret value is unset, the script requires the corresponding Tinfoil secret
to already exist.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

find_tinfoil() {
  if [[ -n "${TINFOIL_BIN:-}" ]]; then
    printf '%s\n' "$TINFOIL_BIN"
    return
  fi
  if command -v tinfoil >/dev/null 2>&1; then
    command -v tinfoil
    return
  fi
  if [[ -x "$HOME/.local/bin/tinfoil" ]]; then
    printf '%s\n' "$HOME/.local/bin/tinfoil"
    return
  fi
  echo "missing tinfoil CLI; install it or set TINFOIL_BIN" >&2
  exit 1
}

tinfoil_bin="$(find_tinfoil)"
repo="${TINFOIL_CONFIG_REPO:-finitecomputer/finite-searxng-tinfoil}"
tag="${TINFOIL_CONFIG_TAG:-${1:-v0.0.5}}"
container="${TINFOIL_CONTAINER_NAME:-finite-searxng}"
secret_name="${SEARXNG_SECRET_NAME:-SEARXNG_SECRET}"
auth_secret_name="${FINITE_SEARCH_TOKEN_NAME-FINITE_SEARCH_TOKEN}"
staging="${TINFOIL_STAGING:-true}"
proxy_port="${TINFOIL_PROXY_PORT:-3301}"

case "$staging" in
  true | false) ;;
  *)
    echo "TINFOIL_STAGING must be true or false" >&2
    exit 1
    ;;
esac

if ! "$tinfoil_bin" whoami >/dev/null 2>&1; then
  cat >&2 <<EOF
Tinfoil CLI is not logged in.

Run one of:
  $tinfoil_bin login --api-key admin_...
  export TINFOIL_API_KEY=admin_...
EOF
  exit 1
fi

ensure_secret() {
  local name="$1"
  local value_var="$2"
  local value="${!value_var:-}"

  if "$tinfoil_bin" secret get "$name" >/dev/null 2>&1; then
    if [[ -n "$value" ]]; then
      printf '%s' "$value" |
        "$tinfoil_bin" secret set "$name" --value-file -
    fi
    return
  fi

  if [[ -z "$value" ]]; then
    cat >&2 <<EOF
Tinfoil secret '$name' does not exist.

Create it by exporting $value_var, for example:
  export $value_var="\$(openssl rand -hex 32)"
  scripts/deploy-staging.sh "$tag"
  unset $value_var
EOF
    exit 1
  fi

  printf '%s' "$value" |
    "$tinfoil_bin" secret create "$name" --value-file -
}

ensure_secret "$secret_name" SEARXNG_SECRET_VALUE
secret_args=(--secret "$secret_name")

if [[ -n "$auth_secret_name" ]]; then
  ensure_secret "$auth_secret_name" FINITE_SEARCH_TOKEN_VALUE
  secret_args+=(--secret "$auth_secret_name")
fi

if "$tinfoil_bin" container get "$container" >/dev/null 2>&1; then
  "$tinfoil_bin" container relaunch "$container" \
    --tag "$tag" \
    "${secret_args[@]}" \
    --staging "$staging"
else
  create_args=(
    container create "$container"
    --repo "$repo"
    --tag "$tag"
    "${secret_args[@]}"
  )
  if [[ "$staging" == "true" ]]; then
    create_args+=(--staging)
  fi
  "$tinfoil_bin" "${create_args[@]}"
fi

cat <<EOF

Deploy submitted for $container ($repo@$tag).

After it is ready, verify through the attested local proxy:
  $tinfoil_bin container connect "$container" -p "$proxy_port"
  curl -fsS -H 'Authorization: Bearer <FINITE_SEARCH_TOKEN>' 'http://127.0.0.1:$proxy_port/search?q=open+source&format=json' | jq '.results | length'
EOF

#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

cd "$REPO_ROOT"

export CARGO_NET_RETRY="${CARGO_NET_RETRY:-10}"

retry() {
  local attempts="$1"
  shift

  local attempt=1
  while true; do
    if "$@"; then
      return 0
    fi

    if [ "$attempt" -ge "$attempts" ]; then
      return 1
    fi

    echo "Command failed on attempt ${attempt}/${attempts}; retrying: $*" >&2
    sleep $((attempt * 10))
    attempt=$((attempt + 1))
  done
}

if ! command -v cargo >/dev/null 2>&1; then
  retry 3 curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
  sh /tmp/rustup-init.sh -y --profile minimal
  # shellcheck disable=SC1090
  source "$HOME/.cargo/env"
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "error: rustup is required to install iOS Rust targets" >&2
  exit 1
fi

if ! command -v protoc >/dev/null 2>&1; then
  if command -v brew >/dev/null 2>&1; then
    brew install protobuf
  else
    echo "error: protoc is required but Homebrew is unavailable" >&2
    exit 1
  fi
fi

if ! command -v xcodegen >/dev/null 2>&1; then
  if command -v brew >/dev/null 2>&1; then
    brew install xcodegen
  else
    echo "error: xcodegen is required but Homebrew is unavailable" >&2
    exit 1
  fi
fi

export PROTOC="$(command -v protoc)"

rustup target add aarch64-apple-ios aarch64-apple-ios-sim

cargo run -q -p finitechat-rmp -- bindings swift --clean

(cd ios && xcodegen generate)

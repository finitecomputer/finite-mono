#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"
FINITECHAT_ROOT="${REPO_ROOT}/finitechat"
IOS_ROOT="${FINITECHAT_ROOT}/ios"
DERIVED_DATA_PATH="${REPO_ROOT}/.state/ios-xcode-cloud-preflight"
EXPECTED_RUST_VERSION="1.91.1"

require_command() {
  local command_name="$1"
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: ${command_name} is required; run this through the repo dev environment" >&2
    exit 1
  fi
}

require_setting() {
  local setting_name="$1"
  local expected_value="$2"
  if ! grep -Eq "^[[:space:]]*${setting_name} = ${expected_value}$" <<<"$BUILD_SETTINGS"; then
    echo "error: expected Release ${setting_name}=${expected_value}" >&2
    exit 1
  fi
}

run_xcodebuild() {
  # The Nix development shell exports generic compiler/binutils variables for
  # Rust and C builds. Xcode interprets those as build-setting overrides and
  # may invoke the raw linker instead of Apple's compiler driver.
  env \
    -u AR \
    -u AS \
    -u CC \
    -u CXX \
    -u LD \
    -u NM \
    -u OBJCOPY \
    -u OBJDUMP \
    -u RANLIB \
    -u SIZE \
    -u STRINGS \
    -u STRIP \
    /usr/bin/xcrun xcodebuild "$@"
}

for command_name in cargo rustc protoc xcodegen xcodebuild; do
  require_command "$command_name"
done

if [[ ! -x "${IOS_ROOT}/ci_scripts/ci_post_clone.sh" ]]; then
  echo "error: Xcode Cloud post-clone script is missing or not executable" >&2
  exit 1
fi

actual_rust_version="$(rustc --version | awk '{print $2}')"
if [[ "$actual_rust_version" != "$EXPECTED_RUST_VERSION" ]]; then
  echo "error: expected rustc ${EXPECTED_RUST_VERSION}, found ${actual_rust_version}" >&2
  exit 1
fi

export PROTOC="$(command -v protoc)"

cd "$FINITECHAT_ROOT"
cargo run --locked -q -p finitechat-rmp -- bindings swift --clean
(cd ios && xcodegen generate)

BUILD_SETTINGS="$(
  run_xcodebuild \
    -project ios/FiniteChat.xcodeproj \
    -scheme FiniteChat \
    -configuration Release \
    -sdk iphonesimulator \
    -destination "generic/platform=iOS Simulator" \
    -showBuildSettings
)"

require_setting PRODUCT_BUNDLE_IDENTIFIER computer.finite.finitechat
require_setting MARKETING_VERSION 1.0
require_setting CURRENT_PROJECT_VERSION 1
require_setting WORKOS_CLIENT_ID client_01KYA32JRWEE23J7QW1F882DVA

run_xcodebuild \
  -quiet \
  -project ios/FiniteChat.xcodeproj \
  -scheme FiniteChat \
  -configuration Release \
  -sdk iphonesimulator \
  -destination "generic/platform=iOS Simulator" \
  -derivedDataPath "$DERIVED_DATA_PATH" \
  CODE_SIGNING_ALLOWED=NO \
  clean build

echo "iOS Xcode Cloud preflight passed for Finite Chat 1.0."

#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: scripts/package-release.sh [--repo-root <dir>] [--binary <path>] [--output-dir <dir>] [--output-file <path>]
EOF
}

REPO_ROOT=""
BINARY_PATH=""
DIST_DIR=""
DIST_FILE=""
DIST_FILE_NAME="${GROVE_DIST_FILE_NAME:-grove-aarch64-apple-darwin.tar.gz}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)
      [[ $# -ge 2 ]] || fail "--repo-root requires a value"
      REPO_ROOT="$2"
      shift 2
      ;;
    --binary)
      [[ $# -ge 2 ]] || fail "--binary requires a value"
      BINARY_PATH="$2"
      shift 2
      ;;
    --output-dir)
      [[ $# -ge 2 ]] || fail "--output-dir requires a value"
      DIST_DIR="$2"
      shift 2
      ;;
    --output-file)
      [[ $# -ge 2 ]] || fail "--output-file requires a value"
      DIST_FILE="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

REPO_ROOT="${REPO_ROOT:-${GROVE_REPO_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}}"
BINARY_PATH="${BINARY_PATH:-${GROVE_BINARY:-$REPO_ROOT/target/release/grove}}"
DIST_DIR="${DIST_DIR:-${GROVE_DIST_DIR:-$REPO_ROOT/dist}}"
DIST_FILE="${DIST_FILE:-${GROVE_DIST_FILE:-$DIST_DIR/$DIST_FILE_NAME}}"

BRIDGE_SOURCE="$REPO_ROOT/bridge/grove_bridge.py"
MERMAID_JSON="$REPO_ROOT/tools/mermaid/package.json"
MERMAID_LOCK="$REPO_ROOT/tools/mermaid/package-lock.json"
MERMAID_RENDER="$REPO_ROOT/tools/mermaid/render_ascii.mjs"

[[ -f "$BINARY_PATH" ]] || fail "binary missing: $BINARY_PATH"
[[ -x "$BINARY_PATH" ]] || fail "binary is not executable: $BINARY_PATH"
[[ -f "$BRIDGE_SOURCE" ]] || fail "bridge script missing: $BRIDGE_SOURCE"
[[ -f "$MERMAID_JSON" ]] || fail "mermaid package.json missing: $MERMAID_JSON"
[[ -f "$MERMAID_LOCK" ]] || fail "mermaid package-lock.json missing: $MERMAID_LOCK"
[[ -f "$MERMAID_RENDER" ]] || fail "mermaid render script missing: $MERMAID_RENDER"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

STAGING_ROOT="$TMP_DIR/grove-aarch64-apple-darwin"
mkdir -p "$STAGING_ROOT/bin" "$STAGING_ROOT/share/grove/bridge" "$STAGING_ROOT/share/grove/mermaid" "$DIST_DIR"

cp "$BINARY_PATH" "$STAGING_ROOT/bin/grove"
cp "$BRIDGE_SOURCE" "$STAGING_ROOT/share/grove/bridge/grove_bridge.py"
cp "$MERMAID_JSON" "$STAGING_ROOT/share/grove/mermaid/package.json"
cp "$MERMAID_LOCK" "$STAGING_ROOT/share/grove/mermaid/package-lock.json"
cp "$MERMAID_RENDER" "$STAGING_ROOT/share/grove/mermaid/render_ascii.mjs"

chmod +x "$STAGING_ROOT/bin/grove"

tar -C "$TMP_DIR" -czf "$DIST_FILE" "$(basename "$STAGING_ROOT")"

echo "packaged: $DIST_FILE"

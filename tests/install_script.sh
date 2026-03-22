#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

ASSET_NAME="grove-aarch64-apple-darwin.tar.gz"
PAYLOAD_ROOT="$TMP_DIR/payload/grove-aarch64-apple-darwin"
RELEASES_ROOT="$TMP_DIR/releases"
PREFIX_LATEST="$TMP_DIR/prefix-latest"
PREFIX_VERSIONED="$TMP_DIR/prefix-versioned"
AUTOLAUNCH_LATEST="$TMP_DIR/autolaunch-latest"
AUTOLAUNCH_VERSIONED="$TMP_DIR/autolaunch-versioned"
EXPECTED_VERSION="v9.9.9"

mkdir -p \
  "$PAYLOAD_ROOT/bin" \
  "$PAYLOAD_ROOT/share/grove/bridge" \
  "$PAYLOAD_ROOT/share/grove/mermaid" \
  "$RELEASES_ROOT/latest/download" \
  "$RELEASES_ROOT/download/$EXPECTED_VERSION"

cat >"$PAYLOAD_ROOT/bin/grove" <<'EOF'
#!/bin/sh
echo "grove"
EOF
chmod +x "$PAYLOAD_ROOT/bin/grove"
cp "$ROOT_DIR/bridge/grove_bridge.py" "$PAYLOAD_ROOT/share/grove/bridge/grove_bridge.py"
cp "$ROOT_DIR/tools/mermaid/package.json" "$PAYLOAD_ROOT/share/grove/mermaid/package.json"
cp "$ROOT_DIR/tools/mermaid/package-lock.json" "$PAYLOAD_ROOT/share/grove/mermaid/package-lock.json"
cp "$ROOT_DIR/tools/mermaid/render_ascii.mjs" "$PAYLOAD_ROOT/share/grove/mermaid/render_ascii.mjs"

tar -C "$TMP_DIR/payload" -czf "$RELEASES_ROOT/latest/download/$ASSET_NAME" grove-aarch64-apple-darwin
cp "$RELEASES_ROOT/latest/download/$ASSET_NAME" "$RELEASES_ROOT/download/$EXPECTED_VERSION/$ASSET_NAME"

GROVE_INSTALL_OS=Darwin \
GROVE_INSTALL_ARCH=arm64 \
GROVE_INSTALL_RELEASE_BASE_URL="file://$RELEASES_ROOT" \
GROVE_INSTALL_PREFIX="$PREFIX_LATEST" \
GROVE_INSTALL_AUTOLAUNCH_DIR="$AUTOLAUNCH_LATEST" \
"$ROOT_DIR/scripts/install.sh" --yes

test -x "$PREFIX_LATEST/bin/grove"
test -f "$PREFIX_LATEST/share/grove/bridge/grove_bridge.py"
test -f "$PREFIX_LATEST/share/grove/mermaid/package.json"
test -L "$AUTOLAUNCH_LATEST/grove_bridge.py"
test ! -d "$PREFIX_LATEST/share/grove/mermaid/node_modules"

printf 'n\nn\n' | \
GROVE_INSTALL_OS=Darwin \
GROVE_INSTALL_ARCH=arm64 \
GROVE_INSTALL_RELEASE_BASE_URL="file://$RELEASES_ROOT" \
GROVE_INSTALL_PREFIX="$PREFIX_VERSIONED" \
GROVE_INSTALL_AUTOLAUNCH_DIR="$AUTOLAUNCH_VERSIONED" \
"$ROOT_DIR/scripts/install.sh" --version "$EXPECTED_VERSION"

test -x "$PREFIX_VERSIONED/bin/grove"
test ! -e "$AUTOLAUNCH_VERSIONED/grove_bridge.py"
test ! -d "$PREFIX_VERSIONED/share/grove/mermaid/node_modules"

if GROVE_INSTALL_OS=Linux GROVE_INSTALL_ARCH=arm64 "$ROOT_DIR/scripts/install.sh" --yes >"$TMP_DIR/unsupported.log" 2>&1; then
  echo "expected unsupported-platform install to fail" >&2
  exit 1
fi

grep -q "macOS arm64" "$TMP_DIR/unsupported.log"

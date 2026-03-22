#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

FAKE_BINARY="$TMP_DIR/grove"
OUTPUT_DIR="$TMP_DIR/dist"
EXPECTED="$TMP_DIR/expected.txt"
ACTUAL="$TMP_DIR/actual.txt"

cat >"$FAKE_BINARY" <<'EOF'
#!/bin/sh
echo "grove"
EOF
chmod +x "$FAKE_BINARY"

"$ROOT_DIR/scripts/package-release.sh" \
  --repo-root "$ROOT_DIR" \
  --binary "$FAKE_BINARY" \
  --output-dir "$OUTPUT_DIR"

ASSET_PATH="$OUTPUT_DIR/grove-aarch64-apple-darwin.tar.gz"
test -f "$ASSET_PATH"

cat >"$EXPECTED" <<'EOF'
grove-aarch64-apple-darwin/
grove-aarch64-apple-darwin/bin/
grove-aarch64-apple-darwin/bin/grove
grove-aarch64-apple-darwin/share/
grove-aarch64-apple-darwin/share/grove/
grove-aarch64-apple-darwin/share/grove/bridge/
grove-aarch64-apple-darwin/share/grove/bridge/grove_bridge.py
grove-aarch64-apple-darwin/share/grove/mermaid/
grove-aarch64-apple-darwin/share/grove/mermaid/package-lock.json
grove-aarch64-apple-darwin/share/grove/mermaid/package.json
grove-aarch64-apple-darwin/share/grove/mermaid/render_ascii.mjs
EOF

tar -tzf "$ASSET_PATH" | sort >"$ACTUAL"
diff -u "$EXPECTED" "$ACTUAL"

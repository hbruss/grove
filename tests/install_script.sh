#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

ASSET_NAME="grove-aarch64-apple-darwin.tar.gz"
PAYLOAD_ROOT="$TMP_DIR/payload/grove-aarch64-apple-darwin"
RELEASES_ROOT="$TMP_DIR/releases"
HOME_LATEST="$TMP_DIR/home-latest"
HOME_VERSIONED="$TMP_DIR/home-versioned"
HOME_ON_PATH="$TMP_DIR/home-on-path"
HOME_UNSUPPORTED="$TMP_DIR/home-unsupported"
PREFIX_LATEST="$HOME_LATEST/.local"
PREFIX_VERSIONED="$HOME_VERSIONED/.local"
PREFIX_ON_PATH="$HOME_ON_PATH/.local"
PREFIX_UNSUPPORTED="$HOME_UNSUPPORTED/.local"
AUTOLAUNCH_LATEST="$TMP_DIR/autolaunch-latest"
AUTOLAUNCH_VERSIONED="$TMP_DIR/autolaunch-versioned"
AUTOLAUNCH_ON_PATH="$TMP_DIR/autolaunch-on-path"
AUTOLAUNCH_UNSUPPORTED="$TMP_DIR/autolaunch-unsupported"
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
GROVE_INSTALL_HOME="$HOME_LATEST" \
GROVE_INSTALL_PREFIX="$PREFIX_LATEST" \
GROVE_INSTALL_AUTOLAUNCH_DIR="$AUTOLAUNCH_LATEST" \
PATH="/usr/bin:/bin" \
"$ROOT_DIR/scripts/install.sh" --yes

test -x "$PREFIX_LATEST/bin/grove"
test -f "$PREFIX_LATEST/share/grove/bridge/grove_bridge.py"
test -f "$PREFIX_LATEST/share/grove/mermaid/package.json"
test -L "$AUTOLAUNCH_LATEST/grove_bridge.py"
test ! -d "$PREFIX_LATEST/share/grove/mermaid/node_modules"
test ! -e "$HOME_LATEST/.zprofile"
test ! -e "$HOME_LATEST/.bash_profile"

printf 'n\nn\ny\n' | \
GROVE_INSTALL_OS=Darwin \
GROVE_INSTALL_ARCH=arm64 \
GROVE_INSTALL_RELEASE_BASE_URL="file://$RELEASES_ROOT" \
GROVE_INSTALL_HOME="$HOME_VERSIONED" \
GROVE_INSTALL_PREFIX="$PREFIX_VERSIONED" \
GROVE_INSTALL_AUTOLAUNCH_DIR="$AUTOLAUNCH_VERSIONED" \
SHELL="/bin/zsh" \
PATH="/usr/bin:/bin" \
"$ROOT_DIR/scripts/install.sh" --version "$EXPECTED_VERSION" >"$TMP_DIR/versioned.log"

test -x "$PREFIX_VERSIONED/bin/grove"
test ! -e "$AUTOLAUNCH_VERSIONED/grove_bridge.py"
test ! -d "$PREFIX_VERSIONED/share/grove/mermaid/node_modules"
test -f "$HOME_VERSIONED/.zprofile"
grep -q '# Added by Grove installer' "$HOME_VERSIONED/.zprofile"
grep -q 'export PATH="\$HOME/.local/bin:\$PATH"' "$HOME_VERSIONED/.zprofile"
grep -q 'Added PATH setup to ' "$TMP_DIR/versioned.log"

printf 'n\nn\n' | \
GROVE_INSTALL_OS=Darwin \
GROVE_INSTALL_ARCH=arm64 \
GROVE_INSTALL_RELEASE_BASE_URL="file://$RELEASES_ROOT" \
GROVE_INSTALL_HOME="$HOME_ON_PATH" \
GROVE_INSTALL_PREFIX="$PREFIX_ON_PATH" \
GROVE_INSTALL_AUTOLAUNCH_DIR="$AUTOLAUNCH_ON_PATH" \
SHELL="/bin/zsh" \
PATH="$PREFIX_ON_PATH/bin:/usr/bin:/bin" \
"$ROOT_DIR/scripts/install.sh" --version "$EXPECTED_VERSION" >"$TMP_DIR/on-path.log"

test -x "$PREFIX_ON_PATH/bin/grove"
test ! -e "$AUTOLAUNCH_ON_PATH/grove_bridge.py"
test ! -e "$HOME_ON_PATH/.zprofile"
if grep -q 'Add ~/.local/bin to your PATH' "$TMP_DIR/on-path.log"; then
  echo "did not expect PATH prompt when install bin is already on PATH" >&2
  exit 1
fi

printf 'n\nn\n' | \
GROVE_INSTALL_OS=Darwin \
GROVE_INSTALL_ARCH=arm64 \
GROVE_INSTALL_RELEASE_BASE_URL="file://$RELEASES_ROOT" \
GROVE_INSTALL_HOME="$HOME_UNSUPPORTED" \
GROVE_INSTALL_PREFIX="$PREFIX_UNSUPPORTED" \
GROVE_INSTALL_AUTOLAUNCH_DIR="$AUTOLAUNCH_UNSUPPORTED" \
SHELL="/bin/fish" \
PATH="/usr/bin:/bin" \
"$ROOT_DIR/scripts/install.sh" --version "$EXPECTED_VERSION" >"$TMP_DIR/unsupported-shell.log"

test -x "$PREFIX_UNSUPPORTED/bin/grove"
test ! -e "$HOME_UNSUPPORTED/.zprofile"
test ! -e "$HOME_UNSUPPORTED/.bash_profile"
grep -q 'Could not determine a supported login profile for /bin/fish.' "$TMP_DIR/unsupported-shell.log"
grep -q 'export PATH="\$HOME/.local/bin:\$PATH"' "$TMP_DIR/unsupported-shell.log"

if GROVE_INSTALL_OS=Linux GROVE_INSTALL_ARCH=arm64 "$ROOT_DIR/scripts/install.sh" --yes >"$TMP_DIR/unsupported.log" 2>&1; then
  echo "expected unsupported-platform install to fail" >&2
  exit 1
fi

grep -q "macOS arm64" "$TMP_DIR/unsupported.log"

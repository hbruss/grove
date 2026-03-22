#!/bin/sh
set -eu

REPO_SLUG="hbruss/grove"
ASSET_NAME="grove-aarch64-apple-darwin.tar.gz"

usage() {
  cat <<'EOF'
Usage: install.sh [--yes] [--version <tag>]

Installs Grove into ~/.local by default.
EOF
}

fail() {
  printf '%s\n' "ERROR: $*" >&2
  exit 1
}

YES=0
VERSION=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --yes)
      YES=1
      shift
      ;;
    --version)
      [ "$#" -ge 2 ] || fail "--version requires a value"
      VERSION="$2"
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

OS="${GROVE_INSTALL_OS:-$(uname -s)}"
ARCH="${GROVE_INSTALL_ARCH:-$(uname -m)}"

[ "$OS" = "Darwin" ] || fail "Grove installer currently supports macOS arm64 only"
[ "$ARCH" = "arm64" ] || fail "Grove installer currently supports macOS arm64 only"

HOME_DIR="${GROVE_INSTALL_HOME:-$HOME}"
PREFIX="${GROVE_INSTALL_PREFIX:-$HOME_DIR/.local}"
BIN_DIR="$PREFIX/bin"
SHARE_DIR="$PREFIX/share/grove"
BRIDGE_DIR="$SHARE_DIR/bridge"
MERMAID_DIR="$SHARE_DIR/mermaid"
AUTOLAUNCH_DIR="${GROVE_INSTALL_AUTOLAUNCH_DIR:-$HOME_DIR/Library/Application Support/iTerm2/Scripts/AutoLaunch}"
RELEASE_BASE_URL="${GROVE_INSTALL_RELEASE_BASE_URL:-https://github.com/$REPO_SLUG/releases}"
NODE_BIN="${GROVE_INSTALL_NODE_BIN:-node}"
NPM_BIN="${GROVE_INSTALL_NPM_BIN:-npm}"

if [ -n "$VERSION" ]; then
  ASSET_URL="$RELEASE_BASE_URL/download/$VERSION/$ASSET_NAME"
else
  ASSET_URL="$RELEASE_BASE_URL/latest/download/$ASSET_NAME"
fi

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

download_asset() {
  destination="$1"
  case "$ASSET_URL" in
    file://*)
      curl -fsSL "$ASSET_URL" -o "$destination"
      ;;
    *)
      curl --proto '=https' --tlsv1.2 -fsSL "$ASSET_URL" -o "$destination"
      ;;
  esac
}

prompt_choice() {
  prompt="$1"
  default="$2"
  if [ "$YES" -eq 1 ]; then
    [ "$default" = "yes" ] && return 0 || return 1
  fi

  while true; do
    printf '%s' "$prompt"
    if ! IFS= read -r answer; then
      answer=""
    fi
    case "$answer" in
      "")
        [ "$default" = "yes" ] && return 0 || return 1
        ;;
      y|Y|yes|YES)
        return 0
        ;;
      n|N|no|NO)
        return 1
        ;;
      *)
        printf '%s\n' "Please answer y or n." >&2
        ;;
    esac
  done
}

ensure_dir() {
  mkdir -p "$1"
}

download_asset "$TMP_DIR/$ASSET_NAME"
tar -xzf "$TMP_DIR/$ASSET_NAME" -C "$TMP_DIR"

PACKAGE_ROOT="$TMP_DIR/grove-aarch64-apple-darwin"
[ -x "$PACKAGE_ROOT/bin/grove" ] || fail "release asset is missing bin/grove"
[ -f "$PACKAGE_ROOT/share/grove/bridge/grove_bridge.py" ] || fail "release asset is missing bridge helper"
[ -f "$PACKAGE_ROOT/share/grove/mermaid/package.json" ] || fail "release asset is missing Mermaid helper files"

ensure_dir "$BIN_DIR"
ensure_dir "$BRIDGE_DIR"
ensure_dir "$MERMAID_DIR"

cp "$PACKAGE_ROOT/bin/grove" "$BIN_DIR/grove"
chmod +x "$BIN_DIR/grove"
cp "$PACKAGE_ROOT/share/grove/bridge/grove_bridge.py" "$BRIDGE_DIR/grove_bridge.py"
cp "$PACKAGE_ROOT/share/grove/mermaid/package.json" "$MERMAID_DIR/package.json"
cp "$PACKAGE_ROOT/share/grove/mermaid/package-lock.json" "$MERMAID_DIR/package-lock.json"
cp "$PACKAGE_ROOT/share/grove/mermaid/render_ascii.mjs" "$MERMAID_DIR/render_ascii.mjs"

printf '%s\n' "Installed grove to $BIN_DIR/grove"

if prompt_choice "Wire the iTerm2 AutoLaunch bridge now? [Y/n] " "yes"; then
  ensure_dir "$AUTOLAUNCH_DIR"
  ln -sf "$BRIDGE_DIR/grove_bridge.py" "$AUTOLAUNCH_DIR/grove_bridge.py"
  printf '%s\n' "Linked bridge to $AUTOLAUNCH_DIR/grove_bridge.py"
else
  printf '%s\n' "Skipped iTerm2 AutoLaunch bridge setup."
  printf '%s\n' "Manual setup:"
  printf 'mkdir -p "%s"\n' "$AUTOLAUNCH_DIR"
  printf 'ln -sf "%s" "%s/grove_bridge.py"\n' "$BRIDGE_DIR/grove_bridge.py" "$AUTOLAUNCH_DIR"
fi

if prompt_choice "Install optional Mermaid helper dependencies with npm? [y/N] " "no"; then
  if ! command -v "$NODE_BIN" >/dev/null 2>&1; then
    printf '%s\n' "Skipping Mermaid helper install because '$NODE_BIN' is not available."
  elif ! command -v "$NPM_BIN" >/dev/null 2>&1; then
    printf '%s\n' "Skipping Mermaid helper install because '$NPM_BIN' is not available."
  else
    (
      cd "$MERMAID_DIR"
      "$NPM_BIN" install --no-audit --no-fund
    )
    printf '%s\n' "Installed optional Mermaid helper dependencies in $MERMAID_DIR"
  fi
else
  printf '%s\n' "Skipped optional Mermaid helper installation."
fi

case ":${PATH:-}:" in
  *:"$BIN_DIR":*)
    ;;
  *)
    printf '%s\n' "Add $BIN_DIR to your PATH if it is not already available in new shells."
    ;;
esac

printf '%s\n' "Done."

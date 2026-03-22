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
PATH_BLOCK_MARKER="# Added by Grove installer"
PROMPT_INPUT_MODE="stdin"
PROMPT_INPUT_PATH="${GROVE_INSTALL_PROMPT_INPUT:-}"

if [ -n "$VERSION" ]; then
  ASSET_URL="$RELEASE_BASE_URL/download/$VERSION/$ASSET_NAME"
else
  ASSET_URL="$RELEASE_BASE_URL/latest/download/$ASSET_NAME"
fi

TMP_DIR="$(mktemp -d)"
cleanup() {
  if [ "$PROMPT_INPUT_MODE" = "fd3" ]; then
    exec 3<&-
  fi
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
  if [ "$PROMPT_INPUT_MODE" = "tty" ]; then
      if ! IFS= read -r answer </dev/tty; then
        answer=""
      fi
    elif [ "$PROMPT_INPUT_MODE" = "fd3" ]; then
      if ! IFS= read -r answer <&3; then
        answer=""
      fi
    elif ! IFS= read -r answer; then
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

setup_prompt_input() {
  if [ -n "$PROMPT_INPUT_PATH" ]; then
    exec 3<"$PROMPT_INPUT_PATH"
    PROMPT_INPUT_MODE="fd3"
    return 0
  fi

  if [ -t 0 ]; then
    PROMPT_INPUT_MODE="stdin"
    return 0
  fi

  if (exec 3</dev/tty) 2>/dev/null; then
    PROMPT_INPUT_MODE="tty"
    return 0
  fi

  PROMPT_INPUT_MODE="stdin"
}

path_target() {
  if [ "$BIN_DIR" = "$HOME_DIR/.local/bin" ]; then
    printf '%s\n' '$HOME/.local/bin'
  else
    printf '%s\n' "$BIN_DIR"
  fi
}

path_target_display() {
  if [ "$BIN_DIR" = "$HOME_DIR/.local/bin" ]; then
    printf '%s\n' '~/.local/bin'
  else
    printf '%s\n' "$BIN_DIR"
  fi
}

path_export_line() {
  target="$(path_target)"
  printf 'export PATH="%s:$PATH"\n' "$target"
}

manual_path_instructions() {
  printf '%s\n' "Add this line to your login profile:"
  path_export_line
}

path_on_path() {
  case ":${PATH:-}:" in
    *:"$BIN_DIR":*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

detect_login_profile() {
  shell_path="${SHELL:-}"
  shell_name="$(basename "$shell_path" 2>/dev/null || printf '%s' "$shell_path")"

  case "$shell_name" in
    zsh)
      printf '%s\n' "$HOME_DIR/.zprofile"
      ;;
    bash)
      printf '%s\n' "$HOME_DIR/.bash_profile"
      ;;
    *)
      return 1
      ;;
  esac
}

profile_has_path_block() {
  profile_path="$1"
  [ -f "$profile_path" ] && grep -Fq "$PATH_BLOCK_MARKER" "$profile_path"
}

append_path_block() {
  profile_path="$1"
  profile_dir="$(dirname "$profile_path")"
  target="$(path_target)"
  ensure_dir "$profile_dir"

  if [ -f "$profile_path" ] && [ -s "$profile_path" ]; then
    printf '\n' >>"$profile_path"
  fi

  {
    printf '%s\n' "$PATH_BLOCK_MARKER"
    printf '%s\n' 'case ":$PATH:" in'
    printf '  *:"%s":*) ;;\n' "$target"
    printf '  *) export PATH="%s:$PATH" ;;\n' "$target"
    printf '%s\n' 'esac'
  } >>"$profile_path"
}

ensure_path_setup() {
  if path_on_path; then
    return 0
  fi

  if profile_path="$(detect_login_profile)"; then
    if profile_has_path_block "$profile_path"; then
      printf '%s\n' "PATH setup is already present in $profile_path"
      return 0
    fi

    if prompt_choice "Add $(path_target_display) to your PATH in $profile_path? [y/N] " "no"; then
      append_path_block "$profile_path"
      printf '%s\n' "Added PATH setup to $profile_path"
    else
      printf '%s\n' "Skipped PATH profile update."
      manual_path_instructions
    fi
    return 0
  fi

  shell_path="${SHELL:-unknown shell}"
  printf '%s\n' "Could not determine a supported login profile for $shell_path."
  manual_path_instructions
}

setup_prompt_input

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

ensure_path_setup

printf '%s\n' "Done."

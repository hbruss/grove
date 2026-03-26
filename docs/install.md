# Install

Grove's primary install path is a one-line installer that downloads a GitHub Release tarball and installs into `~/.local`.

Homebrew is intentionally not the first-release path.

## Before You Start

- Platform: Apple Silicon macOS for the release installer
- Build toolchain: Rust and Cargo only for the source-build fallback
- Intended terminal: iTerm2 (recommended for full bridge and inline-graphics behavior)
- Canonical bridge path: iTerm2 AutoLaunch running Grove's installed `grove_bridge.py`
- Separate shell integration is not required

Outside iTerm2, Grove still runs as a local TUI, but bridge targeting and inline graphics fall back cleanly instead of trying to emulate iTerm2-only behavior.

## Install From A GitHub Release

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://raw.githubusercontent.com/hbruss/grove/main/scripts/install.sh | sh
```

Install a specific tagged release:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://raw.githubusercontent.com/hbruss/grove/main/scripts/install.sh | sh -s -- --version v0.1.7
```

Run the installer non-interactively with the default answers (`Yes` for bridge wiring, `No` for Mermaid helper setup, and `No` for PATH profile setup):

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://raw.githubusercontent.com/hbruss/grove/main/scripts/install.sh | sh -s -- --yes
```

Installer behavior:

- installs `grove` to `~/.local/bin/grove`
- installs companion assets under `~/.local/share/grove/`
- prompts before wiring the iTerm2 AutoLaunch bridge, defaulting to `Yes`
- prompts before optional Mermaid helper setup, defaulting to `No`
- if `~/.local/bin` is missing from `PATH`, offers to add it to a supported shell profile, defaulting to `No`
- if `~/.local/bin` is already on `PATH`, leaves shell profiles alone
- if PATH profile setup is skipped or the shell is unsupported, prints manual PATH steps instead of guessing

The installer uses the latest published GitHub Release asset. If you are working directly from a checkout and want a local dev path instead, use the source-build fallback below.

## Source-Build Fallback

If you do not already have Rust and Cargo, install them first from:
https://www.rust-lang.org/tools/install

```sh
git clone https://github.com/hbruss/grove.git
cd grove
cargo build --release
./target/release/grove
```

## Enable Bridge Targeting In iTerm2

If you answered `Yes` to the installer's bridge prompt, this is already done for you.

From the repo root, create the AutoLaunch directory if needed, then copy or symlink the bridge script into it:

```sh
mkdir -p "$HOME/Library/Application Support/iTerm2/Scripts/AutoLaunch"
ln -sf "$PWD/bridge/grove_bridge.py" \
  "$HOME/Library/Application Support/iTerm2/Scripts/AutoLaunch/grove_bridge.py"
```

Restart iTerm2 after wiring the bridge.

Expected result:

- Grove still starts normally even if the bridge is unavailable.
- When the bridge is live, the status bar shows `bridge: online`.
- `Ctrl+A`, `Ctrl+E`, and `Ctrl+Y` can then target other iTerm2 panes through the bridge.

`scripts/run_bridge_dev.sh` is still available as a local development helper, but it remains a dev path rather than the canonical install model and still needs an environment where the `iterm2` Python module is available.

## Opt In Bridge Debug Logging

The AutoLaunch bridge supports a permanent opt-in debug log driven by a config file:

- config path: `~/.config/grove/bridge-debug.json`
- log format: JSON Lines
- default behavior: disabled when the config file is absent

Minimal config:

```json
{
  "path": "/Users/you/.config/grove/bridge.log",
  "log_session_lists": true
}
```

Notes:

- `path` is required and points to the log file the bridge appends to.
- `log_session_lists` is optional and defaults to `true`.
- The bridge logs command receipt, sender resolution, picker include/exclude decisions, target resolution, role assignment, and send-text resolution without logging full text payloads.
- Restart iTerm2 after creating or changing the config file because AutoLaunch reads the bridge logging config when the bridge starts.
- If the config file exists but is invalid, the bridge fails fast on startup instead of guessing.

## Optional Extras

### Nerd Font

Grove's intended tree presentation uses Nerd Font glyphs for disclosure arrows, icons, and git dots.

- Recommended: choose a Nerd Font in iTerm2.
- Without a Nerd Font, Grove still works, but the current glyph-heavy tree styling will look degraded.

### Mermaid Rich Rendering

- `mmdc` on `PATH` enables rendered Mermaid diagrams in iTerm2.
- Without `mmdc`, Grove falls back to raw Mermaid source.

### `beautiful-mermaid`

- Release installs place the helper files under `~/.local/share/grove/mermaid/`.
- Source checkouts carry the repo-local helper contract under `tools/mermaid/`.
- It is optional.
- It is not a standalone enablement path.
- In the current runtime it only participates alongside `mmdc`; do not treat it as a baseline dependency.
- The release binary discovers the installed helper layout directly; source builds discover the repo-local helper layout.
- The helper still runs through `node` at runtime.
- In a release install, the installer can run the optional dependency setup for you.
- In a source checkout, install the optional helper dependency with:

```sh
cd tools/mermaid
npm install
```

### Static Image Preview

- Supported local files: `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`
- No extra renderer install is required beyond iTerm2's inline-image support.
- If graphics are unavailable, or the file exceeds Grove's preview budgets, preview falls back to a metadata summary.

## Shell Integration

Grove does not require a separate iTerm shell-integration install.

The runtime emits iTerm2 user variables directly, and the bridge reads session metadata through the iTerm2 Python API. If you already use iTerm2 shell integration, Grove does not depend on it.
Optional PATH profile setup is separate from iTerm shell integration. Grove only offers it when `~/.local/bin` is missing from `PATH`, and skipped or unsupported-shell cases fall back to printed manual instructions.

## Troubleshooting

- `bridge: offline`
  - Confirm the bridge prompt was accepted or the bridge script is in iTerm2 AutoLaunch, then restart iTerm2.
- Need definitive bridge evidence
  - Create `~/.config/grove/bridge-debug.json`, restart iTerm2, reproduce the issue, and inspect the configured JSONL log file.
- AI/editor picker opens but shows no useful targets
  - Confirm Grove is running inside iTerm2 and the other panes are also iTerm2 sessions.
- Mermaid shows raw source
  - Check that `mmdc` is on `PATH`, that `node` is available if you expect the optional `beautiful-mermaid` fallback, and remember that inline Mermaid images are iTerm2-only.
- Image preview shows metadata only
  - Check that Grove is in iTerm2; very large files and oversized dimensions also fall back by design.
- Tree glyphs look broken
  - Switch iTerm2 to a Nerd Font.

# Grove

Grove is a Rust + ratatui terminal file explorer built as an AI coding sidecar for iTerm2 on macOS.
It is keyboard-first, tree-and-preview focused, and optimized for fast path handoff to AI and editor panes.

## Install

Preferred release install:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://raw.githubusercontent.com/hbruss/grove/main/scripts/install.sh | sh
```

Versioned install:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://raw.githubusercontent.com/hbruss/grove/main/scripts/install.sh | sh -s -- --version v0.1.7
```

The installer supports Apple Silicon macOS only, installs into `~/.local`, prompts before wiring the iTerm2 bridge with a default of `Yes`, prompts before optional Mermaid helper dependencies with a default of `No`, and, only when `~/.local/bin` is missing from `PATH`, offers to add it to a supported shell profile with a default of `No`. If profile setup is skipped or the shell is unsupported, the installer prints manual PATH steps instead.

The installer uses the latest published GitHub Release asset. If you are working directly from a checkout and want a local dev path instead, use the source-build fallback in [docs/install.md](docs/install.md).

If you skip the installer's bridge prompt or use the source-build fallback, wire Grove's `grove_bridge.py` into iTerm2 AutoLaunch so `Ctrl+A`, `Ctrl+E`, and `Ctrl+Y` can target other panes.

## Requirements

- macOS
- Apple Silicon for the release installer
- iTerm2 for bridge targeting and inline graphics

## Optional Extras

- Nerd Font for the intended tree styling
- `mmdc` for Mermaid diagram rendering
- `beautiful-mermaid` as an optional Mermaid text-render fallback that the installer or a source checkout can set up; it still needs `node` at runtime

## Documentation

- [Install](docs/install.md)
- [User Guide](docs/user-guide.md)
- [Docs Index](docs/index.md)

## Status

Grove currently optimizes for one canonical current implementation rather than compatibility shims for older local states.

# Grove

Grove is a Rust + ratatui terminal file explorer built as an AI coding sidecar for iTerm2 on macOS.
It is keyboard-first, tree-and-preview focused, and optimized for fast path handoff to AI and editor panes.

## Install Today (Source Build)

Today, Grove installs from source from this repository.
The planned GitHub Releases installer path is not implemented yet.

### Quick Start

```sh
git clone https://github.com/hbruss/grove.git
cd grove
cargo build --release
./target/release/grove
```

If you do not already have Rust and Cargo, install them first from:
https://www.rust-lang.org/tools/install

Optional but recommended for the full iTerm2 workflow: wire `bridge/grove_bridge.py` into iTerm2 AutoLaunch so `Ctrl+A`, `Ctrl+E`, and `Ctrl+Y` can target other panes.

## Requirements

- macOS
- Rust + Cargo (current install path)
- iTerm2 for bridge targeting and inline graphics

## Optional Extras

- Nerd Font for the intended tree styling
- `mmdc` for Mermaid diagram rendering
- `beautiful-mermaid` as an optional source-checkout helper you can install under `tools/mermaid/`

## Documentation

- [Install](docs/install.md)
- [User Guide](docs/user-guide.md)
- [Docs Index](docs/index.md)

## Status

Grove currently optimizes for one canonical current implementation rather than compatibility shims for older local states.

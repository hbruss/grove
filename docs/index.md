# Grove Docs

This documentation tree is the current representation of the repository. It uses wiki links so the code, architecture, and operational model can stay connected as the implementation evolves.

Current implementation status: the repo has a runnable ratatui shell with shallow tree loading, directional tree navigation, lazy one-level expansion/collapse, viewport-aware scrolling, hidden and `.gitignore` visibility toggles that now persist globally, a working path-filter stack with ancestor-preserving filtered rows and demand-driven background indexing only for `/` and `Ctrl+F`, and a preview panel that handles directories, plain text, Grove-owned styled markdown, JSON, bounded binary summaries, optional Mermaid preview targets, bounded selected-file static image previews for local `.png`, `.jpg`, `.jpeg`, `.gif`, and `.webp` files, cached long-form scrolling, wheel scrolling, click-positioned preview cursor movement, Grove-managed line/range selection, preview-local copy on `c`, in-place editor handoff on `Right`, and external open on `o`. On top of that shell, Phases 4 through 9D plus the follow-on multi-select batch-send slice are now wired end to end: startup self-tagging, a Unix-socket bridge client, explicit same-tab then same-window target resolution with manual-picker fallback, keyboard target selection on `Ctrl+A`, file-only `Ctrl+E` editor targeting that opens the selected file immediately on commit, a first-class `Current pane` option at the top of the editor target picker, relative-path injection on `Ctrl+Y`, explicit tree multi-select mode on `m`, row toggling on `Space`, batch clear on `x`, per-tab path-based batch persistence across visibility/filter rebuilds, newline-separated batch send on `Ctrl+Y`, live repo discovery, repo-relative git status refresh, branch-aware repo summaries in the tree and status bar, guarded diff entry on `d` with `p` to return to preview, file-level stage/unstage on `s` / `u`, diff previews with explicit green/red changed-line backgrounds and subtler hunk-header tinting, a modal content-search overlay on `Ctrl+F`, a single command surface on `Ctrl+P` with sectioned empty-query ordering, a unified `Roots` navigator with `Pinned` and `Open` sections that collapse when empty, `Ctrl+T` selected-directory root promotion, `Ctrl+R` add-root browsing from the user home directory with explicit `.` and `..` rows, directory-first root pinning on `b`, collision-safe root labels, prompt/confirm dialogs for file operations, create/rename/duplicate/move/trash flows with overwrite confirmation, copy relative/absolute path actions, reveal in Finder, a Zed-like Nerd Font tree with disclosure glyphs, subtle git dots, and depth tinting, a compact preview metadata lockup with file size, timestamps, permissions, and owner data, a Grove-owned `pulldown-cmark` markdown renderer with inline links and fenced code blocks, preview cursor/range highlight, click-to-position preview interaction, `Shift+Up` / `Shift+Down` range expansion, preview-specific action-bar hints, preview-local copy of rendered lines on `c`, line-aware editor opens for both local-process and shell-target editor modes, a debounced open-root watcher that keeps tree, preview, search, and git state current across external changes while recovering missing selections and missing roots without killing the runtime, a selected-target Mermaid pipeline that detects native Mermaid files plus Markdown Mermaid fences, renders them in the background, uses `mmdc` for iTerm2 inline-image diagrams when available, and otherwise falls back to a status header plus raw Mermaid source, and a selected-file static-image pipeline that prepares supported raster images in the background, reuses the same iTerm2 overlay path for inline display, and falls back to a metadata summary when graphics are unavailable or the preview budget is exceeded.

## Entry Points

- [[install]]
- [[user-guide]]
- [[implementation-blueprint]]
- [[architecture/index]]
- [[architecture/runtime]]
- [[architecture/repository]]
- [[todo/2026-03-18-blueprint-execution-checklist]]

## Install Contract

- Grove is source-build-first in the current checkout.
- The canonical live bridge path is iTerm2 AutoLaunch running `bridge/grove_bridge.py`.
- Grove does not require a separate iTerm shell-integration install.
- Nerd Font rendering is recommended for the intended tree styling, but remains optional.
- Mermaid rendering via `mmdc` and the repo-local `beautiful-mermaid` helper remain optional.
- Inline image preview is iTerm2-only and falls back cleanly when graphics are unavailable or the file exceeds preview budgets.
- The frozen first consumer distribution path is GitHub Releases plus an installer script, not Homebrew-first.

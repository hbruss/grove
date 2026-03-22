# Architecture Index

## Current State

Grove has completed Phase 0, the Phase 1 tree-navigation slice, three Phase 2 slices, five Phase 3 runtime/preview slices, the initial Phase 4 bridge/target-picker slice, the initial Phase 5 git subsystem slice, the Phase 6 keyboard-first discovery slice, the Phase 7 lazy-runtime/tree-visual slice, the Phase 8 bookmarks/file-ops/editor-integration slice, the follow-on root-workflow/preview-clarity slice, the Phase 9A preview-richness slice, the Phase 9B watcher-hardening slice, the Phase 9C Mermaid preview slice, the Phase 9D inline-image preview slice, and the follow-on multi-select batch-send slice. The repository now has:

- a canonical blueprint at [[../implementation-blueprint]]
- current-state docs wired with wiki links
- a minimal Rust crate bootstrap that builds, exits cleanly on `q`, restores terminal state on panic, and serves as the base for the contract freeze
- frozen core contracts across config, state, events, tree, preview, git, bridge, and search
- a renderable shell UI in `src/ui/` and runtime wiring through `bootstrap::run()`, documented in [[runtime]]
- a Phase 1 tree slice: shallow root+immediate-child loading in `tree::loader`, lazy one-level directory expansion/collapse, parent-row selection on `Left`, and viewport-aware tree scrolling documented in [[runtime]]
- a Phase 2 path-filter slice: `/`-focused query input, filtered virtual rows with ancestors preserved, query-clear selection restore, and a background path index documented in [[runtime]]
- a Phase 2 batching slice: incremental path-index batches, partial filter results before completion, and visible indexing status in the path-filter panel
- a Phase 2 visibility slice: `Ctrl+H` hidden-file toggles, `Ctrl+G` `.gitignore` toggles, and visibility-aware tree/filter rebuilds documented in [[runtime]]
- a Phase 3 preview slice: directory summary preview, small plain-text file preview, and cached payload refresh on selection change
- a Phase 3 preview slice: markdown rendering, JSON pretty-printing, and binary summary preview on top of the same cached payload path
- a Phase 3 markdown slice: styled markdown rendering on top of the preview cache path, now superseded by Grove-owned event rendering in the later preview-richness slice
- a Phase 3 navigation slice: global `Ctrl+H` / `Ctrl+G` persistence through `config.toml`, explicit preview focus with `Tab`, and scrollable long-form preview content documented in [[runtime]]
- a Phase 3 ergonomics slice: cached preview render lines, wheel scrolling, in-place editor open on `Right` for files, and external open on `o`, documented in [[runtime]]
- a Phase 4 bridge slice: a Python bridge daemon in `bridge/`, a Rust Unix-socket client in `src/bridge/`, startup Grove self-tagging, explicit same-tab then same-window target resolution, keyboard target picker flows on `Ctrl+A`, file-only `Ctrl+E` target-and-open behavior, a first-class `Current pane` option for editor target selection, relative-path send on `Ctrl+Y`, and bridge-aware action/status surfaces documented in [[runtime]]
- a Phase 5 git slice: live repo discovery through `git2`, repo-relative status caching, tree badge rollups, diff context mode on `d` with `p` to return to preview, tinted diff changed-line rendering, file-level stage/unstage on `s` / `u`, and explicit git warnings/status surfaces documented in [[runtime]]
- a Phase 6 keyboard-first discovery slice: modal content search on `Ctrl+F`, `SearchResults` routing in the right panel, a shared action catalog, a command surface on `Ctrl+P`, and catalog-derived action-bar hints documented in [[runtime]]
- a Phase 7 runtime-and-visual slice: lazy path-index startup only on `/` or `Ctrl+F`, local `.gitignore`-aware directory expansion, a Nerd Font tree renderer with disclosure glyphs, git dots, and depth tinting, plus a structured preview metadata header documented in [[runtime]]
- a Phase 8 workflow slice: real config-backed bookmarks and tab activation, reusable prompt/confirm/target dialogs, create/rename/duplicate/move/trash flows with overwrite confirmation, copy relative/absolute path and reveal-in-Finder actions, and line-aware editor integration for both local-process and shell-target modes documented in [[runtime]]
- a root-workflow/preview-clarity slice: a unified `Roots` navigator with `Pinned` and `Open` sections that collapse when empty, `Ctrl+T` selected-directory root promotion, directory-first root pinning on `b`, collision-safe root labels, a unified sectioned command surface on `Ctrl+P`, repo-level git summaries in both the tree strip and status bar, a preview metadata lockup band with explicit file size, and guarded diff entry that keeps clean or directory selections in preview documented in [[runtime]]
- a root-picker slice: a `Ctrl+R` add-root modal that starts from the user home directory, inherits the live `H`/`G` visibility settings, shows explicit `.` and `..` rows, browses directories without recursive indexing, and pins plus opens/activates the highlighted root on `Enter`, documented in [[runtime]]
- a multi-select batch-send slice: explicit tree multi-select mode on `m`, row toggling on `Space`, batch clear on `x`, path-based per-tab batch state that survives visibility/filter rebuilds while pruning vanished paths during file-op and watcher refresh, and newline-separated root-relative batch send on `Ctrl+Y`, documented in [[runtime]]
- a Phase 9A preview-richness slice: a Grove-owned `pulldown-cmark` markdown renderer with inline links and fenced code blocks, preview cursor/range state rendered from cached lines, preview click positioning, `Shift+Up` / `Shift+Down` range expansion, and preview-local copy on `c`, documented in [[runtime]]
- a Phase 9B watcher-hardening slice: a real debounced `notify` watcher over open roots only, runtime watcher registration/reconciliation in `bootstrap::run()`, refresh-plan application that preserves tree/filter/preview/search state where possible, and missing-root recovery that closes orphaned tabs or retargets the last tab to the nearest surviving parent documented in [[runtime]]
- a Phase 9C Mermaid preview slice: native `.mmd` / `.mermaid` detection, Markdown Mermaid-fence detection, selected-target background Mermaid rendering, optional `mmdc`-backed iTerm2 inline-image presentation, and text/raw fallback states documented in [[runtime]]
- a Phase 9D inline-image preview slice: selected-file static raster image detection for `.png`, `.jpg`, `.jpeg`, `.gif`, and `.webp`, bounded background preparation, shared iTerm2 inline-image overlay presentation, and metadata-summary fallback states documented in [[runtime]]

## Linked Docs

- [[../install]]
- [[../user-guide]]
- [[../implementation-blueprint]]
- [[repository]]
- [[runtime]]

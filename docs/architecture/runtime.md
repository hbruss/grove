# Runtime

## Current Runtime State

Phase 1 tree navigation, three Phase 2 slices, five Phase 3 runtime/preview slices, the initial Phase 4 bridge/target-picker slice, the initial Phase 5 git subsystem slice, the Phase 6 keyboard-first discovery slice, the Phase 7 lazy-runtime/tree-visual slice, the Phase 8 bookmarks/file-ops/editor-integration slice, the follow-on root-workflow/preview-clarity slice, the Phase 9A preview-richness slice, the Phase 9B watcher-hardening slice, the Phase 9C Mermaid preview slice, the Phase 9D inline-image preview slice, and the follow-on multi-select batch-send slice are connected end to end:

- `src/main.rs` calls `grove::bootstrap::run()`
- `src/bootstrap.rs` enters terminal mode in a fail-fast path
- the runtime draws the shell through ratatui before entering the input loop
- the production input loop handles `Up`, `Down`, `Left`, `Right`, `Shift+Up`, `Shift+Down`, `Tab`, `/`, printable path-filter input, `Backspace`, `Ctrl+H`, `Ctrl+G`, `Ctrl+A`, `Ctrl+E`, `Ctrl+R`, `Ctrl+Y`, `Ctrl+F`, `Ctrl+P`, `Ctrl+T`, `Space`, `b`, `c`, `d`, `m`, `p`, `s`, `u`, `x`, `Enter`, `Esc`, `PageUp`, `PageDown`, `Home`, `End`, `o`, wheel scrolling, preview left-click, and `q`
- `Right` lazily expands the selected directory one level at a time, or opens the selected file in the configured editor in the current pane
- `Left` collapses the selected expanded directory or selects its parent row
- `/` focuses the path filter
- `Tab` cycles focus between the tree, roots navigator, and preview panels
- `o` opens the selected file externally through the system opener
- `Ctrl+A` opens the AI target picker
- `Ctrl+E` warns unless the current selection can build a real editor-open request; on a file or search hit it opens the editor target picker with `Current pane` as the first/default option
- `Ctrl+Y` sends a newline-separated list of batched root-relative paths to the AI target when the active tree batch is non-empty, and otherwise falls back to the selected relative path
- `Ctrl+F` opens a modal content-search overlay without mutating the path filter
- `Ctrl+P` opens the unified command surface from the shared action catalog
- `Ctrl+T` opens the selected directory as a root tab or activates an already-open tab for that root
- `Ctrl+R` opens a modal `Add Root` picker that always starts from the user home directory
- `m` toggles explicit multi-select mode while tree focus is active
- `Up` and `Down` move through root entries when roots focus is active
- `Space` toggles the current non-root tree row into or out of the active batch while multi-select mode is enabled
- `b` toggles the selected directory as a pinned root when the tree selection is a directory, and otherwise falls back to the active root
- `c` in preview focus copies the selected rendered preview lines, or the current preview line when no explicit range exists
- `d` switches the preview panel into unstaged git diff mode only when the selected file has cached unstaged or untracked changes; otherwise Grove stays in preview and surfaces a status message
- `p` returns the preview panel to normal file/directory preview mode
- `s` stages the selected file through the git backend
- `u` unstages the selected file through the git backend
- `x` clears the active multi-select batch without leaving tree focus
- `Shift+Up` and `Shift+Down` in preview focus expand or shrink a contiguous line-range selection anchored on the current preview cursor
- unresolved AI-target sends open the manual picker instead of guessing
- `Enter` commits the highlighted picker option when dialog focus is active
- inside the `Add Root` picker, `Left` moves to the parent directory, `Right` enters the highlighted directory row, and `Enter` pins plus opens or activates the highlighted root target
- committing an editor target from the picker immediately opens the selected file through the existing editor-open path
- `Enter` in content search submits the active query, or activates the selected hit when `SearchResults` is already populated
- `Enter` in roots focus opens or activates the selected root, and missing pinned roots now warn in the status bar instead of opening a broken tab or exiting the runtime
- `Esc` cancels the picker and restores the prior focus when dialog focus is active
- `Esc` also closes the content-search overlay and unified command surface while restoring the prior focus
- `Esc` in tree focus exits multi-select mode without clearing the existing batch
- `Esc` in preview focus clears an active preview selection before falling back to other close/blur behavior
- `Ctrl+H` toggles hidden-file visibility when focus is outside the path filter and persists the new preference to `config.toml`
- bare `Backspace` outside the path filter now aliases the hidden-file toggle so terminals that report `Ctrl+H` as plain backspace still get the same behavior
- `Ctrl+G` toggles `.gitignore` respect and persists the new preference to `config.toml`
- path-filter queries rebuild a filtered virtual tree that preserves matching ancestors
- clearing the query restores the prior unfiltered tree selection
- path-index batches can populate filter results before the full index completes
- unfiltered path-index batches now refresh already-expanded directories as more children arrive
- filtered path-index batches preserve the current selected match when it still exists
- tree selection changes trigger an immediate redraw so the styled selection accent follows the selected row
- the tree viewport now tracks `scroll_row` so deep selections stay visible in the rendered panel
- multi-select state is stored as a per-tab path set, so scattered selections can span different visible branches under the same root without depending on transient node IDs
- tree rows now render Nerd Font disclosure glyphs, file/folder icons, subtle git dots, and depth-tinted names instead of raw ASCII markers and git badge text
- the tree now renders a compact repo strip above the rows when the active root is inside a git repo
- the preview panel now shows a directory summary for directory selections
- the preview panel now shows small plain-text file contents for text-like file selections
- markdown files now render through a Grove-owned `pulldown-cmark` event renderer into cached ratatui lines
- markdown previews now keep inline links inline as `label (url)` text instead of appending a detached `Links` section
- fenced code blocks, headings, lists, quotes, task markers, and tables now route through the same Grove-owned markdown line renderer
- native `.mmd` and `.mermaid` files now route through an explicit Mermaid preview model instead of plain-text fallback
- Markdown documents that contain Mermaid fences now attach the first Mermaid block as a selected preview target alongside the cached markdown payload
- selected Mermaid targets render in the background only after the user lands on that preview target; Grove does not prerender Mermaid content at startup or during directory browse
- when `mmdc` is available and Grove is running inside iTerm2, Mermaid preview swaps from pending/raw state into an inline image overlay in the preview panel
- switching away from a Mermaid image preview now clears the previous iTerm overlay before the next non-image preview frame draws, so stale diagrams do not stick behind later previews
- when Mermaid rendering is unavailable, graphics are unsupported, or a render fails, preview shows a status header and the raw Mermaid source below it
- supported local `.png`, `.jpg`, `.jpeg`, `.gif`, and `.webp` files now route through an explicit image preview model instead of the binary fallback path
- selected supported images prepare in the background only after the user lands on that file; Grove does not prerender image previews during tree browse
- when Grove is running inside iTerm2 and the selected image stays within the preview byte and pixel budgets, image preview swaps from pending state into an inline image overlay in the preview panel
- when image graphics are unavailable, the file exceeds the preview budget, or decode/transcode fails, preview stays inside the TUI with a metadata-summary fallback instead of blocking or crashing
- `.json` files now pretty-print in the preview panel
- binary files now render a bounded hex and ASCII summary
- preview content now renders a compact metadata lockup band with path, explicit file size, timestamps, permissions, and owner/group data before the body content
- preview content now tracks its own `scroll_row` and can be navigated while preview focus is active
- preview focus now owns a `cursor_line` plus an optional contiguous line-range selection in rendered-line coordinates
- preview left-click positions the preview cursor on the clicked rendered line and focuses preview
- preview cursor and range highlight are applied at render time on top of cached preview lines, so scrolling and copy reuse the same cached line model
- preview render lines are now cached by preview generation, presentation hint, and panel width so scroll/clamp no longer reparses markdown on every redraw
- diff mode renders unstaged unified diffs for the selected file with explicit green/red changed-line backgrounds and a subtler neutral hunk-header band, and unavailable diff targets stay in preview mode with a status-bar message instead of replacing the right pane with a placeholder
- content search runs against the indexed file snapshot, renders whole-repo hit lists into `SearchResults`, and activation reveals the selected file in the tree before returning preview focus to the matching line
- the production runtime now runs a debounced filesystem watcher over open roots only and applies refresh plans before redraw
- watcher refresh keeps expanded directories, the active path-filter query, and the selected path when they still exist after an external change
- watcher refresh also keeps surviving multi-select paths even when they are currently hidden by filter or visibility settings, and removes only paths that no longer exist under the root
- when the selected path disappears under watcher refresh, Grove moves selection to the nearest surviving sibling or parent and surfaces a warning
- when an open root disappears, Grove closes that tab if another open root remains or retargets the last tab to the nearest surviving parent root instead of aborting the runtime
- wheel scrolling now moves tree selection when the mouse is over the tree and scrolls content when the mouse is over preview
- the action bar is now driven by a shared action catalog and switches between normal hints, picker hints, content-search hints, and unified-command-surface hints
- when tree multi-select mode is active, the action bar switches to batch-specific hints for toggle, clear, done, and `Ctrl+Y` send
- when preview is focused, the action bar now switches to preview-specific hints for scroll, line-range selection, copy, and selection clear
- the unified command surface now shows empty-query sections with selection actions first, then root, git, target, and view actions when they are valid for the current state
- the status bar now shows bridge connectivity, current AI/editor targets, branch-aware git summary counts, multi-select batch counts, picker state, and the latest runtime message; when a runtime message is active it takes precedence over picker detail

## Current Entry Flow

- `src/main.rs` calls `grove::bootstrap::install_panic_hook()`
- `src/main.rs` calls `grove::bootstrap::run()`
- `src/bootstrap.rs` configures terminal state with `TerminalSession::enter()`
- `src/bootstrap.rs` renders one frame with `render_shell_once(...)`
- `src/bootstrap.rs` runs production input through crossterm key, mouse, and resize events
- `src/bootstrap.rs` keeps the byte-reader runtime seam only for deterministic tests
- `src/bootstrap.rs` initializes bridge state and watcher registrations before the first draw
- `src/bootstrap.rs` mutates the active tab through tree-navigation, path-filter, visibility-toggle, editor-open, external-open, bridge-target-picker, relative-path-send, content-search, unified-command-surface actions, root-tab promotion, prompt/confirm dialogs, guarded diff-mode entry, file-ops, root pin/unpin, and git stage/unstage actions
- `src/bootstrap.rs` persists visibility preferences after successful `Ctrl+H` and `Ctrl+G` toggles
- `src/bootstrap.rs` suspends Grove around editor and external-open commands, then restores the terminal and redraws
- `src/bootstrap.rs` self-tags Grove with iTerm2 user variables, pings the bridge socket, and leaves the shell usable if the bridge is unavailable
- `src/bootstrap.rs` lists candidate sessions through the bridge when target selection is requested or when unresolved send requests fall back to manual selection
- `src/bootstrap.rs` commits AI target assignment and remote editor target assignment through the bridge `SetRole` path, treats `Current pane` editor selection as a local runtime state change, and immediately opens the selected file after a successful editor-target picker commit
- `src/bootstrap.rs` commits prompt dialogs against raw user-typed relative paths, opens overwrite/trash confirmations when needed, and refreshes the active tab around successful file mutations
- `src/bootstrap.rs` stages or unstages only the selected file path, rejects root/directory/conflicted selections explicitly, invalidates preview after successful git mutation, and relies on the normal redraw path to refresh git state
- `src/app.rs` leaves the recursive path index idle until `/` or `Ctrl+F` demands it
- `src/app.rs` accumulates partial path-index entries and reapplies the active query as batches arrive
- `src/app.rs` processes the background path index in bounded per-tick batch slices so large visibility rebuilds do not monopolize the UI thread
- `src/app.rs` rebuilds the active tab when hidden or `.gitignore` visibility toggles change, invalidates the recursive snapshot, and keeps lazy browsing responsive
- `src/app.rs` owns per-tab multi-select mode and root-relative batch state, reuses it across visibility/filter rebuilds, and prunes vanished paths during file-op and watcher refresh without relying on loaded node IDs
- `src/app.rs` refreshes the cached preview payload when the selected path changes
- `src/app.rs` owns per-tab content-search query, generation, worker/runtime handles, result payloads, and selected-hit state
- `src/app.rs` caches repo discovery and repo-relative git status state per tab, rolls git badges onto loaded tree nodes, and invalidates preview payloads when mode or git mutation changes require a fresh render
- `src/app.rs` owns roots-navigator selection, config-backed pinned-root persistence, direct active-root pin toggling, root-tab activation/promotion, collision-safe root labels, preview diff-availability checks, and post-file-op tree refresh/reveal semantics
- `src/app.rs` caches preview payloads as a panel title plus a structured metadata header, plain body lines, and optional markdown, image, or Mermaid preview contracts
- `src/app.rs` caches rendered preview lines separately from preview payloads so runtime scroll operations can reuse them
- `src/app.rs` owns per-tab image-preview request-key generation, bounded background image worker state, stale-result rejection, and inline-image payload state for the active selected image
- `src/app.rs` owns per-tab Mermaid discovery, request-key generation, background render worker state, stale-result rejection, and inline-image payload state for the active preview target
- `src/app.rs` owns preview cursor state, contiguous range selection state, preview click line targeting, copy-range helpers, and selection invalidation/clamping across preview changes
- `src/app.rs` owns picker selection state against the current bridge session list, unified-command-surface overlay state against the shared action catalog, and restores prior focus when overlays close
- `src/open.rs` resolves line-aware editor open requests into either local-process invocations or shell-target command lines
- `src/bootstrap.rs` keeps `scroll_row` aligned to the visible tree viewport height before redraw
- `src/bootstrap.rs` clamps preview scroll against the rendered preview line count and preview viewport height before redraw
- `src/bootstrap.rs` polls the path-index worker, content-search worker, image render worker, Mermaid render worker, watcher root reconciliation, debounced watcher refresh plans, and git refresh before redraw, then exits on `q` or EOF
- `src/bootstrap.rs` emits inline preview images through an iTerm2-only post-draw overlay path after ratatui finishes the normal preview frame, and the same overlay seam now serves both Mermaid diagrams and static image previews
- `src/bootstrap.rs` reconciles watcher roots again after watcher-driven root changes so removed or recovered tabs do not stay registered for another tick
- `src/debug_log.rs` appends timestamped file logs when `GROVE_DEBUG_LOG` is set, and the current probes cover visibility rebuilds, path-index polling, and render timing

## Current Bridge Semantics

- the bridge daemon lives at `bridge/grove_bridge.py`
- the development launcher lives at `scripts/run_bridge_dev.sh`
- the Rust bridge client speaks newline-delimited JSON over a Unix socket at `$TMPDIR/grove-bridge-<uid>.sock`
- Grove tags its own pane with `user.groveRole=grove` and a generated `user.groveInstance=<instance_id>` at startup
- bridge target resolution is explicit and ordered:
  1. same tab
  2. same window
  3. manual picker
- heuristic target guessing is not implemented in the current slice
- AI and editor targets are tracked separately in runtime state
- the editor target picker prepends a synthetic `Current pane` row above bridge session rows, while the AI picker remains session-only
- picker rows show either `Current pane` for the local editor path or session title, role, job name, and window/tab location hints for bridge-backed targets
- bridge target assignment currently acknowledges success through the generic bridge `Pong` response
- target choices are runtime-only today; the persisted target contract exists, but startup state load/save is not wired yet

## Current Git Semantics

- the active tab discovers a repo from its root through `LibgitBackend`
- git state is cached as repo-relative per-path status entries plus an optional discovered repo handle with the current branch label
- clean cached git state is reused across redraws; render no longer rescans the repo unless startup or a runtime action marks git state dirty
- arbitrary external repo changes under an open root now mark git state dirty through the debounced watcher path instead of relying on per-render rescans
- tree nodes render compact git dots for direct file status and rolled-up directory status
- the tree strip and status bar both derive repo-level staged, unstaged, untracked, and conflicted counts from that cached status map instead of triggering extra backend refreshes
- collapsed directories still receive badges when only unloaded descendants are changed
- diff mode is a per-tab context mode that routes preview loading through the git backend's unstaged unified diff path
- `p` returns the active tab to normal preview routing without changing tree selection
- `s` and `u` are file-only actions; root, directories, and conflicted selections surface explicit warnings instead of guessing
- successful stage/unstage invalidates the active preview before redraw so diff mode does not keep stale content
- stage/unstage accepts regular files and symlink files; broken symlinks stage as file entries rather than being misread as deletions

## Current Search and Action-Surface Semantics

- path filter and content search are distinct systems with separate focus, query, and result state
- `Ctrl+F` opens content search over the current tab; `/` still opens the path filter
- content search results are generation-scoped so stale worker responses are ignored
- content search now reuses the growing indexed snapshot while the path index is still building instead of freezing on the first empty-snapshot submission
- the first content-search slice is text-first and whole-repo against the indexed file snapshot; binary-ish files are skipped
- `SearchResults` is a right-panel context mode, not a tree mutation
- activating a search hit reveals the file in the tree, switches the tab back to normal preview mode, and seeds the preview jump so the matching line stays selected after redraw
- the shared action catalog is the canonical source for action-bar entries and unified-command-surface rows
- action availability is catalog-driven, so unavailable git mutations and unavailable diff entry stay hidden instead of surfacing as dead UI
- the unified command surface filters enabled action entries by label and hint text, and empty-query ordering groups them into selection, root, git, target, and view sections
- root pin/unpin, root-tab promotion, close-tab, create/rename/duplicate/move/trash, reveal, copy-path, relative-path send, and editor-open actions all route through the same catalog surface
- overlay precedence is explicit: target picker, unified command surface, content search, then base runtime

## Current Root Pinning, File-Op, and Editor Semantics

- pinned roots persist in `config.toml` and render inside the unified `Roots` navigator alongside open-but-unpinned session roots
- pinned roots are root-only state; files do not participate in root-pinning semantics
- root pin actions target the selected directory root when one is highlighted in the tree; file selections and root selections fall back to the active root
- the `Add Root` picker is directory-only, starts from `$HOME` on every open, and inherits the current global hidden-file and `.gitignore` visibility settings without starting the recursive path index
- the `Add Root` picker always renders explicit `.` and `..` rows, so the current directory and its parent can be committed as root targets without overloading navigation keys
- activating a pinned root opens it as a new tab or switches to the existing tab if that root is already open
- open roots that are also pinned render only once under the `Pinned` section, while transient session roots render under `Open`
- the `Roots` navigator collapses empty sections instead of rendering a redundant `none` row, and pinned rows do not repeat open state with a literal `open` badge
- root labels share one collision-safe helper, so basename collisions pick up a dim parent-path disambiguator
- add-root commit failures stay inside the picker with an inline error plus status-bar error instead of aborting the runtime, and failed bookmark writes roll back the in-memory pin mutation before returning control
- prompt dialogs preserve the raw typed relative path, including leading or trailing spaces; only the truly empty string is rejected
- create and move flows accept root-relative destination paths; rename seeds the current sibling name, and duplicate seeds a sibling `copy` destination
- rename, duplicate, and move detect destination collisions, open an explicit overwrite confirmation, move the destination aside into a temporary backup, and restore it if the replacement fails before cleanup completes
- successful file mutations rebuild the active tab shallow tree, replay prior directory expansion, reveal the resulting path when one exists, invalidate preview/search state as needed, and mark git state dirty
- copy relative path and copy absolute path use the system clipboard
- reveal in Finder routes through the platform-specific reveal command instead of mutating Grove state
- file-op and clipboard failures surface an error in the status bar and keep the runtime alive instead of unwinding out of the TUI
- line-aware editor opens use the selected search-hit line in `SearchResults`, the current preview scroll row in normal preview mode, and the diff-derived first changed line in diff mode
- editor opens resolve to either a suspended local-process command in the current pane or an injected shell-target command against the bound editor pane, and both modes share the same precedence: explicit Grove command, then `$EDITOR`, then `micro`
- when no explicit editor target session is set, Grove now labels that state as `current pane` instead of `unset`

## Current Tree Semantics

- the root is loaded shallowly on startup
- unloaded directories gain children only when explicitly expanded
- expanding a directory with `.gitignore` respect enabled now uses a local ignore-aware child walker instead of building a recursive subtree snapshot
- a full path index can merge unloaded descendants into the arena without changing canonical expansion state
- re-expanding a previously loaded directory does not duplicate nodes
- collapsing a directory hides descendants but preserves loaded children in the arena
- `Left` on a non-root child row selects the parent when the selected node is not expanded
- filtered rows are derived from matches plus ancestors, not from the current expansion-only row set
- hidden-file visibility and `.gitignore` respect are applied consistently to the loader and the background path index
- hidden-file and `.gitignore` toggles rebuild the visible tree immediately but only restart recursive indexing when a live path-filter or content-search query actually needs it
- visibility rebuilds now replay previously expanded directories before selection restore so deep tree context survives `Ctrl+H` and `Ctrl+G`
- visibility rebuilds preserve multi-select batches for still-existing paths even when those rows become temporarily invisible under the new visibility settings
- visibility rebuilds replace the active background index, and superseded index workers now stop when their receiver is dropped instead of finishing stale full-tree walks
- the path-filter panel renders compact visibility state as `H` and `G` flags next to indexing status
- preview payloads are cached by selected relative path so redraws do not reread the same file repeatedly
- preview payloads now keep raw markdown source separate from the structured header and plain preview lines so the UI can render markdown at the current panel width
- preview payloads can now also carry explicit Mermaid source/display/status state for native Mermaid files or the first fenced Mermaid block in Markdown
- preview render caches are invalidated when preview generation changes or preview width changes
- preview render caches can now reserve a fixed-height Mermaid image slot so the iTerm2 overlay lands in a stable region of the right pane
- preview focus owns line-wise and page-wise scrolling without stealing tree navigation keys while tree focus is active
- preview navigation keeps the preview cursor aligned with scroll operations, while `Shift+Up` / `Shift+Down` extend a separate explicit range selection
- raw visibility walks do not recurse through symlink directories
- preview routing now handles directory summary, binary summary, Grove-owned styled markdown rendering, optional Mermaid targets, JSON pretty-printing, and plain-text fallback
- global hidden-file and `.gitignore` visibility preferences are canonicalized in `config.toml`, not runtime state
- editor resolution prefers an explicit Grove editor command, then shell-parsed `$EDITOR`, then `micro`
- picker selection is keyed by an explicit picker option enum: either `Current pane` or a selected session id against the current bridge session list

## Current Watcher Semantics

- the production runtime constructs a real `notify` watcher and keeps it behind the `WatcherService` seam so the deterministic test path can still inject a stub
- only currently open roots are watched; pinned-but-closed roots are not registered
- watched-root identity now canonicalizes the nearest surviving ancestor and appends any missing suffix, so removed roots still match their previously watched canonical path
- raw filesystem events are normalized, bucketed by watched root, and coalesced after `config.watcher.debounce_ms`
- watcher refresh rebuilds the shallow tree, replays expanded directories, preserves the path-filter query, invalidates stale preview/search/diff state only when needed, and marks git state dirty when the plan says repo state changed
- watcher refresh reconciles per-tab multi-select batches by path existence under the root, so vanished paths drop out while hidden or filtered survivors remain queued
- missing selected paths recover to the nearest surviving sibling or parent with an explicit warning instead of leaving a ghost row behind
- if a watched root disappears and other tabs remain open, Grove closes the missing-root tab, activates the nearest surviving tab, and immediately reconciles watcher registrations
- if the last watched root disappears, Grove recovers that tab to the nearest surviving parent root and immediately reconciles watcher registrations against the new root

## Install And Dependency Contract

- the canonical public install path is `scripts/install.sh`, which downloads `grove-aarch64-apple-darwin.tar.gz` from GitHub Releases and installs into `~/.local`
- the installer currently supports Apple Silicon macOS only, accepts `--version <tag>` and `--yes`, and keeps its prompts explicit instead of silently changing other tools
- source builds currently use `cargo build --release`, which produces `target/release/grove`
- source builds remain the fallback and developer path
- the canonical live bridge path is iTerm2 AutoLaunch running Grove's installed `grove_bridge.py`; `scripts/run_bridge_dev.sh` remains a dev-only shell path
- the installer prompts before wiring the iTerm2 AutoLaunch bridge and defaults that prompt to `Yes`
- Grove does not require a separate iTerm shell-integration install; the runtime emits iTerm2 user variables directly and the bridge reads session metadata through the iTerm2 Python API
- iTerm2-only behavior includes live bridge targeting, Mermaid inline-image presentation, and static image inline preview; outside iTerm2 Grove keeps the TUI usable and falls back to warnings, raw source, or metadata summaries
- Nerd Font rendering is recommended for the intended tree presentation; without it the UI remains functional but the glyph-heavy tree styling degrades
- Mermaid rich rendering is optional and requires `mmdc` on `PATH`
- the installer prompts before optional Mermaid helper setup and defaults that prompt to `No`
- the repo-local `beautiful-mermaid` helper under `tools/mermaid/` is optional and only participates alongside `mmdc`; it is not a baseline dependency, and source checkouts install it with `npm install` from `tools/mermaid/`
- static raster image preview needs no extra renderer beyond iTerm2, but only supports local `.png`, `.jpg`, `.jpeg`, `.gif`, and `.webp` files that stay within the preview budgets
- Homebrew is not the primary install path for the first release

## Safety Semantics

- Terminal setup is fail-fast. If raw mode or alternate-screen setup fails, `run()` returns an error.
- Terminal restore remains fail-safe and always attempts raw-mode disable during teardown.
- The runtime seam is directly testable with a ratatui `TestBackend` through `run_with_terminal_and_reader(...)`.

## Current Limitation

- The production runtime now uses crossterm events, but the deterministic test seam still uses the older byte-reader input path. Those two paths intentionally share the same runtime action handler, but they are not yet fully unified.
- Mouse support in the current slice is preview/tree wheel scrolling plus preview left-click cursor positioning. Click actions elsewhere, divider drag, and mouse-driven root interactions are still out of scope.
- The bridge daemon requires the iTerm2 Python API environment. The Python bridge unit tests pass in a normal shell, but live bridge control still needs iTerm2 AutoLaunch or another environment where the `iterm2` module is available.
- Mouse-invoked action surfaces, clear-target flows, and copy/inject variants beyond relative-path send are still out of scope.
- Bridge target choices are not persisted across Grove restarts yet.
- Git diff mode is currently unstaged-only. Separate staged/HEAD diff views, blame, history, and broader git actions are still out of scope.
- Preview loading is still inline on selection change for normal files. Mermaid render work is the current exception: discovery stays inline, but the actual diagram render happens on a background worker after selection.
- HTML files still preview as raw source. Grove does not attempt inline browser rendering in the preview panel.
- Preview selection is Grove-managed and line-based. Native terminal drag selection and snippet-send from preview are still out of scope.
- Mermaid rendering is selected-target-only. Native `.mmd` / `.mermaid` files and the first fenced Mermaid block in a Markdown document can render through `mmdc` in iTerm2, while supported local raster images use the separate selected-file image pipeline.
- The optional `beautiful-mermaid` ASCII fallback is implemented through the repo-local helper contract, and the helper dependency can be installed under `tools/mermaid/` in a source checkout.
- YAML files still use plain-text preview. Rich YAML parsing is deferred until the parser choice is frozen.
- Closed roots are not watched. External changes under a pinned-but-closed root do not surface until that root is opened again.

## Linked Docs

- [[index]]
- [[../implementation-blueprint]]


# Grove Implementation Blueprint

Status: implementation plan  
Date: 2026-03-18  
Target platform: macOS on Apple Silicon, iTerm2, Rust  
Assumption: your note says "64 mb of ram" but I am treating the machine as the M1 MacBook Pro with 64 GB RAM from the prior plan.

This document is the build blueprint for Grove. It is written so you can hand it to Codex CLI, run it in one or more sessions, and build the product in controlled phases without drifting into unnecessary scope.

## 1. Executive summary

Grove is a fast, local-first terminal file explorer that runs in one iTerm2 pane and acts as a context sidecar for AI coding tools running in other panes. The product is not a generic file manager first. It is an AI workflow accelerator first.

Keep from the original concept:

- Rust + ratatui for the TUI
- iTerm2 Python bridge for pane control
- `.gitignore` aware walking
- rich text preview for source files
- git status, diff, blame, and file info
- direct send actions such as path, `@ref`, diff, and snippet
- tabs, bookmarks, mouse support, command palette, and context menu

Change from the original concept:

1. Use a lazy tree plus a flat visible row model from the beginning.
2. Make the iTerm2 bridge an AutoLaunch daemon, not an ad hoc `python ... &` helper.
3. Separate the AI target from the editor target.
4. Split path filtering from content search.
5. Ship text-first preview in v1 and defer PDF and drag-drop. Bounded selected-file static raster images and Mermaid diagrams in iTerm2 are the explicit narrow preview exceptions.
6. Use trash semantics only. No permanent delete in the UI.
7. Use bounded channels and explicit cancellation for all background work.
8. Prefer small persistent state and runtime caches only. No persistent repo database in v1.

The success criteria are simple:

- first frame fast
- idle CPU near zero
- low memory growth on large repos
- no redraw stutter
- no destructive file operation surprises
- reliable pane targeting
- elegant default UX with discoverable actions

## 2. Product definition

### 2.1 Core job to be done

When you are coding with Claude Code CLI, Codex CLI, or another AI TUI in iTerm2, Grove should let you:

- find files instantly
- inspect the contents, diff, blame, and metadata without leaving the terminal
- send the right context to the AI pane with one action
- open the file in an editor or reveal it in Finder without breaking your flow
- handle common project navigation and lightweight file operations without feeling like a bloated file manager

### 2.2 Primary workflows

1. Navigate a repo, select a file, send an `@ref` or path to the AI pane.
2. Inspect a changed file in diff mode and send the diff to the AI pane.
3. Jump to a search result, preview the file at that line, and send a small snippet.
4. Open the same file in a real editor, either as a local GUI process or in a separate editor shell pane.
5. Create, rename, duplicate, move, or trash a file while staying in the same terminal layout.
6. Work across several projects with tabs and bookmarks.

### 2.3 Hard non-goals for v1

These are intentionally out of v1:

- general inline image preview beyond bounded selected-file local raster images in iTerm2
- PDF preview
- drag and drop
- permanent delete
- embedded terminal editor
- remote filesystem support as a first-class goal
- tmux-specific integration
- persistent on-disk index database
- a giant plugin system
- full mouse text selection inside preview

## 3. Design principles

### 3.1 Performance principles

- First frame matters more than full index completion.
- Never block the UI thread on disk, git, bridge, or preview work.
- Draw only when state changed or a timer actually expired.
- Use bounded channels. Drop stale work instead of queueing forever.
- Every background result must carry a generation ID so stale results are ignored.

### 3.2 UX principles

- Mouse-first and keyboard-complete.
- Every important action must exist in three places: action bar, context menu, and command palette.
- The UI should stay quiet. Minimal chrome, thin borders, useful spacing.
- Path filter is always visible and instant.
- Content search is powerful but clearly separate so it never slows down path navigation.
- Advanced behavior should feel obvious from the UI. Do not force memorization.

### 3.3 Scope principles

- Grove is an AI coding sidecar, not Finder in a terminal costume.
- File operations are useful, but they must not dominate the design.
- Prefer fewer features implemented well over many brittle features.

## 4. Success metrics and budgets

These are the budgets the implementation should target. If a feature violates these budgets, the feature is wrong until proven otherwise by measurement.

| Metric | Target |
|---|---:|
| Time to first frame in a medium repo | under 150 ms |
| Path filter response after keystroke | under 40 ms |
| Preview load for text files under 1 MiB | under 100 ms |
| Idle CPU | under 1% |
| Idle memory in a normal repo | under 80 MiB |
| Memory in a very large repo around 100k entries | under 180 MiB |
| File watcher CPU when idle | effectively zero |
| Full content search on a medium repo | under 750 ms |
| Bridge reconnect after daemon restart | under 2 s |

Rules that protect these budgets:

- No full recursive tree render every frame.
- No unbounded preview cache.
- No permanent timer tick loop at 100 ms when nothing is animating.
- No sync git refresh in the render path.
- No shell-out preview hook without timeout and output cap.

## 5. Architecture overview

### 5.1 Process model

```text
iTerm2 window
+---------------------------------------------------------------+
| Pane A: Grove TUI                                             |
|   - left: tree + path filter + bookmarks + action bar         |
|   - right: preview / diff / blame / info / search             |
|   - bottom: status bar                                        |
|                                                               |
| Pane B: AI target                                             |
|   - Claude Code CLI / Codex CLI / other terminal AI           |
|                                                               |
| Pane C: Editor target (optional)                              |
|   - shell running nvim, vim, helix, etc.                      |
+---------------------------------------------------------------+

Background components
- Rust app: single UI thread + worker threads
- iTerm2 AutoLaunch Python daemon: bridge and pane API access
- notify watcher
- git backend
```

### 5.2 High-level runtime model

Use a synchronous UI core with worker threads and message passing.

Do not use Tokio in v1.

Reason:

- Most work here is either terminal event handling or blocking local IO.
- `git2`, file walking, file preview, and `notify` are synchronous or callback based.
- A simple UI thread plus worker threads with bounded channels is easier to reason about, easier to benchmark, and less likely to create accidental idle overhead.

### 5.3 Core threads

1. UI thread  
   Owns all mutable app state and performs every draw.

2. Input thread  
   Reads crossterm input events and forwards them to the UI thread.

3. Watcher thread  
   Owns `notify` watcher registration and forwards coalesced filesystem events.

4. Bridge client thread  
   Maintains the Unix socket connection to the Python daemon and forwards responses/events.

5. Preview worker  
   Handles preview loading and rendering prep.

6. Search worker  
   Handles content search jobs.

7. Git worker  
   Handles git status refresh, diff, blame, and history jobs.

8. Background indexer  
   Builds and updates the project path index without blocking the first frame.

All communication goes through bounded crossbeam channels.

## 6. Key decisions and rationale

| Area | Decision | Avoid | Why |
|---|---|---|---|
| Tree model | Arena of nodes + flat visible row list | Recursive render tree walking every frame | Better startup, scrolling, and filter performance |
| Loading | Lazy per-directory load plus background full index | Synchronous full repo walk before first draw | First frame stays fast |
| Search | Path filter always-on; content search separate | One search box that sometimes blocks | Keeps path navigation instant |
| Bridge lifecycle | iTerm2 AutoLaunch daemon | Ad hoc background Python process | Cleaner install, fewer env issues |
| Pane targeting | Explicit role tags + instance ID + fallback heuristics | Title matching alone | More reliable in real multi-pane layouts |
| Targets | Separate AI target and editor target | One target for all actions | Avoids opening a file in the AI prompt |
| Delete behavior | Move to Trash only | Permanent delete | Safer daily-driver behavior |
| Preview scope | Text-first v1 with bounded iTerm2 Mermaid and static-image exceptions | broad image/PDF rendering in v1 | Less redraw complexity and lower risk |
| Watcher model | Watch full root recursively, filter in app, rescan on demand | Register watcher with display-level ignores | Prevents missing `.git` metadata changes |
| Multiline injection | Conservative and configurable | Assuming all AI CLIs behave the same | Terminal apps differ on multiline input |
| Persistence | Small config/state only | Persistent repo DB | Less bloat, simpler invalidation |
| Error handling | Degrade per-feature, never crash the TUI | Fatal errors for optional subsystems | Better day-to-day reliability |

## 7. Detailed UX specification

### 7.1 Layout

Inside the Grove pane, use this layout:

```text
+---------------------------------------------------------------+
| Tabs                                                          |
+--------------------------+------------------------------------+
| Path filter              | Context tabs                       |
+--------------------------+------------------------------------+
| Bookmarks                | Preview / Diff / Blame / Info /    |
+--------------------------+ Search                             |
| Tree rows                |                                    |
|                          |                                    |
|                          |                                    |
|                          |                                    |
+--------------------------+------------------------------------+
| Action bar                                                   |
+---------------------------------------------------------------+
| Status bar                                                   |
+---------------------------------------------------------------+
```

### 7.2 Focus areas

`Focus` enum:

- `Tree`
- `Preview`
- `PathFilter`
- `ContentSearch`
- `CommandPalette`
- `ContextMenu`
- `Dialog`

Rules:

- `Tab` cycles Tree -> Preview -> PathFilter -> Tree.
- `Shift+Tab` cycles backward.
- Opening a modal or menu steals focus and restores it on close.
- Path filter clears focus with `Esc` but does not wipe the query unless `Esc` is pressed twice or the clear button is clicked.

### 7.3 Tree behavior

Left panel contents, top to bottom:

1. Tabs
2. Path filter
3. Bookmarks
4. Tree rows
5. Action bar

Tree behavior:

- The tree preserves expansion state when path filter is empty.
- When path filter is active, the UI shows a virtual filtered tree consisting of matches plus ancestors.
- Filtering must never mutate the real expansion state.
- Selecting a directory previews a directory summary by default.
- Selecting a file previews the current active context mode for that file.
- Multi-select is supported for files and directories.
- Directories can be expanded/collapsed by click on chevron or `Left` / `Right`.

### 7.4 Context panel modes

Modes:

- `Preview`
- `Diff`
- `Blame`
- `Info`
- `SearchResults`

Behavior:

- Mode persists while selection changes.
- If mode is not valid for the selected item, fallback gracefully:
  - diff/blame on a non-git file falls back to Preview with a status note
  - blame on a directory falls back to Info
- Search results mode is entered only by content search actions, not by path filter.

### 7.5 Action surfaces

Every meaningful action must be reachable in all three surfaces:

1. Action bar
2. Right-click context menu
3. Command palette

Actions to surface prominently:

- Send absolute path
- Send relative path
- Send `@ref`
- Send contents
- Send diff
- Send snippet
- Open in editor
- Reveal in Finder
- New file
- New folder
- Rename
- Duplicate
- Move
- Trash
- Pin / unpin bookmark
- Set AI target
- Set editor target
- Switch context mode
- Toggle hidden
- Toggle `.gitignore`
- Start content search

### 7.6 Mouse model

Required mouse support in v1:

- left click select
- click chevron expand/collapse
- right click context menu
- scroll wheel over tree or preview
- click divider to drag-resize panels
- click tab to activate
- click bookmark to activate or create a tab
- click preview line to position the preview cursor
- double-click file to open in editor
- double-click directory to expand/collapse

Not required in v1:

- drag and drop
- mouse drag text selection in preview

### 7.7 Keyboard model

Use simple terminal-safe defaults. Do not depend on `Cmd` key handling in the terminal.

Recommended defaults:

| Action | Key |
|---|---|
| Focus path filter | `/` |
| Open content search | `Ctrl+F` |
| Command palette | `Ctrl+P` |
| Next focus | `Tab` |
| Previous focus | `Shift+Tab` |
| Move selection | `Up` / `Down` |
| Expand/collapse | `Right` / `Left` |
| Page scroll | `PageUp` / `PageDown` |
| Top/bottom | `Home` / `End` |
| Open or toggle directory | `Enter` |
| Toggle multi-select mode | `m` |
| Toggle current row in multi-select | `Space` |
| Extend preview line selection | `Shift+Up` / `Shift+Down` |
| Toggle hidden files | `Ctrl+H` |
| Toggle `.gitignore` respect | `Ctrl+G` |
| New tab | `Ctrl+T` |
| Close tab | `Ctrl+W` |
| Resize left panel smaller/larger | `Ctrl+[` / `Ctrl+]` |
| Switch tabs | `Alt+1` through `Alt+9` |
| Clear / close | `Esc` |

Important: do not depend on `Ctrl+digit` as a primary control in terminals.

### 7.8 Bookmarks and tabs

Bookmarks are pinned roots, not mutable in-tree shortcuts.

Rules:

- Clicking a bookmark activates an existing tab for that root, or opens a new tab if none exists.
- Bookmarks do not silently mutate the current tab root.
- Tabs represent project roots.
- Each tab owns:
  - root path
  - tree state
  - visible rows
  - selection and multiselect state
  - current context mode
  - path filter state
  - content search state
  - git repo info
  - scroll offsets

This model is easier to understand and easier to persist safely.

## 8. Scope by release

### 8.1 v1.0 daily-driver scope

Must ship in v1:

- lazy tree and flat visible rows
- path filter
- content search overlay and results panel
- text preview with syntax highlight
- markdown render via `pulldown-cmark`
- JSON / YAML pretty preview
- binary hex preview
- directory info preview
- git status in tree
- unified diff
- blame
- file info and history
- iTerm2 bridge AutoLaunch daemon
- explicit AI target and editor target
- send path / relative / `@ref` / contents / diff / snippet / tree
- tabs
- bookmarks
- create file
- create directory
- rename
- duplicate
- move
- trash
- open in local editor or editor target
- reveal in Finder
- command palette
- context menu
- click, scroll, resize
- config + persisted state
- hardening and benchmark coverage

### 8.2 v1.1 quality-of-life scope

Do after v1 is solid:

- stage / unstage from UI
- preview hooks
- shell wrapper for standalone `cd` on exit
- richer search options
- more polished theme options
- optional ANSI-aware hook output

### 8.3 v2 and later

- inline image preview beyond bounded selected-file raster support in iTerm2
- PDF preview
- drag and drop
- side-by-side diff
- richer markdown rendering
- more advanced selection and snippet tools
- extended plugin surface if there is a real need

## 9. Project structure

Use this repository layout:

```text
grove/
├── src/
│   ├── main.rs
│   ├── app.rs
│   ├── action.rs
│   ├── event.rs
│   ├── error.rs
│   ├── config.rs
│   ├── state.rs
│   ├── dirs.rs
│   ├── bootstrap.rs
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── root.rs
│   │   ├── layout.rs
│   │   ├── theme.rs
│   │   ├── tabs.rs
│   │   ├── path_filter.rs
│   │   ├── bookmarks.rs
│   │   ├── tree.rs
│   │   ├── preview.rs
│   │   ├── status_bar.rs
│   │   ├── action_bar.rs
│   │   ├── command_palette.rs
│   │   ├── context_menu.rs
│   │   └── dialog.rs
│   ├── tree/
│   │   ├── mod.rs
│   │   ├── model.rs
│   │   ├── loader.rs
│   │   ├── indexer.rs
│   │   ├── visible.rs
│   │   ├── filter.rs
│   │   ├── sort.rs
│   │   └── watcher.rs
│   ├── preview/
│   │   ├── mod.rs
│   │   ├── model.rs
│   │   ├── loader.rs
│   │   ├── syntax.rs
│   │   ├── markdown.rs
│   │   ├── json_yaml.rs
│   │   ├── binary.rs
│   │   ├── directory.rs
│   │   ├── diff.rs
│   │   ├── blame.rs
│   │   ├── info.rs
│   │   └── hooks.rs
│   ├── search/
│   │   ├── mod.rs
│   │   ├── path.rs
│   │   └── content.rs
│   ├── git/
│   │   ├── mod.rs
│   │   ├── backend.rs
│   │   ├── libgit2.rs
│   │   └── model.rs
│   ├── bridge/
│   │   ├── mod.rs
│   │   ├── client.rs
│   │   ├── protocol.rs
│   │   └── tagging.rs
│   ├── actions/
│   │   ├── mod.rs
│   │   ├── inject.rs
│   │   ├── file_ops.rs
│   │   └── open.rs
│   └── util/
│       ├── mod.rs
│       ├── text.rs
│       ├── paths.rs
│       └── debounce.rs
├── bridge/
│   ├── __init__.py
│   ├── grove_bridge.py
│   └── test_grove_bridge.py
├── scripts/
│   └── run_bridge_dev.sh
├── src/
│   └── ...
├── tests/
│   └── ...
├── tools/
│   └── mermaid/
│       ├── package-lock.json
│       ├── package.json
│       └── render_ascii.mjs
├── docs/
│   ├── index.md
│   ├── install.md
│   ├── user-guide.md
│   ├── implementation-blueprint.md
│   ├── architecture/
│   ├── ai/
│   ├── plans/
│   └── todo/
├── Cargo.toml
├── Cargo.lock
└── rust_out
```

## 10. Core data model and contracts

Freeze these contracts early so parallel workstreams can proceed safely.

### 10.1 App events

```rust
pub enum AppEvent {
    Input(InputEvent),
    Fs(FsBatch),
    Index(IndexBatch),
    PreviewReady(PreviewGeneration, PreviewPayload),
    SearchReady(SearchGeneration, SearchPayload),
    GitReady(GitGeneration, GitPayload),
    Bridge(BridgeEvent),
    Timer(TimerKind),
}
```

### 10.2 Actions

`Action` is the command surface for UI and command palette. It must be stable early.

Core action groups:

- navigation
- selection
- path filter
- content search
- mode switching
- injection
- clipboard
- file operations
- tabs
- bookmarks
- bridge and target actions
- toggles
- dialogs

### 10.3 App state

```rust
pub struct App {
    pub config: Config,
    pub state: PersistedState,
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub focus: Focus,
    pub status: StatusBarState,
    pub overlays: OverlayState,
    pub bridge: BridgeState,
    pub timers: TimerRegistry,
    pub should_quit: bool,
}
```

### 10.4 Tab state

```rust
pub struct TabState {
    pub root: PathBuf,
    pub tree: TreeState,
    pub multi_select: MultiSelectState,
    pub preview: PreviewState,
    pub path_filter: PathFilterState,
    pub content_search: ContentSearchState,
    pub git: GitTabState,
    pub split_ratio: f32,
}
```

### 10.5 Tree state

Use monotonic node IDs and never reuse them during a tab lifetime.

```rust
pub struct NodeId(pub u32);

pub enum NodeKind {
    File,
    Directory,
    SymlinkFile,
    SymlinkDirectory,
}

pub enum DirLoadState {
    Unloaded,
    Loading,
    Loaded,
    Error(String),
}

pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub name: String,
    pub rel_path: PathBuf,
    pub kind: NodeKind,
    pub expanded: bool,
    pub depth: u16,
    pub dir_load: DirLoadState,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub git: GitStatus,
    pub is_hidden: bool,
    pub selected: bool,
    pub highlight_until: Option<Instant>,
    pub children: Vec<NodeId>,
}

pub struct VisibleRow {
    pub node_id: NodeId,
    pub depth: u16,
    pub is_match: bool,
    pub match_ranges: Vec<std::ops::Range<usize>>,
}

pub struct TreeState {
    pub root_abs: PathBuf,
    pub nodes: Vec<Option<Node>>,
    pub root_id: NodeId,
    pub path_to_id: HashMap<PathBuf, NodeId>,
    pub visible_rows: Vec<VisibleRow>,
    pub selected_row: usize,
    pub scroll_row: usize,
    pub multiselect: BTreeSet<NodeId>,
    pub sort_mode: SortMode,
    pub show_hidden: bool,
    pub respect_gitignore: bool,
    pub index_state: IndexState,
}
```

### 10.6 Search generations and preview generations

All async work must be generation-scoped.

```rust
pub struct PreviewGeneration(pub u64);
pub struct SearchGeneration(pub u64);
pub struct GitGeneration(pub u64);
```

When selection changes, increment preview generation.
When content search query changes, increment search generation.
When repo changes or git refresh is requested, increment git generation.

Drop results that do not match the current generation.

### 10.7 Git backend trait

```rust
pub trait GitBackend: Send + Sync {
    fn discover_repo(&self, root: &Path) -> Result<Option<RepoHandle>, GitError>;
    fn status_map(&self, repo: &RepoHandle) -> Result<HashMap<PathBuf, GitStatus>, GitError>;
    fn diff_for_path(&self, repo: &RepoHandle, rel_path: &Path, mode: DiffMode) -> Result<UnifiedDiff, GitError>;
    fn blame_for_path(&self, repo: &RepoHandle, rel_path: &Path) -> Result<BlameView, GitError>;
    fn history_for_path(&self, repo: &RepoHandle, rel_path: &Path, limit: usize) -> Result<Vec<CommitSummary>, GitError>;
    fn stage_path(&self, repo: &RepoHandle, rel_path: &Path) -> Result<(), GitError>;
    fn unstage_path(&self, repo: &RepoHandle, rel_path: &Path) -> Result<(), GitError>;
}
```

Implement `libgit2` first, but keep the trait from day one.

### 10.8 Bridge protocol contract

Rust and Python must share a stable protocol.

```rust
pub enum BridgeCommand {
    Ping,
    ListSessions { instance_id: String },
    SetRole { session_id: String, role: TargetRole },
    ClearRole { session_id: String },
    ResolveTargets { instance_id: String },
    SendText {
        instance_id: String,
        target: SendTarget,
        text: String,
        append_newline: bool,
    },
    GetSessionSnapshot { session_id: String },
}

pub enum BridgeResponse {
    Pong,
    SessionList(Vec<SessionSummary>),
    TargetsResolved(TargetResolution),
    SendOk { target_session_id: String },
    Error { message: String },
}
```

## 11. Tree, index, search, and watcher design

### 11.1 Loading strategy

The tree has two loading paths:

1. Lazy per-directory load  
   Used when the user expands a directory that is not loaded.

2. Background full index  
   Used to populate the search index and prefill tree metadata over time.

Both paths dedupe by `rel_path`.

### 11.2 Per-directory load

When a directory is expanded and `dir_load == Unloaded`:

- spawn a `DirLoadJob`
- run `ignore::WalkBuilder::new(abs_dir)`
- set `parents(true)`, keep standard filters enabled, and set `max_depth(Some(1))`
- disable following symlinks
- return immediate children only
- sort children before attaching to the node
- mark directory as `Loaded`

This gives correct ignore behavior while keeping local expansion fast.

### 11.3 Background full index

On tab open and after forced rescan:

- spawn a background index job from the tab root
- use `ignore::WalkBuilder::new(root).build_parallel()`
- stream results back in batches of 128 to 512 entries
- create nodes for paths not already present
- update metadata and search index for existing paths
- rebuild visible rows at controlled intervals, not on every single entry
- show status text such as `Indexing 12,341 paths...`

The background index is a convenience and speed layer, not a prerequisite for using the tree.

### 11.4 Visible row rebuild algorithm

Rebuild `visible_rows` only when one of these changes:

- expansion state
- loaded children
- sort mode
- hidden/gitignore toggles
- path filter results
- node deletion or rename

Rules:

- Flatten only once per state change.
- Render only the current viewport slice of `visible_rows`.
- Store search highlight ranges in `VisibleRow`, not in the renderer.

### 11.5 Path filter behavior

Path filter is always visible and always fast.

Implementation:

- maintain a path search index separate from the tree view
- use `nucleo` for ranking
- search candidates are root-relative display paths
- rebuild filtered virtual tree by:
  1. getting matching node IDs
  2. adding all ancestors
  3. flattening the union into `visible_rows`
- when the filter clears, restore normal visible rows and scroll selection to the previously selected real node if still present

### 11.6 Content search behavior

Content search is a separate overlay or command flow.

Rules:

- launching content search does not mutate the path filter
- the query and results belong to `ContentSearchState`
- results are shown in `SearchResults` mode in the right panel
- use `grep-searcher` + `grep-regex`
- group results by file
- include a few lines of context
- clicking a result:
  - selects the file in the tree
  - opens preview for that file
  - scrolls preview to the result line
  - leaves content search state intact

### 11.7 Watcher model

Watch the whole root recursively.

Important rule:

Do not apply display-level ignore patterns to the OS watcher registration.

Why:

- `.git` changes still matter for git refresh
- external file changes in ignored directories may still impact parent summaries
- app-level filtering is safer than under-watching

Watcher behavior:

- coalesce raw events into a short debounce window, default 125 ms
- if `need_rescan()` is true, schedule a low-priority full rescan
- if a path was created, deleted, or renamed, update or invalidate the local subtree
- if the event hits `.git/` or git metadata, schedule git refresh
- recently changed nodes get temporary highlight

### 11.8 Poll fallback

Use native watcher by default on macOS.

Fallback to polling when:

- native watcher setup fails
- the root is on a filesystem with poor event support
- the user enables poll fallback explicitly
- rescan warnings keep recurring

### 11.9 Symlink policy

- Do not follow symlink directories during recursive indexing.
- Show symlink directories in the tree as entries.
- In Info mode, show the link target.
- Opening or sending a symlink uses the symlink path, not the resolved path, unless the user explicitly requests resolution.

This avoids cycles and unexpected cross-repo walks.

## 12. Preview system design

### 12.1 Preview request lifecycle

On selection change:

1. increment preview generation
2. cancel stale preview work by generation mismatch
3. submit preview request to preview worker
4. preview worker reads and prepares a render-friendly payload
5. UI thread swaps in the payload only if the generation still matches

### 12.2 Preview thresholds

Default thresholds:

| Type | Default |
|---|---:|
| Binary sniff bytes | 8 KiB |
| Syntax-highlight max size | 1 MiB |
| Plain text preview max size | 4 MiB |
| Max rendered lines before truncation note | 20,000 |
| Hook timeout | 1.5 s |
| Hook output max bytes | 256 KiB |

Rules:

- Syntax-highlight only up to the syntax threshold.
- Plain text preview can go above highlight threshold up to raw threshold.
- Above raw threshold, show a summary and explicit `Load anyway` action.
- Never read an entire giant file on the UI thread.

### 12.3 File type routing

Routing order:

1. If directory -> directory preview
2. If hook exists and hooks are enabled -> hook preview
3. If extension is native Mermaid -> Mermaid preview
4. If binary sniff says binary -> hex preview
5. If extension is markdown and it contains Mermaid fences -> markdown preview with a selected Mermaid target
6. If extension is markdown -> markdown preview
7. If JSON / YAML -> pretty text preview
8. Else -> text preview with syntax highlight if eligible

### 12.4 Syntax highlight

Use `syntect`.

Implementation notes:

- initialize syntax and theme sets once
- store them in a shared immutable cache
- use `syntect` dump/load support to avoid repeated slow startup work
- line numbers are rendered by Grove, not embedded in the text payload
- wrap is a view option, not a data-loading option

### 12.5 Markdown

Do not write a markdown parser.

Use `pulldown-cmark` to parse CommonMark events, then render those events into ratatui text lines.

Support in v1:

- headings
- paragraphs
- emphasis and strong text
- bullet and numbered lists
- fenced code blocks
- inline code
- block quotes
- links as text plus URL
- tables and task lists if enabled through parser options

Do not attempt full HTML rendering in v1.

### 12.6 JSON and YAML

- Parse if cheap and safe.
- Pretty-print into text form.
- Syntax-highlight using the normal text preview path.
- If parse fails, fallback to plain text preview.

### 12.7 Binary preview

- Show hex + ASCII for the first chunk.
- Include file size and a simple detected type if known by extension or magic bytes.
- Never try to render binary as UTF-8 text by default.

### 12.8 Directory preview

Show:

- absolute path
- immediate child count
- file count and dir count if already known
- size summary if already cached
- selected directory notes such as git repo root or bookmark status

If recursive size is requested, compute it in a cancellable background task.

### 12.9 Diff and blame

Diff:

- unified diff only in v1
- selectable diff mode:
  - unstaged
  - staged
  - compare to HEAD
- diff preview shares the same scroll and line cursor model

Blame:

- line-based blame with commit, author, date, and code
- clicking a blame line can open a small commit summary dialog
- no heavy inline expansion in v1

### 12.10 Preview selection and snippet send

Support line-based selection in preview in v1 without mouse drag text selection.

Rules:

- click sets preview cursor line
- `Shift+Up` and `Shift+Down` expand a line range selection
- right-click on a selected range includes `Send selection` and `Copy selection`
- if no explicit line selection exists, `Send snippet` offers a small line-range dialog such as `start:end`

This gives snippet support without full terminal-text-selection complexity.

### 12.11 Preview hooks

Hooks are a v1.1 feature, but the interface should exist early.

Rules:

- hook command is argv-based, not a raw shell string
- hook runs with timeout
- hook output is capped
- nonzero exit falls back to the default preview
- hook output is treated as plain text in v1.1 unless ANSI parsing is deliberately added

## 13. Git subsystem

### 13.1 Backend choice

Implement `libgit2` first through the backend trait.

Why:

- local, direct, no shell-out dependency for common operations
- good enough for status, diff, blame, and short history

But the trait must stay in place because git backend flexibility is worth preserving.

### 13.2 Repo discovery

- discover repo from tab root
- cache repo handle per tab
- if no repo found, git features simply hide or show a no-repo message
- never treat missing git as fatal

### 13.3 Status refresh model

Refresh status:

- on tab open
- after debounced filesystem events
- after `.git` metadata changes
- after stage/unstage actions
- after explicit refresh command

Store a map from root-relative path to git status and apply it to nodes lazily.

### 13.4 Diff and history

- diff jobs are on-demand
- file history uses a small limit by default, like 10
- do not precompute history for the whole tree

### 13.5 Stage and unstage

Ship either late v1 or v1.1, depending on schedule quality.

If implemented in v1:

- file-only stage and unstage first
- no partial hunk staging in v1
- refresh status immediately after success

## 14. iTerm2 bridge and target model

This is the most important subsystem after the tree model.

### 14.1 Bridge install model

Official install path:

- `bridge/grove_bridge.py` is installed or symlinked into  
  `~/Library/Application Support/iTerm2/Scripts/AutoLaunch/`

Dev path:

- `scripts/run_bridge_dev.sh` launches the bridge in a normal shell for local testing

Do not make `python3 bridge.py &` the primary production install path.
Do not require a separate shell-integration install just to make Grove work; the runtime already emits the iTerm2 user variables it needs.

### 14.2 Why the bridge exists

The Rust app cannot control iTerm2 sessions directly. The Python daemon owns all iTerm2 API work, including:

- listing sessions
- reading session variables
- setting session variables
- resolving targets
- sending text to targets
- optional session snapshot reads for heuristics

### 14.3 Self-tagging and role tagging

Use iTerm2 user-defined variables for robust session identity.

On Grove startup:

- generate an `instance_id`
- emit iTerm2 `SetUserVar` control sequences from the Grove process to tag its own pane:
  - `user.groveRole = "grove"`
  - `user.groveInstance = "<instance_id>"`

Target sessions are tagged by the bridge with Python API calls:

- AI pane: `user.groveRole = "ai"`
- editor pane: `user.groveRole = "editor"`

This gives a stable way to know:
- which pane is Grove
- which pane is the AI target
- which pane is the editor target

### 14.4 Target resolution order

When Rust asks the bridge to send text to a target role:

1. find the sender session by `user.groveInstance`
2. look for a role-tagged target in the same iTerm2 tab
3. if none, look in the same window
4. if none, try heuristics based on session variables and visible content
5. if still none, return an error that triggers the session picker UI

### 14.5 Heuristic fallback

Heuristics should be last resort only.

Signals to use:

- `user.groveRole`
- session name
- `jobName`
- `commandLine`
- last few screen lines if needed

Heuristic hints:

- AI target candidates: `claude`, `claude-code`, `codex`, `aider`, similar job names
- editor target candidates: `nvim`, `vim`, `hx`, `helix`, `zed`, `code`

### 14.6 Session picker UI

Add a command palette action for:

- `Set AI target`
- `Set editor target`
- `Clear AI target`
- `Clear editor target`

Picker rows should show:

- session title
- role tag if any
- job name
- command line
- working directory if available
- window/tab location hint

When the user picks a session:

- bridge sets the role variable on that session
- clears the same role from the previous target in that window if needed
- returns success to Grove
- Grove shows a status note

### 14.7 Bridge protocol details

Socket:

- Unix domain socket at a UID-scoped temp path, for example  
  `$TMPDIR/grove-bridge-<uid>.sock`

Encoding:

- newline-delimited JSON

Rules:

- every request has a request ID
- every response echoes the request ID
- bridge must stay resilient if Rust reconnects
- Rust client uses reconnect with backoff

### 14.8 Sending text

Use `Session.async_send_text(text, suppress_broadcast=True)` on the resolved session.

Send types:

- path
- relative path
- `@ref`
- batch of paths
- file contents
- snippet
- diff
- directory tree
- editor command to editor target

### 14.9 Multiline transport

Multiline terminal input is not universally well-behaved across AI CLIs.

Design rule:

- path and `@ref` are the primary injection mechanisms
- raw contents and diff sends are supported, but conservative by default

Config:

```toml
[injection.ai]
multiline_transport = "typed" # typed | bracketed_paste
warn_line_count = 300
```

Behavior:

- default to `typed`
- if the user opts into `bracketed_paste`, wrap multiline text with paste bracket sequences
- never append a final newline unless explicitly requested
- always warn on large multiline sends

### 14.10 AI target versus editor target

These must be separate concepts.

AI target actions:

- send path
- send `@ref`
- send diff
- send snippet
- send contents

Editor target actions:

- open file
- open file at line
- open search result in editor
- open diff/blame line in editor

If no editor target exists, use local process launch if configured.

## 15. Editor integration and file operations

### 15.1 Editor integration modes

Support both:

1. `local_process`
2. `shell_target`

Config example:

```toml
[injection.editor]
mode = "local_process" # local_process | shell_target
command = "code"
args = ["-g", "{{path}}:{{line}}"]

# alternative shell-target config:
# mode = "shell_target"
# command = "nvim"
# args = ["+{{line}}", "{{path}}"]
```

Implementation rules:

- local process launch uses `std::process::Command`, not a shell
- shell-target mode builds a safely quoted command string and sends it to the editor target
- do not route editor opens to the AI target

### 15.2 File operation scope

Ship these operations:

- new file
- new folder
- rename
- duplicate
- move
- trash
- reveal in Finder
- open in default app

Drag and drop is deferred.

### 15.3 Safety rules for file operations

- rename and move must confirm overwrite if destination exists
- move across filesystems falls back to copy + verify + trash source only if needed
- trash uses OS trash semantics
- no permanent delete action in v1
- errors are status messages plus log entries, never silent failures

### 15.4 Trash

Use the `trash` crate for cross-platform trash behavior with macOS support.

Even though macOS is the target, using a crate here is simpler and safer than rolling custom Finder or AppleScript behavior.

## 16. Configuration and persistence

### 16.1 Paths

Use:

- config: `~/.config/grove/config.toml`
- state: `~/.config/grove/state.json`
- cache: `~/.cache/grove/`

Create directories on first run.

### 16.2 Config shape

Recommended initial config:

```toml
[general]
show_hidden = false
respect_gitignore = true
sort_by = "name"
theme = "dark"

[layout]
split_ratio = 0.40

[preview]
syntax_highlight = true
highlight_max_bytes = 1_048_576
raw_text_max_bytes = 4_194_304
binary_sniff_bytes = 8192
word_wrap = true
line_numbers = true
mermaid_command = ""
mermaid_render_timeout_ms = 5_000

[git]
show_status = true
refresh_debounce_ms = 200

[injection.ai]
append_newline = false
batch_separator = "newline"
multiline_transport = "typed"
warn_line_count = 300

[injection.editor]
mode = "local_process"
command = "code"
args = ["-g", "{{path}}:{{line}}"]

[watcher]
debounce_ms = 125
highlight_changes_ms = 5000
poll_fallback = true

[bookmarks]
pins = []
```

### 16.3 Persisted state

Persist only lightweight state:

- open tabs and active tab
- split ratio
- bookmark list
- recent roots
- last focused panel
- per-tab current mode
- a bounded set of expanded directories per tab
- last AI/editor target selection metadata if useful

Do not persist giant search indexes or full tree snapshots.

## 17. Performance implementation rules

These rules are mandatory.

### 17.1 UI draw rules

- draw only when state is dirty or a timer deadline is reached
- use `crossterm::event::poll(timeout)` with dynamic timeout
- no permanent frame loop
- status toasts and highlight fade are timer-driven and sparse

### 17.2 Channel rules

- every cross-thread channel must be bounded
- preview worker queue length should be 1 or 2
- content search queue length should be 1
- git queue length can be small, like 4
- if a new request supersedes an old one, drop the old one

### 17.3 Cache rules

Caches allowed:

- syntax/theme load cache
- small preview payload LRU
- per-tab git status map
- search index in memory
- optional stat metadata cache

Caches not allowed in v1:

- persistent repo database
- unbounded preview cache
- full file content cache across the repo

### 17.4 Memory rules

- keep node data compact
- use root-relative paths, not repeated absolute roots
- do not keep giant strings if they can be recomputed cheaply
- clear preview payloads when switching tabs if memory pressure matters

### 17.5 Benchmark discipline

If performance is bad:

1. benchmark
2. profile
3. fix the measured issue

Do not add random complexity without measurement.

## 18. Error handling and recovery

### 18.1 Panic handling

Install a panic hook that:

- disables raw mode
- leaves alternate screen
- restores mouse capture state
- prints a readable panic summary
- writes the full panic to a log

A terminal app that leaves raw mode behind is unacceptable.

### 18.2 Feature degradation

Subsystem failure behavior:

- bridge down -> injection disabled, UI still works
- git unavailable -> git modes hidden or show friendly message
- watcher failure -> offer poll fallback or manual refresh
- preview failure -> show plain text or summary fallback
- hook failure -> fallback to standard preview

### 18.3 Status messaging

Use short status messages with severity:

- success
- info
- warning
- error

Never use noisy modal dialogs unless the action is destructive or truly blocking.

### 18.4 Logging

Use `tracing` with:

- stderr logging in debug mode
- file logging in verbose mode to `~/.config/grove/grove.log`
- a separate `bridge.log` for the Python daemon

## 19. Testing and benchmark plan

### 19.1 Unit tests

Write unit tests for:

- tree node insertion and deletion
- visible row rebuilding
- path filtering
- config parsing and defaults
- bridge protocol serialization
- preview type routing
- markdown render mapping
- binary sniff
- git status mapping
- file operation safety checks

### 19.2 Integration tests

Write integration tests for:

- lazy directory loading
- background indexing
- watcher event coalescing
- rename/move/trash flows on temp dirs
- git repo fixture operations
- bridge client protocol against a fake daemon

### 19.3 UI snapshot tests

Use ratatui buffer snapshot tests for:

- tree rendering
- action bar rendering
- context menu rendering
- command palette
- search results view
- diff and blame layout

This will catch many UI regressions cheaply.

### 19.4 Manual test matrix

Manual tests must include:

- small repo
- medium repo
- very large repo
- git repo with many changes
- non-git directory
- symlink-heavy tree
- unicode filenames
- hidden files on and off
- `.gitignore` on and off
- bridge connected and disconnected
- AI pane only
- AI pane plus editor pane
- local process editor mode
- shell target editor mode

### 19.5 Benchmark fixtures

Create scripts to generate fixture repos:

- `small`: 2k entries
- `medium`: 20k entries
- `large`: 100k entries
- optional nested monorepo style fixture

Benchmark commands:

- startup and first frame
- path filter throughput
- preview load
- content search
- watcher churn after mass file writes

## 20. CI and quality gates

### 20.1 Required checks

For every merge:

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- UI snapshot tests
- selected benchmark smoke checks
- Python bridge lint or basic syntax check

### 20.2 macOS CI

Since macOS is the target, use macOS CI as the main signal.

Optional Linux `cargo check` is fine, but macOS is the real target.

### 20.3 Dependency discipline

- keep the dependency set small
- remove dead dependencies quickly
- pin versions in `Cargo.lock`
- do not upgrade large libraries mid-feature unless necessary

## 21. Recommended dependency set

Use current compatible stable versions at implementation time. Do not hardcode stale version guesses in the plan. Let Codex add and lock the latest compatible versions.

Core crates:

- `ratatui`
- `crossterm`
- `ignore`
- `notify`
- `git2`
- `nucleo`
- `grep-searcher`
- `grep-regex`
- `syntect`
- `pulldown-cmark`
- `trash`
- `serde`
- `serde_json`
- `toml`
- `clap`
- `crossbeam-channel`
- `tracing`
- `tracing-subscriber`
- `unicode-width`
- `mime_guess`
- `thiserror`
- `base64`
- `shell-escape`
- `dirs`
- `uuid`

Dev dependencies:

- `tempfile`
- `assert_fs`
- `insta`
- `pretty_assertions`

Recommended release profile:

```toml
[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
strip = "symbols"
panic = "abort"
```

Optional local build optimization for your own machine:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

Do not bake `target-cpu=native` into shared checked-in config.

## 22. Implementation phases

This is the delivery plan. Each phase must end with a green build and a manual smoke test.

### Phase 0: Contract freeze and bootstrap

Goal: create a stable scaffold so parallel work can proceed.

Tasks:

- create repo structure
- wire `Cargo.toml`
- create `main.rs`, `bootstrap.rs`, `app.rs`, `action.rs`, `event.rs`, `error.rs`, `config.rs`, `state.rs`
- add panic hook and terminal restore logic
- add raw mode / alternate screen enter/leave
- add base theme and root layout shell
- define frozen interfaces:
  - `Action`
  - `AppEvent`
  - `Config`
  - `PersistedState`
  - `BridgeCommand` / `BridgeResponse`
  - `GitBackend`
  - `PreviewGeneration`, `SearchGeneration`, `GitGeneration`

Done when:

- app launches to an empty shell UI
- `q` exits cleanly
- panic restores terminal
- tests compile
- frozen interfaces are documented

### Phase 1: Tree shell, tabs, and lazy root load

Goal: show a working project tree with basic navigation.

Tasks:

- implement `TabState`
- implement `TreeState` and node arena
- load root plus immediate children
- render tabs, path filter shell, bookmarks shell, tree, action bar, status bar
- implement selection, scrolling, expand/collapse
- implement visible row rebuild
- implement mouse click, scroll, and divider resize
- implement sort by name only first

Done when:

- opening a project shows the root tree quickly
- directories expand lazily
- scroll is smooth
- no recursive full-tree render path exists

### Phase 2: Background indexer and path filter

Goal: make navigation and filtering feel instant.

Tasks:

- implement background full indexer with batched updates
- integrate `nucleo` path search index
- add always-visible path filter input
- create filtered virtual tree with ancestors preserved
- rebuild visible rows on filter change
- add hidden and `.gitignore` toggles
- add sort modes beyond name

Done when:

- first frame is fast
- path filter remains fast while indexing continues
- clearing filter restores normal tree state

### Phase 3: Preview system core

Goal: make the right panel useful.

Tasks:

- preview request/response pipeline
- text preview with syntect
- markdown preview using `pulldown-cmark`
- JSON/YAML pretty preview
- binary hex preview
- directory preview
- preview cursor and line selection
- send snippet from selected lines
- preview scroll behavior

Done when:

- selecting files feels immediate
- stale preview jobs are dropped
- line selection works without mouse drag text selection

### Phase 4: iTerm2 bridge and target picker

Goal: make Grove an AI sidecar, not just a tree viewer.

Tasks:

- implement Python AutoLaunch daemon
- implement Rust Unix socket client
- implement self-tagging with `SetUserVar`
- implement role tagging for AI and editor targets
- implement session list and picker
- implement target resolution
- implement send path, relative path, `@ref`, batch send
- implement send contents, diff, snippet, and tree
- implement bridge status indicator and reconnect flow

Done when:

- Grove can reliably send a path to the selected AI pane
- editor open never lands in the AI pane by mistake
- reconnect after bridge restart works

### Phase 5: Git subsystem

Goal: make git context first-class.

Tasks:

- implement `GitBackend` with `libgit2`
- repo discovery
- status map refresh
- git status indicators in tree
- diff mode
- blame mode
- info mode with short history
- optional stage/unstage if quality permits

Done when:

- modified, staged, untracked, and conflicted files display correctly
- diff and blame work for selected files
- git absence degrades cleanly

### Phase 6: Content search, command palette, and context menu

Goal: make advanced actions discoverable and fast.

Tasks:

- content search overlay and worker
- search results mode
- command palette with action search
- right-click context menu
- action bar logic by selection context
- keyboard and mouse access for all major actions

Done when:

- content search is separate from path filter
- actions are easy to discover without memorizing shortcuts

### Phase 7: Bookmarks, file operations, and editor integration

Goal: make the tool practical every day.

Tasks:

- bookmark management
- bookmark opens or activates tabs
- create file and folder
- rename
- duplicate
- move
- trash
- reveal in Finder
- open in default app
- editor integration for local process and shell target

Done when:

- common non-destructive file workflows are solid
- trash semantics work
- editor open at line works from preview and search results

### Phase 8: Watcher hardening and polish

Goal: make it resilient under real repo churn.

Tasks:

- watcher debounce and coalescing
- `.git` event routing to git refresh
- `need_rescan()` handling
- poll fallback path
- recently changed highlight
- status toasts
- tune redraw and timers
- benchmark and fix obvious hotspots

Done when:

- mass file updates do not freeze the UI
- git refresh stays responsive
- idle CPU remains near zero

### Phase 10: Install docs and quality sweep

Goal: freeze the consumer-facing install and usage contract without widening runtime scope.

Tasks:

- install/setup docs for the current source-build path
- bridge AutoLaunch setup docs
- a consumer-facing user guide and keyboard shortcut reference
- an explicit optional-dependency contract for Nerd Font, `mmdc`, and `beautiful-mermaid`
- a first-release distro direction of GitHub Releases plus an installer script
- a final manual test checklist and copy sweep

Done when:

- a clean current-checkout install is documented
- optional dependencies and iTerm2-only behavior are explicit
- the user guide is sufficient without session history
- the repo is ready for first-release packaging docs

## 23. Parallel workstream plan

This section is designed for multiple Codex sessions or parallel agents.

### 23.1 Freeze these files first on main

Complete Phase 0 on main before parallelizing. Freeze:

- `src/action.rs`
- `src/event.rs`
- `src/config.rs`
- `src/state.rs`
- `src/bridge/protocol.rs`
- `src/git/backend.rs`
- `src/preview/model.rs`
- `src/search/mod.rs` request/response types
- `src/tree/model.rs` core public types

Do not let parallel agents change these contracts casually.

### 23.2 Recommended workstreams

#### Workstream A: Tree and indexing

Owns:

- `src/tree/*`
- `src/ui/tree.rs`
- `src/ui/path_filter.rs`

Depends on:

- frozen core contracts only

Deliverables:

- lazy tree
- visible rows
- path filter
- background indexer
- watcher integration hooks

#### Workstream B: Preview system

Owns:

- `src/preview/*`
- `src/ui/preview.rs`

Depends on:

- frozen preview contracts
- basic tree selection events

Deliverables:

- text preview
- markdown
- JSON/YAML
- binary and directory preview
- preview cursor and line selection

#### Workstream C: Bridge and targets

Owns:

- `bridge/*`
- `src/bridge/*`
- related target dialogs in UI

Depends on:

- frozen bridge protocol
- basic command dispatch

Deliverables:

- AutoLaunch daemon
- socket client
- tagging
- session picker
- send actions

#### Workstream D: Git and search

Owns:

- `src/git/*`
- `src/search/content.rs`
- search results view pieces

Depends on:

- tree root and preview mode hooks

Deliverables:

- git status
- diff
- blame
- file history
- content search

#### Workstream E: Tabs, bookmarks, file ops, editor integration

Owns:

- `src/actions/file_ops.rs`
- `src/actions/open.rs`
- `src/ui/tabs.rs`
- `src/ui/bookmarks.rs`
- related state wiring

Depends on:

- stable tab and action contracts
- basic tree operations

Deliverables:

- tabs
- bookmarks
- file ops
- editor open modes

### 23.3 Merge order

Recommended merge order:

1. Phase 0 main
2. Workstream A
3. Workstream B and Workstream C in parallel
4. Workstream D
5. Workstream E
6. integration and hardening branch
7. final docs and install sweep

### 23.4 Worktree suggestion

Example local worktree layout:

```bash
git worktree add ../grove-tree feat/tree-index
git worktree add ../grove-preview feat/preview
git worktree add ../grove-bridge feat/bridge
git worktree add ../grove-git feat/git-search
git worktree add ../grove-ops feat/tabs-fileops
```

## 24. Codex CLI operating instructions

Use this section if you want to hand the plan directly to Codex.

1. Read the whole document once.
2. Implement Phase 0 on main first.
3. Freeze contracts before parallelization.
4. Keep each phase or workstream mergeable.
5. Run format, clippy, and tests before each merge.
6. Do not change architecture decisions without updating this blueprint or adding a benchmark-backed note in the repo.
7. Prefer incremental vertical slices over giant rewrites.
8. If a feature threatens performance budgets, cut or defer the feature instead of masking the problem.
9. When unsure between scope and quality, choose quality.
10. Do not add PDF preview, drag-drop, or permanent delete to v1 unless this document is updated.

### 24.1 Session-by-session execution model

Suggested Codex session prompts:

- Session 1: "Implement Phase 0 from docs/implementation-blueprint.md. Keep contracts small and compile clean."
- Session 2: "Implement Workstream A against the frozen contracts. Do not touch bridge or preview modules."
- Session 3: "Implement Workstream C against the frozen bridge protocol. Do not change action names."
- Session 4: "Implement Workstream B for text-first preview only. No images, no PDFs."
- Session 5: "Implement Workstream D with git status, diff, blame, and content search."
- Session 6: "Implement Workstream E with tabs, bookmarks, file ops, and editor integration."
- Session 7: "Integrate all workstreams, fix conflicts, run benchmarks, and harden the event loop."

## 25. Final definition of done

Grove is done for v1 when all of the following are true:

- opens fast and stays responsive in real repos
- path filter feels instant
- preview is stable for common text formats
- diff and blame are useful
- AI target selection is reliable
- editor open is separate and reliable
- common file ops work safely
- trash semantics are in place
- the UI never leaves the terminal broken after crash or panic
- watcher churn does not spike idle CPU
- tests and benchmarks exist
- the tool feels small, direct, and daily-driver ready

## 26. Reference links

Official docs worth keeping nearby during implementation:

- iTerm2 Python API daemons: https://iterm2.com/python-api/tutorial/daemons.html
- iTerm2 session API: https://iterm2.com/python-api/session.html
- iTerm2 variables: https://iterm2.com/documentation-variables.html
- iTerm2 scripting fundamentals: https://iterm2.com/documentation-scripting-fundamentals.html
- iTerm2 proprietary escape codes: https://iterm2.com/documentation-escape-codes.html
- `ignore` crate docs: https://docs.rs/ignore/latest/ignore/
- `notify` crate docs: https://docs.rs/notify/latest/notify/
- `nucleo` docs: https://docs.rs/nucleo/latest/nucleo/
- `syntect` docs: https://docs.rs/syntect/latest/syntect/
- `pulldown-cmark` docs: https://docs.rs/pulldown-cmark/latest/pulldown_cmark/
- `trash` crate docs: https://docs.rs/trash/latest/trash/
- `git2` docs: https://docs.rs/git2/latest/git2/

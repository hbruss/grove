# User Guide

Grove is designed for a simple pane layout:

- one iTerm2 pane running Grove
- one AI pane
- optionally one editor pane

Install/setup details live in [[install]]. Runtime internals live in [[architecture/runtime]].

## First Launch

- `Tab` cycles focus between the tree, roots navigator, and preview.
- `/` jumps into the path filter.
- `q` quits Grove.
- Important: in the tree, `Right` is the open/edit key. `Enter` is mainly for overlays, search results, and roots focus.

## Roots Workflow

- `Tab` into `Roots`, then use `Up` / `Down` to move.
- `Enter` opens or activates the highlighted root.
- `Ctrl+T` promotes the selected directory into a root tab.
- `b` pins the selected directory root; if the current selection is not a directory target, Grove falls back to the active root.
- `Ctrl+R` opens `Add Root` from `$HOME`.
- Inside `Add Root`, explicit `.` and `..` rows are always present.
- Inside `Add Root`, `Left` goes to the parent, `Right` opens the highlighted directory, and `Enter` pins plus opens it.

## Tree Navigation

- `Up` / `Down` move the tree selection.
- `Left` collapses the selected expanded directory or moves to the parent row.
- `Right` expands the selected directory one level at a time.
- `/` opens the path filter without mutating expansion state.
- `Ctrl+H` toggles hidden files.
- Bare `Backspace` outside the filter is the same hidden-file toggle.
- `Ctrl+G` toggles `.gitignore` filtering.

## Multi-Select And AI Send

- `m` enters or exits explicit multi-select mode while tree focus is active.
- `Space` toggles the current non-root row into or out of the batch.
- `x` clears the batch.
- `Esc` exits multi-select mode but keeps the batch queued.
- `Ctrl+Y` sends the batch as a plain newline-separated list of root-relative paths.
- If the batch is empty, `Ctrl+Y` falls back to the currently selected path.
- `Ctrl+A` opens the AI target picker.

## Editor Workflow

- `Right` on a file opens it through the current-pane editor path.
- `Ctrl+E` opens the editor target picker.
- Committing the editor picker immediately opens the selected file through that target.
- The editor picker always includes `Current pane` as the first/default option.
- `o` opens the selected file through the system opener.

## Preview Workflow

- `Tab` into `Preview`.
- `Up` / `Down`, `PageUp` / `PageDown`, `Home`, and `End` scroll the rendered preview.
- `Shift+Up` / `Shift+Down` grows or shrinks a contiguous line selection.
- `c` copies the selected preview lines, or the current line when no explicit range exists.
- `Esc` clears the preview selection.

## Git Workflow

- `d` enters unstaged diff mode when the selected file has supported unstaged or untracked changes.
- `p` returns to normal preview mode.
- `s` stages the selected file.
- `u` unstages the selected file.
- Unsupported diff or stage targets stay in preview and surface a status message instead of replacing the panel with placeholder UI.

## Search, Commands, And File Ops

- `Ctrl+F` opens whole-repo content search.
- `Enter` submits the active content-search query, then activates the selected hit once results are ready.
- `Ctrl+P` opens the unified command surface.
- Many file and root operations live behind `Ctrl+P`: new file, new directory, copy relative path, copy absolute path, reveal in Finder, rename, duplicate, move, trash, pin/unpin root, close tab, and explicit diff/preview mode switches.

## Optional Preview Features

- Markdown, JSON, binary summaries, diff mode, and metadata previews are part of the core runtime.
- Mermaid preview can render native `.mmd` / `.mermaid` files and Markdown Mermaid fences when optional tooling is present.
- Static inline image preview supports local `.png`, `.jpg`, `.jpeg`, `.gif`, and `.webp` files in iTerm2.
- Missing optional tooling falls back to text or metadata instead of blocking the app.

## Keyboard Shortcut Reference

| Area | Key | Action |
| --- | --- | --- |
| Global | `q` | Quit Grove |
| Global | `Tab` | Cycle focus between tree, roots, and preview |
| Global | `/` | Focus the path filter |
| Visibility | `Ctrl+H` | Toggle hidden files |
| Visibility | `Backspace` | Hidden-file toggle when focus is not in the path filter |
| Visibility | `Ctrl+G` | Toggle `.gitignore` filtering |
| Tree | `Up` / `Down` | Move selection |
| Tree | `Left` | Collapse directory or move to the parent row |
| Tree | `Right` | Expand directory or open the selected file in the current pane |
| Roots | `Enter` | Open or activate the highlighted root |
| Roots | `Ctrl+T` | Promote the selected directory to a root tab |
| Roots | `Ctrl+R` | Open the `Add Root` picker |
| Roots | `b` | Pin the selected directory root or the active root |
| Multi-select | `m` | Enter or exit multi-select mode from tree focus |
| Multi-select | `Space` | Toggle the current non-root row into or out of the batch |
| Multi-select | `x` | Clear the current batch |
| Multi-select | `Esc` | Exit multi-select mode without clearing the batch |
| AI | `Ctrl+A` | Pick the AI target pane |
| AI | `Ctrl+Y` | Send the current path or the active multi-select batch |
| Editor | `Ctrl+E` | Pick the editor target and immediately open the selected file |
| File open | `o` | Open the selected file through the system opener |
| Search | `Ctrl+F` | Open content search |
| Commands | `Ctrl+P` | Open the unified command surface |
| Preview | `Up` / `Down` | Scroll the preview |
| Preview | `PageUp` / `PageDown` | Scroll the preview by page |
| Preview | `Home` / `End` | Jump to the top or bottom of the preview |
| Preview | `Shift+Up` / `Shift+Down` | Expand or shrink the preview line selection |
| Preview | `c` | Copy the selected preview lines |
| Preview | `Esc` | Clear the preview selection |
| Git | `d` | Enter diff mode |
| Git | `p` | Return to preview mode |
| Git | `s` | Stage the selected file |
| Git | `u` | Unstage the selected file |
| Overlay | `Up` / `Down` | Move inside pickers and lists |
| Overlay | `Left` / `Right` | Parent/enter navigation inside `Add Root` |
| Overlay | `Enter` | Commit the highlighted picker or command |
| Overlay | `Esc` | Cancel the active picker, prompt, or command surface |

## Troubleshooting

- `Ctrl+Y` sends one path instead of many
  - Return to tree focus, confirm the status bar shows a non-zero multi-select count, and make sure the rows were actually queued with `Space`.
- `Enter` did not open a file from the tree
  - Tree open/edit is on `Right`, not `Enter`.
- A root entry says it is missing
  - The pin points at a stale path. Re-pin the real directory.
- `Ctrl+E` opened in the wrong place
  - Re-open the editor target picker and pick `Current pane` or the correct editor pane explicitly.

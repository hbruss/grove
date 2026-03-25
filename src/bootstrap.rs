use std::fmt::Display;
use std::fs;
use std::io::{Read, Stdout, Write, stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use crossterm::cursor::{MoveTo, RestorePosition, SavePosition, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

use crate::action::Action;
use crate::app::App;
use crate::bridge::protocol::{
    BridgeCommand, BridgeResponse, SendTarget, SessionSummary, TargetRole,
};
use crate::error::{GroveError, Result};
use crate::git::backend::{GitBackend, GitStatus, LibgitBackend};
use crate::state::{
    ConfirmDialogIntent, ConfirmDialogState, ContextMode, DialogState, Focus, PromptDialogIntent,
    PromptDialogState, StatusSeverity, TargetPickerSelection,
};
use crate::tree::model::{Node, NodeKind};
use crate::watcher::{WatcherRuntime, WatcherService};

pub fn install_panic_hook() {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        restore_terminal();
        previous_hook(panic_info);
    }));
}

pub fn run() -> Result<()> {
    let mut terminal_session = TerminalSession::enter()?;
    let config_path = crate::storage::config_path()?;
    let config = crate::config::Config::load_from_path(&config_path)?;
    let mut app = App::new_with_config(config, config_path);
    let hooks = RuntimeActionHooks {
        open_in_editor: production_open_in_editor,
        open_externally: production_open_externally,
        reveal_in_file_manager: production_reveal_in_file_manager,
        initialize_bridge: production_initialize_bridge,
        list_sessions: production_list_sessions,
        send_text: production_send_text,
        set_role: production_set_role,
    };
    let mut watcher = WatcherRuntime::new_notify(app.config.watcher.clone())?;
    initialize_runtime_with_hooks_and_watcher(
        &mut terminal_session.terminal,
        &mut app,
        &hooks,
        &mut watcher,
    )?;
    run_shell_loop_with_events_and_watcher(
        &mut terminal_session.terminal,
        &mut app,
        hooks,
        &mut watcher,
    )
}

pub fn run_with_terminal_and_reader<B: Backend, R: Read>(
    terminal: &mut Terminal<B>,
    reader: &mut R,
    app: &mut App,
) -> Result<()>
where
    B::Error: Display,
{
    let hooks = RuntimeActionHooks::noops();
    let mut watcher = WatcherRuntime::new(app.config.watcher.clone(), NoopWatcherService);
    initialize_runtime_with_hooks_and_watcher(terminal, app, &hooks, &mut watcher)?;
    run_shell_loop_with_reader_and_watcher(terminal, reader, app, hooks, &mut watcher)
}

pub fn run_with_terminal_and_reader_and_hooks<B: Backend, R: Read>(
    terminal: &mut Terminal<B>,
    reader: &mut R,
    app: &mut App,
    hooks: RuntimeActionHooks<B>,
) -> Result<()>
where
    B::Error: Display,
{
    let mut watcher = WatcherRuntime::new(app.config.watcher.clone(), NoopWatcherService);
    initialize_runtime_with_hooks_and_watcher(terminal, app, &hooks, &mut watcher)?;
    run_shell_loop_with_reader_and_watcher(terminal, reader, app, hooks, &mut watcher)
}

pub fn run_with_terminal_and_reader_and_watcher<B, R, W>(
    terminal: &mut Terminal<B>,
    reader: &mut R,
    app: &mut App,
    watcher: &mut W,
) -> Result<()>
where
    B: Backend,
    B::Error: Display,
    R: Read,
    W: WatcherService,
{
    let hooks = RuntimeActionHooks::noops();
    initialize_runtime_with_hooks_and_watcher(terminal, app, &hooks, watcher)?;
    run_shell_loop_with_reader_and_watcher(terminal, reader, app, hooks, watcher)
}

fn initialize_runtime_with_hooks_and_watcher<B: Backend, W: WatcherService>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    hooks: &RuntimeActionHooks<B>,
    watcher: &mut W,
) -> Result<()>
where
    B::Error: Display,
{
    (hooks.initialize_bridge)(app)?;
    sync_watcher_roots(app, watcher)?;
    render_shell_once(terminal, app)
}

pub fn render_shell_once<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()>
where
    B::Error: Display,
{
    let git_backend = LibgitBackend;
    render_shell_once_with_git_backend(terminal, app, &git_backend)
}

fn render_shell_once_with_git_backend<B: Backend, G: GitBackend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    git_backend: &G,
) -> Result<()>
where
    B::Error: Display,
{
    let render_started_at = Instant::now();
    let _ = app.poll_active_tab_path_index()?;
    let _ = app.poll_active_tab_content_search()?;
    let _ = app.poll_active_tab_image_render()?;
    let _ = app.poll_active_tab_mermaid_render()?;
    let git_refresh_needed = should_refresh_git_state(app);
    let git_refresh = if git_refresh_needed {
        app.refresh_active_git_state_with_backend(git_backend)
    } else {
        Ok(false)
    };
    let git_refresh_status = if git_refresh_needed {
        match &git_refresh {
            Ok(true) => "changed",
            Ok(false) => "unchanged",
            Err(_) => "error",
        }
    } else {
        "cached"
    };
    apply_git_refresh_result(app, git_refresh);
    let _ = app.refresh_active_preview();
    refresh_active_preview_render_cache(terminal, app)?;
    sync_selected_row_visibility(terminal, app)?;
    sync_active_preview_scroll(terminal, app)?;
    let current_overlay = current_iterm_preview_overlay(terminal, app)?;
    let (overlay_to_clear, next_overlay) =
        reconcile_iterm_preview_overlay(app.last_preview_overlay, current_overlay);
    clear_iterm_preview_overlay(overlay_to_clear)?;
    terminal
        .draw(|frame| crate::ui::render(frame, app))
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    render_iterm_preview_overlay(current_overlay, app)?;
    app.last_preview_overlay = next_overlay;
    let path_index_status = app
        .tabs
        .get(app.active_tab)
        .map(|tab| match &tab.path_index.status {
            crate::app::PathIndexStatus::Idle => "idle".to_string(),
            crate::app::PathIndexStatus::Building { indexed_paths } => {
                format!("building(indexed_paths={indexed_paths})")
            }
            crate::app::PathIndexStatus::Ready => "ready".to_string(),
            crate::app::PathIndexStatus::Error(message) => format!("error({message})"),
        })
        .unwrap_or_else(|| "missing-tab".to_string());
    crate::debug_log::log(&format!(
        "component=render git_refresh={git_refresh_status} path_index_status={path_index_status} focus={:?} duration_ms={}",
        app.focus,
        render_started_at.elapsed().as_millis()
    ));
    Ok(())
}

fn should_refresh_git_state(app: &App) -> bool {
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return false;
    };

    !tab.git.initialized || tab.git.needs_refresh
}

fn apply_git_refresh_result(app: &mut App, result: Result<bool>) {
    match result {
        Ok(_) => {
            if app.status.message.starts_with("git refresh failed: ") {
                app.status.severity = StatusSeverity::Info;
                app.status.message.clear();
            }
        }
        Err(err) => {
            app.status.severity = StatusSeverity::Error;
            app.status.message = format!("git refresh failed: {err}");
        }
    }
}

pub fn run_shell_with_reader<R: Read>(reader: &mut R) -> Result<()> {
    let mut buffered = BufferedInputReader::new(reader);
    loop {
        let Some(input) = read_runtime_input(&mut buffered)? else {
            return Ok(());
        };
        if matches!(input, RuntimeInput::Character('q')) {
            return Ok(());
        }
    }
}

fn run_shell_loop_with_reader_and_watcher<B, R, W>(
    terminal: &mut Terminal<B>,
    reader: &mut R,
    app: &mut App,
    hooks: RuntimeActionHooks<B>,
    watcher: &mut W,
) -> Result<()>
where
    B: Backend,
    B::Error: Display,
    R: Read,
    W: WatcherService,
{
    let mut buffered = BufferedInputReader::new(reader);
    loop {
        poll_runtime_background_work(terminal, app, watcher)?;
        let Some(input) = read_runtime_input(&mut buffered)? else {
            return Ok(());
        };

        match handle_runtime_input(terminal, app, input, &hooks)? {
            RuntimeControlFlow::Exit => return Ok(()),
            RuntimeControlFlow::Continue => {}
        }
    }
}

fn run_shell_loop_with_events_and_watcher<W: WatcherService>(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    hooks: RuntimeActionHooks<CrosstermBackend<Stdout>>,
    watcher: &mut W,
) -> Result<()> {
    loop {
        poll_runtime_background_work(terminal, app, watcher)?;

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }

        let event = event::read()?;
        if let Event::Resize(_, _) = event {
            render_shell_once(terminal, app)?;
            continue;
        }

        let Some(input) = runtime_input_from_event(event) else {
            continue;
        };

        match handle_runtime_input(terminal, app, input, &hooks)? {
            RuntimeControlFlow::Exit => return Ok(()),
            RuntimeControlFlow::Continue => {}
        }
    }
}

fn poll_runtime_background_work<B, W>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    watcher: &mut W,
) -> Result<()>
where
    B: Backend,
    B::Error: Display,
    W: WatcherService,
{
    let mut changed = false;
    changed |= app.poll_active_tab_path_index()?;
    changed |= app.poll_active_tab_content_search()?;
    changed |= app.poll_active_tab_image_render()?;
    changed |= app.poll_active_tab_mermaid_render()?;
    changed |= sync_watcher_roots(app, watcher)?;
    let watcher_changed = poll_watcher_refresh_plans(app, watcher)?;
    changed |= watcher_changed;
    if watcher_changed {
        changed |= sync_watcher_roots(app, watcher)?;
    }
    if changed {
        render_shell_once(terminal, app)?;
    }
    Ok(())
}

fn sync_watcher_roots<W: WatcherService>(app: &App, watcher: &mut W) -> Result<bool> {
    let roots = collect_open_root_paths(app);
    watcher.reconcile_open_roots(&roots)
}

fn poll_watcher_refresh_plans<W: WatcherService>(app: &mut App, watcher: &mut W) -> Result<bool> {
    let mut changed = false;
    for plan in watcher.poll_refresh_plans()? {
        changed |= app.apply_watcher_refresh_plan(&plan)?;
    }
    Ok(changed)
}

fn collect_open_root_paths(app: &App) -> Vec<PathBuf> {
    app.tabs.iter().map(|tab| tab.root.clone()).collect()
}

#[derive(Default)]
struct NoopWatcherService;

impl WatcherService for NoopWatcherService {
    fn reconcile_open_roots(&mut self, _roots: &[PathBuf]) -> Result<bool> {
        Ok(false)
    }

    fn poll_refresh_plans(&mut self) -> Result<Vec<crate::watcher::RefreshPlan>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum RuntimeControlFlow {
    Continue,
    Exit,
}

pub type RuntimeActionHandler<B> = fn(&mut Terminal<B>, &mut App) -> Result<bool>;
pub type RuntimeBridgeInitializeHandler = fn(&mut App) -> Result<()>;
pub type RuntimeListSessionsHandler = fn(&mut App) -> Result<Vec<SessionSummary>>;
pub type RuntimeSendTextHandler = fn(&mut App, SendTarget, String, bool) -> Result<BridgeResponse>;
pub type RuntimeSetRoleHandler = fn(&mut App, String, TargetRole) -> Result<BridgeResponse>;

#[derive(Clone, Copy)]
pub struct RuntimeActionHooks<B: Backend> {
    pub open_in_editor: RuntimeActionHandler<B>,
    pub open_externally: RuntimeActionHandler<B>,
    pub reveal_in_file_manager: RuntimeActionHandler<B>,
    pub initialize_bridge: RuntimeBridgeInitializeHandler,
    pub list_sessions: RuntimeListSessionsHandler,
    pub send_text: RuntimeSendTextHandler,
    pub set_role: RuntimeSetRoleHandler,
}

impl<B: Backend> RuntimeActionHooks<B>
where
    B::Error: Display,
{
    fn noops() -> Self {
        Self {
            open_in_editor: noop_runtime_action::<B>,
            open_externally: noop_runtime_action::<B>,
            reveal_in_file_manager: noop_runtime_action::<B>,
            initialize_bridge: noop_bridge_initialize,
            list_sessions: noop_list_sessions,
            send_text: noop_send_text,
            set_role: noop_set_role,
        }
    }
}

fn handle_runtime_input<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    input: RuntimeInput,
    hooks: &RuntimeActionHooks<B>,
) -> Result<RuntimeControlFlow>
where
    B::Error: Display,
{
    if app.focus == Focus::Dialog {
        return handle_dialog_input(terminal, app, input, hooks);
    }
    if app.focus == Focus::CommandPalette {
        return handle_command_palette_input(terminal, app, input, hooks);
    }
    if app.focus == Focus::ContentSearch {
        return handle_content_search_input(terminal, app, input);
    }

    match input {
        RuntimeInput::Character('q') if app.focus != Focus::PathFilter => {
            return Ok(RuntimeControlFlow::Exit);
        }
        RuntimeInput::MoveUp => {
            let changed = if app.focus == Focus::Preview {
                scroll_active_preview_up(terminal, app)?
            } else if app.focus == Focus::Roots {
                app.move_root_selection(-1)
            } else {
                app.focus == Focus::Tree && move_selection_up(app)
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::MoveDown => {
            let changed = if app.focus == Focus::Preview {
                scroll_active_preview_down(terminal, app)?
            } else if app.focus == Focus::Roots {
                app.move_root_selection(1)
            } else {
                app.focus == Focus::Tree && move_selection_down(app)
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::ExtendPreviewSelectionUp => {
            if app.focus == Focus::Preview && extend_active_preview_selection(terminal, app, -1)? {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::ExtendPreviewSelectionDown => {
            if app.focus == Focus::Preview && extend_active_preview_selection(terminal, app, 1)? {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::MoveLeft => {
            let changed = if app.focus == Focus::Tree && path_filter_query_is_empty(app) {
                move_selection_left(app)
            } else {
                false
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::MoveRight => {
            if app.focus == Focus::Tree && path_filter_query_is_empty(app) {
                let changed = if move_selection_right(app) {
                    true
                } else if selected_node_is_file(app) {
                    (hooks.open_in_editor)(terminal, app)?
                } else {
                    false
                };
                if changed {
                    render_shell_once(terminal, app)?;
                }
            }
        }
        RuntimeInput::SetAiTarget => {
            if open_target_picker(app, TargetRole::Ai, hooks)? {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::SetEditorTarget => {
            if open_editor_target_picker_or_warn(app, hooks)? {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::SetPreviewMode => {
            let changed = if app.focus == Focus::PathFilter {
                app.append_path_filter_char('p')?
            } else if app.focus == Focus::ContentSearch {
                app.append_active_content_search_char('p')
            } else {
                app.set_active_context_mode(ContextMode::Preview)
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::SetDiffMode => {
            let changed = if app.focus == Focus::PathFilter {
                app.append_path_filter_char('d')?
            } else if app.focus == Focus::ContentSearch {
                app.append_active_content_search_char('d')
            } else {
                app.activate_diff_mode_if_available()
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::StageSelectedPath => {
            let changed = if app.focus == Focus::PathFilter {
                app.append_path_filter_char('s')?
            } else if app.focus == Focus::ContentSearch {
                app.append_active_content_search_char('s')
            } else {
                stage_selected_path(app)?
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::UnstageSelectedPath => {
            let changed = if app.focus == Focus::PathFilter {
                app.append_path_filter_char('u')?
            } else if app.focus == Focus::ContentSearch {
                app.append_active_content_search_char('u')
            } else {
                unstage_selected_path(app)?
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::SendRelativePath => {
            if app.focus != Focus::PathFilter && send_selected_relative_path(app, hooks)? {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::WheelUp { column, row } => {
            let changed = if mouse_targets_preview(terminal, app, column, row)? {
                scroll_active_preview_up(terminal, app)?
            } else if mouse_targets_tree(terminal, app, column, row)? {
                move_selection_up(app)
            } else {
                false
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::WheelDown { column, row } => {
            let changed = if mouse_targets_preview(terminal, app, column, row)? {
                scroll_active_preview_down(terminal, app)?
            } else if mouse_targets_tree(terminal, app, column, row)? {
                move_selection_down(app)
            } else {
                false
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::PreviewClick { column, row } => {
            if set_active_preview_cursor_from_click(terminal, app, column, row)? {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::FocusNextPanel => {
            if app.focus_next_panel() {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::FocusPathFilter => {
            if app.focus == Focus::PathFilter {
                if app.append_path_filter_char('/')? {
                    render_shell_once(terminal, app)?;
                }
            } else if app.focus == Focus::ContentSearch {
                if app.append_active_content_search_char('/') {
                    render_shell_once(terminal, app)?;
                }
            } else if app.focus_path_filter() {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::OpenContentSearch => {
            if app.open_active_content_search() {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::OpenCommandPalette => {
            if app.open_unified_command_surface() {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::OpenRootPicker => {
            if open_root_picker(app)? {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::OpenSelectedDirectoryAsRootTab => {
            if open_selected_directory_as_root_tab(app)? {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::Character(ch) => {
            if app.focus == Focus::PathFilter {
                if app.append_path_filter_char(ch)? {
                    render_shell_once(terminal, app)?;
                }
            } else if app.focus == Focus::ContentSearch {
                if app.append_active_content_search_char(ch) {
                    render_shell_once(terminal, app)?;
                }
            } else if app.focus == Focus::Tree && ch == 'm' {
                if app.toggle_active_multi_select_mode() {
                    render_shell_once(terminal, app)?;
                }
            } else if app.focus == Focus::Tree && ch == ' ' {
                if app.active_multi_select_mode()
                    && app.toggle_selected_path_in_active_multi_select()
                {
                    render_shell_once(terminal, app)?;
                }
            } else if app.focus == Focus::Tree && ch == 'x' {
                if app.clear_active_multi_select() {
                    render_shell_once(terminal, app)?;
                }
            } else if app.focus == Focus::Preview && ch == 'c' {
                if copy_active_preview_selection(terminal, app)? {
                    render_shell_once(terminal, app)?;
                }
            } else if ch == 'v' {
                if app.toggle_active_preview_visibility() {
                    render_shell_once(terminal, app)?;
                }
            } else if ch == 'b' {
                if toggle_bookmark_target(app)? {
                    render_shell_once(terminal, app)?;
                }
            } else if ch == 'o'
                && selected_node_is_file(app)
                && (hooks.open_externally)(terminal, app)?
            {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::ToggleHidden => {
            let changed = if app.focus == Focus::PathFilter {
                app.backspace_path_filter()?
            } else {
                let changed = app.toggle_show_hidden()?;
                if changed {
                    app.save_config()?;
                }
                changed
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::ToggleGitignore => {
            let changed = app.toggle_respect_gitignore()?;
            if changed {
                app.save_config()?;
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::Backspace => {
            let changed = if app.focus == Focus::PathFilter {
                app.backspace_path_filter()?
            } else if app.focus == Focus::ContentSearch {
                app.backspace_active_content_search()
            } else {
                let changed = app.toggle_show_hidden()?;
                if changed {
                    app.save_config()?;
                }
                changed
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::Escape => {
            let changed = if app.focus == Focus::Tree {
                app.exit_active_multi_select_mode()
            } else if app.focus == Focus::Preview {
                app.clear_active_preview_selection()
            } else if app.focus == Focus::PathFilter {
                if path_filter_query_is_empty(app) {
                    app.blur_path_filter()
                } else {
                    app.clear_active_path_filter_and_keep_focus()?
                }
            } else if app.focus == Focus::ContentSearch {
                app.close_active_content_search()
            } else {
                false
            };
            if changed {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::PageUp => {
            if app.focus == Focus::Preview && scroll_active_preview_page_up(terminal, app)? {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::PageDown => {
            if app.focus == Focus::Preview && scroll_active_preview_page_down(terminal, app)? {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::Home => {
            if app.focus == Focus::Preview && app.scroll_active_preview_home() {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::End => {
            if app.focus == Focus::Preview && scroll_active_preview_end(terminal, app)? {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::Enter => {
            if app.focus == Focus::ContentSearch {
                let changed = if app.tabs[app.active_tab].mode == ContextMode::SearchResults
                    && app.tabs[app.active_tab]
                        .content_search
                        .selected_hit_index
                        .is_some()
                    && matches!(
                        app.tabs[app.active_tab].content_search.status,
                        crate::app::ContentSearchStatus::Ready
                    ) {
                    app.activate_selected_content_search_hit()
                } else {
                    app.submit_active_content_search()?
                };
                if changed {
                    render_shell_once(terminal, app)?;
                }
            } else if app.focus == Focus::Roots && activate_selected_root_or_warn(app) {
                render_shell_once(terminal, app)?;
            }
        }
        RuntimeInput::Ignore => {}
    }
    Ok(RuntimeControlFlow::Continue)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum RuntimeInput {
    MoveUp,
    MoveDown,
    ExtendPreviewSelectionUp,
    ExtendPreviewSelectionDown,
    MoveLeft,
    MoveRight,
    SetAiTarget,
    SetEditorTarget,
    SetPreviewMode,
    SetDiffMode,
    StageSelectedPath,
    UnstageSelectedPath,
    SendRelativePath,
    WheelUp { column: u16, row: u16 },
    WheelDown { column: u16, row: u16 },
    PreviewClick { column: u16, row: u16 },
    FocusNextPanel,
    FocusPathFilter,
    OpenContentSearch,
    OpenCommandPalette,
    OpenRootPicker,
    OpenSelectedDirectoryAsRootTab,
    ToggleHidden,
    ToggleGitignore,
    Character(char),
    Enter,
    Backspace,
    Escape,
    PageUp,
    PageDown,
    Home,
    End,
    Ignore,
}

fn read_runtime_input<R: Read>(
    reader: &mut BufferedInputReader<'_, R>,
) -> Result<Option<RuntimeInput>> {
    let Some(first) = read_one_byte(reader)? else {
        return Ok(None);
    };

    match first {
        0x09 => Ok(Some(RuntimeInput::FocusNextPanel)),
        0x01 => Ok(Some(RuntimeInput::SetAiTarget)),
        0x05 => Ok(Some(RuntimeInput::SetEditorTarget)),
        0x06 => Ok(Some(RuntimeInput::OpenContentSearch)),
        0x10 => Ok(Some(RuntimeInput::OpenCommandPalette)),
        0x12 => Ok(Some(RuntimeInput::OpenRootPicker)),
        0x14 => Ok(Some(RuntimeInput::OpenSelectedDirectoryAsRootTab)),
        b'/' => Ok(Some(RuntimeInput::FocusPathFilter)),
        b'd' => Ok(Some(RuntimeInput::SetDiffMode)),
        b'p' => Ok(Some(RuntimeInput::SetPreviewMode)),
        b's' => Ok(Some(RuntimeInput::StageSelectedPath)),
        b'u' => Ok(Some(RuntimeInput::UnstageSelectedPath)),
        0x07 => Ok(Some(RuntimeInput::ToggleGitignore)),
        0x08 => Ok(Some(RuntimeInput::ToggleHidden)),
        0x0a => Ok(Some(RuntimeInput::Enter)),
        0x0d => Ok(Some(RuntimeInput::Enter)),
        0x19 => Ok(Some(RuntimeInput::SendRelativePath)),
        0x7f => Ok(Some(RuntimeInput::Backspace)),
        0x1b => parse_escape_sequence(reader),
        byte if byte.is_ascii_graphic() || byte == b' ' => {
            Ok(Some(RuntimeInput::Character(byte as char)))
        }
        _ => Ok(Some(RuntimeInput::Ignore)),
    }
}

fn parse_escape_sequence<R: Read>(
    reader: &mut BufferedInputReader<'_, R>,
) -> Result<Option<RuntimeInput>> {
    let Some(second) = read_one_byte(reader)? else {
        return Ok(Some(RuntimeInput::Escape));
    };
    if second == b'O' {
        let Some(third) = read_one_byte(reader)? else {
            return Ok(Some(RuntimeInput::Escape));
        };
        let input = match third {
            b'F' => RuntimeInput::End,
            b'H' => RuntimeInput::Home,
            _ => RuntimeInput::Escape,
        };
        return Ok(Some(input));
    }
    if second != b'[' {
        reader.unread_byte(second);
        return Ok(Some(RuntimeInput::Escape));
    }

    let Some(third) = read_one_byte(reader)? else {
        return Ok(Some(RuntimeInput::Escape));
    };
    let input = match third {
        b'A' => RuntimeInput::MoveUp,
        b'B' => RuntimeInput::MoveDown,
        b'C' => RuntimeInput::MoveRight,
        b'D' => RuntimeInput::MoveLeft,
        b'F' => RuntimeInput::End,
        b'H' => RuntimeInput::Home,
        b'1' => match read_one_byte(reader)? {
            Some(b'~') => RuntimeInput::Home,
            Some(other) => {
                reader.unread_byte(other);
                RuntimeInput::Escape
            }
            None => RuntimeInput::Escape,
        },
        b'4' => match read_one_byte(reader)? {
            Some(b'~') => RuntimeInput::End,
            Some(other) => {
                reader.unread_byte(other);
                RuntimeInput::Escape
            }
            None => RuntimeInput::Escape,
        },
        b'5' => match read_one_byte(reader)? {
            Some(b'~') => RuntimeInput::PageUp,
            Some(other) => {
                reader.unread_byte(other);
                RuntimeInput::Escape
            }
            None => RuntimeInput::Escape,
        },
        b'6' => match read_one_byte(reader)? {
            Some(b'~') => RuntimeInput::PageDown,
            Some(other) => {
                reader.unread_byte(other);
                RuntimeInput::Escape
            }
            None => RuntimeInput::Escape,
        },
        _ => RuntimeInput::Escape,
    };
    Ok(Some(input))
}

fn runtime_input_from_event(event: Event) -> Option<RuntimeInput> {
    match event {
        Event::Key(key_event) => runtime_input_from_key_event(key_event),
        Event::Mouse(mouse_event) => runtime_input_from_mouse_event(mouse_event),
        Event::Resize(_, _) => Some(RuntimeInput::Ignore),
        Event::FocusGained | Event::FocusLost | Event::Paste(_) => None,
    }
}

fn runtime_input_from_key_event(key_event: KeyEvent) -> Option<RuntimeInput> {
    if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }

    match (key_event.code, key_event.modifiers) {
        (KeyCode::Tab, _) => Some(RuntimeInput::FocusNextPanel),
        (KeyCode::Char('a'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(RuntimeInput::SetAiTarget)
        }
        (KeyCode::Char('e'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(RuntimeInput::SetEditorTarget)
        }
        (KeyCode::Char('f'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(RuntimeInput::OpenContentSearch)
        }
        (KeyCode::Char('t'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(RuntimeInput::OpenSelectedDirectoryAsRootTab)
        }
        (KeyCode::Char('r'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(RuntimeInput::OpenRootPicker)
        }
        (KeyCode::Char('p'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(RuntimeInput::OpenCommandPalette)
        }
        (KeyCode::Char('d'), modifiers)
            if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
        {
            Some(RuntimeInput::SetDiffMode)
        }
        (KeyCode::Char('p'), modifiers)
            if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
        {
            Some(RuntimeInput::SetPreviewMode)
        }
        (KeyCode::Char('s'), modifiers)
            if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
        {
            Some(RuntimeInput::StageSelectedPath)
        }
        (KeyCode::Char('u'), modifiers)
            if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
        {
            Some(RuntimeInput::UnstageSelectedPath)
        }
        (KeyCode::Char('/'), _) => Some(RuntimeInput::FocusPathFilter),
        (KeyCode::Char('g'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(RuntimeInput::ToggleGitignore)
        }
        (KeyCode::Char('h'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(RuntimeInput::ToggleHidden)
        }
        (KeyCode::Char('y'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(RuntimeInput::SendRelativePath)
        }
        (KeyCode::Backspace, modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(RuntimeInput::ToggleHidden)
        }
        (KeyCode::Backspace, _) => Some(RuntimeInput::Backspace),
        (KeyCode::Esc, _) => Some(RuntimeInput::Escape),
        (KeyCode::Up, modifiers) if modifiers == KeyModifiers::SHIFT => {
            Some(RuntimeInput::ExtendPreviewSelectionUp)
        }
        (KeyCode::Down, modifiers) if modifiers == KeyModifiers::SHIFT => {
            Some(RuntimeInput::ExtendPreviewSelectionDown)
        }
        (KeyCode::Up, _) => Some(RuntimeInput::MoveUp),
        (KeyCode::Down, _) => Some(RuntimeInput::MoveDown),
        (KeyCode::Left, _) => Some(RuntimeInput::MoveLeft),
        (KeyCode::Right, _) => Some(RuntimeInput::MoveRight),
        (KeyCode::PageUp, _) => Some(RuntimeInput::PageUp),
        (KeyCode::PageDown, _) => Some(RuntimeInput::PageDown),
        (KeyCode::Home, _) => Some(RuntimeInput::Home),
        (KeyCode::End, _) => Some(RuntimeInput::End),
        (KeyCode::Enter, _) => Some(RuntimeInput::Enter),
        (KeyCode::Char(ch), modifiers)
            if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
        {
            Some(RuntimeInput::Character(ch))
        }
        _ => None,
    }
}

fn runtime_input_from_mouse_event(mouse_event: MouseEvent) -> Option<RuntimeInput> {
    match mouse_event.kind {
        MouseEventKind::ScrollUp => Some(RuntimeInput::WheelUp {
            column: mouse_event.column,
            row: mouse_event.row,
        }),
        MouseEventKind::ScrollDown => Some(RuntimeInput::WheelDown {
            column: mouse_event.column,
            row: mouse_event.row,
        }),
        MouseEventKind::Down(MouseButton::Left) => Some(RuntimeInput::PreviewClick {
            column: mouse_event.column,
            row: mouse_event.row,
        }),
        _ => None,
    }
}

fn read_one_byte<R: Read>(reader: &mut BufferedInputReader<'_, R>) -> Result<Option<u8>> {
    if let Some(pending) = reader.pending.take() {
        return Ok(Some(pending));
    }

    let mut buffer = [0_u8; 1];
    let bytes_read = reader.inner.read(&mut buffer)?;
    if bytes_read == 0 {
        return Ok(None);
    }
    Ok(Some(buffer[0]))
}

struct BufferedInputReader<'a, R: Read> {
    inner: &'a mut R,
    pending: Option<u8>,
}

impl<'a, R: Read> BufferedInputReader<'a, R> {
    fn new(inner: &'a mut R) -> Self {
        Self {
            inner,
            pending: None,
        }
    }

    fn unread_byte(&mut self, byte: u8) {
        self.pending = Some(byte);
    }
}

fn noop_runtime_action<B: Backend>(_terminal: &mut Terminal<B>, _app: &mut App) -> Result<bool>
where
    B::Error: Display,
{
    Ok(false)
}

fn execute_catalog_action<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    action: Action,
    hooks: &RuntimeActionHooks<B>,
) -> Result<bool>
where
    B::Error: Display,
{
    match action {
        Action::SetAiTarget => open_target_picker(app, TargetRole::Ai, hooks),
        Action::SetEditorTarget => open_editor_target_picker_or_warn(app, hooks),
        Action::OpenContentSearch => Ok(app.open_active_content_search()),
        Action::TogglePreviewVisibility => Ok(app.toggle_active_preview_visibility()),
        Action::NewFile => open_prompt_for_action(app, PromptDialogIntent::NewFile),
        Action::NewDirectory => open_prompt_for_action(app, PromptDialogIntent::NewDirectory),
        Action::Rename => open_prompt_for_action(app, PromptDialogIntent::Rename),
        Action::Duplicate => open_prompt_for_action(app, PromptDialogIntent::Duplicate),
        Action::Move => open_prompt_for_action(app, PromptDialogIntent::Move),
        Action::Trash => open_trash_confirm(app),
        Action::SetContextModePreview => Ok(app.set_active_context_mode(ContextMode::Preview)),
        Action::SetContextModeDiff => Ok(app.activate_diff_mode_if_available()),
        Action::OpenInEditor => {
            if selected_node_is_file(app) {
                (hooks.open_in_editor)(terminal, app)
            } else {
                Ok(false)
            }
        }
        Action::OpenExternally => {
            if selected_path_abs_path(app).is_some() {
                (hooks.open_externally)(terminal, app)
            } else {
                Ok(false)
            }
        }
        Action::SendRelativePath => send_selected_relative_path(app, hooks),
        Action::CopyRelativePath => copy_selected_relative_path(app),
        Action::CopyAbsolutePath => copy_selected_absolute_path(app),
        Action::RevealInFinder => {
            if selected_path_abs_path(app).is_some() {
                (hooks.reveal_in_file_manager)(terminal, app)
            } else {
                Ok(false)
            }
        }
        Action::StageSelectedPath => stage_selected_path(app),
        Action::UnstageSelectedPath => unstage_selected_path(app),
        Action::PinBookmark => pin_bookmark_target(app),
        Action::UnpinBookmark => unpin_bookmark_target(app),
        Action::NewTab => open_selected_directory_as_root_tab(app),
        Action::CloseTab => close_active_tab(app),
        Action::OpenCommandPalette => Ok(app.open_unified_command_surface()),
        Action::OpenContextMenu => Ok(false),
        Action::FocusPathFilter => Ok(app.focus_path_filter()),
        Action::FocusNextPanel => Ok(app.focus_next_panel()),
        _ => Ok(false),
    }
}

fn selected_node_is_file(app: &App) -> bool {
    selected_node(app)
        .is_some_and(|node| matches!(node.kind, NodeKind::File | NodeKind::SymlinkFile))
}

fn selected_rel_path_buf(app: &App) -> Option<PathBuf> {
    app.active_batchable_selected_rel_path()
}

fn selected_path_abs_path(app: &App) -> Option<PathBuf> {
    let tab = app.tabs.get(app.active_tab)?;
    let rel_path = selected_rel_path_buf(app)?;
    Some(tab.root.join(rel_path))
}

fn selected_node_abs_path(app: &App) -> Option<std::path::PathBuf> {
    let node = selected_node(app)?;
    if !matches!(node.kind, NodeKind::File | NodeKind::SymlinkFile) {
        return None;
    }
    let tab = app.tabs.get(app.active_tab)?;
    Some(tab.root.join(&node.rel_path))
}

fn selected_editor_open_request(app: &App) -> Option<crate::open::EditorOpenRequest> {
    let tab = app.tabs.get(app.active_tab)?;
    if tab.mode == ContextMode::SearchResults {
        let hit = tab.selected_content_search_hit()?;
        return Some(crate::open::EditorOpenRequest::new(
            tab.root.join(&hit.path),
            hit.line.max(1),
        ));
    }

    let path = selected_node_abs_path(app)?;
    let line = match tab.mode {
        ContextMode::Preview => tab.preview.scroll_row.saturating_add(1).max(1),
        ContextMode::Diff => tab
            .preview
            .editor_line_hint
            .unwrap_or_else(|| tab.preview.scroll_row.saturating_add(1).max(1)),
        _ => tab.preview.editor_line_hint.unwrap_or(1),
    };
    Some(crate::open::EditorOpenRequest::new(path, line))
}

fn selected_node(app: &App) -> Option<&Node> {
    let tab = app.tabs.get(app.active_tab)?;
    let row = tab.tree.visible_rows.get(tab.tree.selected_row)?;
    tab.tree.node(row.node_id)
}

fn open_prompt_for_action(app: &mut App, intent: PromptDialogIntent) -> Result<bool> {
    let dialog = match intent {
        PromptDialogIntent::NewFile => PromptDialogState {
            title: "New file".to_string(),
            subtitle: Some("Enter the destination path relative to the active root".to_string()),
            value: creation_prompt_seed(app),
            confirm_label: "create".to_string(),
            intent,
        },
        PromptDialogIntent::NewDirectory => PromptDialogState {
            title: "New directory".to_string(),
            subtitle: Some("Enter the destination path relative to the active root".to_string()),
            value: creation_prompt_seed(app),
            confirm_label: "create".to_string(),
            intent,
        },
        PromptDialogIntent::Rename => {
            let Some(source_rel) = require_selected_rel_path(app, "rename") else {
                return Ok(true);
            };
            PromptDialogState {
                title: "Rename".to_string(),
                subtitle: Some("Enter the new sibling name".to_string()),
                value: sibling_name(&source_rel),
                confirm_label: "rename".to_string(),
                intent,
            }
        }
        PromptDialogIntent::Duplicate => {
            let Some(source_rel) = require_selected_rel_path(app, "duplicate") else {
                return Ok(true);
            };
            PromptDialogState {
                title: "Duplicate".to_string(),
                subtitle: Some(
                    "Enter the destination path relative to the active root".to_string(),
                ),
                value: duplicate_prompt_seed(&source_rel),
                confirm_label: "duplicate".to_string(),
                intent,
            }
        }
        PromptDialogIntent::Move => {
            let Some(source_rel) = require_selected_rel_path(app, "move") else {
                return Ok(true);
            };
            PromptDialogState {
                title: "Move".to_string(),
                subtitle: Some(
                    "Enter the destination path relative to the active root".to_string(),
                ),
                value: source_rel.display().to_string(),
                confirm_label: "move".to_string(),
                intent,
            }
        }
    };

    Ok(app.open_prompt_dialog(dialog))
}

fn open_trash_confirm(app: &mut App) -> Result<bool> {
    let Some(rel_path) = require_selected_rel_path(app, "trash") else {
        return Ok(true);
    };
    Ok(app.open_confirm_dialog(ConfirmDialogState {
        title: "Trash path".to_string(),
        message: format!("Move {} to trash?", rel_path.display()),
        confirm_label: "trash".to_string(),
        intent: ConfirmDialogIntent::TrashPath { rel_path },
    }))
}

fn require_selected_rel_path(app: &mut App, action_label: &str) -> Option<PathBuf> {
    let rel_path = selected_rel_path_buf(app);
    if rel_path.is_none() {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = format!("select a non-root path to {action_label}");
    }
    rel_path
}

fn creation_prompt_seed(app: &App) -> String {
    let Some(node) = selected_node(app) else {
        return String::new();
    };

    let base = match node.kind {
        NodeKind::Directory | NodeKind::SymlinkDirectory => node.rel_path.clone(),
        _ => node
            .rel_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
    };
    if base.as_os_str().is_empty() {
        String::new()
    } else {
        format!("{}/", base.display())
    }
}

fn sibling_name(rel_path: &Path) -> String {
    rel_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn duplicate_prompt_seed(source_rel: &Path) -> String {
    let parent = source_rel.parent().unwrap_or_else(|| Path::new(""));
    let sibling = sibling_name(source_rel);
    let duplicate_name = if let Some((stem, ext)) = sibling.rsplit_once('.') {
        if stem.is_empty() {
            format!("{sibling} copy")
        } else {
            format!("{stem} copy.{ext}")
        }
    } else {
        format!("{sibling} copy")
    };
    let dest = if parent.as_os_str().is_empty() {
        PathBuf::from(duplicate_name)
    } else {
        parent.join(duplicate_name)
    };
    dest.display().to_string()
}

fn commit_prompt_dialog(app: &mut App) -> Result<bool> {
    let Some(prompt) = app.prompt_dialog_state().cloned() else {
        return Ok(false);
    };
    if prompt.value.is_empty() {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = "path cannot be empty".to_string();
        return Ok(true);
    }

    let execution = match execute_prompt_dialog_intent(
        app,
        prompt.intent,
        PathBuf::from(prompt.value),
        false,
    ) {
        Ok(execution) => execution,
        Err(err) => {
            set_runtime_error_status(
                app,
                format!(
                    "{} failed: {}",
                    prompt_intent_label(prompt.intent),
                    runtime_error_text(&err)
                ),
            );
            return Ok(true);
        }
    };

    match execution {
        FileOpExecution::Applied {
            message,
            reveal_rel_path,
        } => {
            let _ = app.close_dialog();
            if let Err(err) = app.refresh_active_tab_after_file_op(reveal_rel_path.as_deref()) {
                set_runtime_error_status(
                    app,
                    format!(
                        "{} applied but refresh failed: {err}",
                        prompt_intent_label(prompt.intent)
                    ),
                );
                return Ok(true);
            }
            app.status.severity = StatusSeverity::Success;
            app.status.message = message;
            Ok(true)
        }
        FileOpExecution::Noop => Ok(true),
        FileOpExecution::OpenConfirm(dialog) => Ok(app.open_confirm_dialog(dialog)),
    }
}

fn commit_confirm_dialog(app: &mut App) -> Result<bool> {
    let Some(confirm) = app.confirm_dialog_state().cloned() else {
        return Ok(false);
    };

    match confirm.intent {
        ConfirmDialogIntent::TrashPath { rel_path } => {
            let root = active_root(app)?;
            if let Err(err) = crate::file_ops::trash_path(&root, &rel_path) {
                set_runtime_error_status(app, format!("trash failed: {err}"));
                return Ok(true);
            }
            let reveal_rel_path = rel_path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .map(Path::to_path_buf);
            let _ = app.close_dialog();
            if let Err(err) = app.refresh_active_tab_after_file_op(reveal_rel_path.as_deref()) {
                set_runtime_error_status(app, format!("trash applied but refresh failed: {err}"));
                return Ok(true);
            }
            app.status.severity = StatusSeverity::Success;
            app.status.message = format!("trashed {}", rel_path.display());
            Ok(true)
        }
        ConfirmDialogIntent::OverwriteDestination {
            operation,
            source_rel,
            dest_rel,
        } => {
            let execution =
                match execute_source_dest_intent(app, operation, source_rel, dest_rel, true) {
                    Ok(execution) => execution,
                    Err(err) => {
                        set_runtime_error_status(
                            app,
                            format!(
                                "{} failed: {}",
                                prompt_intent_label(operation),
                                runtime_error_text(&err)
                            ),
                        );
                        return Ok(true);
                    }
                };

            match execution {
                FileOpExecution::Applied {
                    message,
                    reveal_rel_path,
                } => {
                    let _ = app.close_dialog();
                    if let Err(err) =
                        app.refresh_active_tab_after_file_op(reveal_rel_path.as_deref())
                    {
                        set_runtime_error_status(
                            app,
                            format!(
                                "{} applied but refresh failed: {err}",
                                prompt_intent_label(operation)
                            ),
                        );
                        return Ok(true);
                    }
                    app.status.severity = StatusSeverity::Success;
                    app.status.message = message;
                    Ok(true)
                }
                FileOpExecution::Noop => Ok(true),
                FileOpExecution::OpenConfirm(dialog) => Ok(app.open_confirm_dialog(dialog)),
            }
        }
    }
}

enum FileOpExecution {
    Noop,
    Applied {
        message: String,
        reveal_rel_path: Option<PathBuf>,
    },
    OpenConfirm(ConfirmDialogState),
}

fn execute_prompt_dialog_intent(
    app: &mut App,
    intent: PromptDialogIntent,
    input_rel_path: PathBuf,
    overwrite_confirmed: bool,
) -> Result<FileOpExecution> {
    match intent {
        PromptDialogIntent::NewFile => {
            let root = active_root(app)?;
            let created = crate::file_ops::create_file(&root, &input_rel_path)?;
            Ok(FileOpExecution::Applied {
                message: format!("created {}", created.display()),
                reveal_rel_path: Some(created),
            })
        }
        PromptDialogIntent::NewDirectory => {
            let root = active_root(app)?;
            let created = crate::file_ops::create_directory(&root, &input_rel_path)?;
            Ok(FileOpExecution::Applied {
                message: format!("created {}", created.display()),
                reveal_rel_path: Some(created),
            })
        }
        PromptDialogIntent::Rename => {
            let Some(source_rel) = require_selected_rel_path(app, "rename") else {
                return Ok(FileOpExecution::Noop);
            };
            if input_rel_path.components().count() != 1 {
                app.status.severity = StatusSeverity::Warning;
                app.status.message = "rename expects a new sibling name".to_string();
                return Ok(FileOpExecution::Noop);
            }
            let dest_rel = source_rel
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .map(|parent| parent.join(&input_rel_path))
                .unwrap_or(input_rel_path);
            execute_source_dest_intent(
                app,
                PromptDialogIntent::Rename,
                source_rel,
                dest_rel,
                overwrite_confirmed,
            )
        }
        PromptDialogIntent::Duplicate => {
            let Some(source_rel) = require_selected_rel_path(app, "duplicate") else {
                return Ok(FileOpExecution::Noop);
            };
            execute_source_dest_intent(
                app,
                PromptDialogIntent::Duplicate,
                source_rel,
                input_rel_path,
                overwrite_confirmed,
            )
        }
        PromptDialogIntent::Move => {
            let Some(source_rel) = require_selected_rel_path(app, "move") else {
                return Ok(FileOpExecution::Noop);
            };
            execute_source_dest_intent(
                app,
                PromptDialogIntent::Move,
                source_rel,
                input_rel_path,
                overwrite_confirmed,
            )
        }
    }
}

fn execute_source_dest_intent(
    app: &mut App,
    intent: PromptDialogIntent,
    source_rel: PathBuf,
    dest_rel: PathBuf,
    overwrite_confirmed: bool,
) -> Result<FileOpExecution> {
    let root = active_root(app)?;
    let result = if overwrite_confirmed && root.join(&dest_rel).exists() {
        execute_confirmed_overwrite(&root, intent, &source_rel, &dest_rel)
    } else {
        execute_source_dest_operation(&root, intent, &source_rel, &dest_rel)
    };

    match result {
        Ok(dest_rel) => Ok(FileOpExecution::Applied {
            message: match intent {
                PromptDialogIntent::Rename => format!("renamed to {}", dest_rel.display()),
                PromptDialogIntent::Duplicate => format!("duplicated to {}", dest_rel.display()),
                PromptDialogIntent::Move => format!("moved to {}", dest_rel.display()),
                PromptDialogIntent::NewFile | PromptDialogIntent::NewDirectory => unreachable!(),
            },
            reveal_rel_path: Some(dest_rel),
        }),
        Err(crate::error::GroveError::Io(err))
            if err.kind() == std::io::ErrorKind::AlreadyExists && !overwrite_confirmed =>
        {
            Ok(FileOpExecution::OpenConfirm(ConfirmDialogState {
                title: "Overwrite destination".to_string(),
                message: format!("{} already exists. Replace it?", dest_rel.display()),
                confirm_label: "replace".to_string(),
                intent: ConfirmDialogIntent::OverwriteDestination {
                    operation: intent,
                    source_rel,
                    dest_rel,
                },
            }))
        }
        Err(err) => Err(err),
    }
}

fn execute_source_dest_operation(
    root: &Path,
    intent: PromptDialogIntent,
    source_rel: &Path,
    dest_rel: &Path,
) -> Result<PathBuf> {
    match intent {
        PromptDialogIntent::Rename => crate::file_ops::rename_path(root, source_rel, dest_rel),
        PromptDialogIntent::Duplicate => {
            crate::file_ops::duplicate_path(root, source_rel, dest_rel)
        }
        PromptDialogIntent::Move => crate::file_ops::move_path(root, source_rel, dest_rel),
        PromptDialogIntent::NewFile | PromptDialogIntent::NewDirectory => unreachable!(),
    }
}

fn execute_confirmed_overwrite(
    root: &Path,
    intent: PromptDialogIntent,
    source_rel: &Path,
    dest_rel: &Path,
) -> Result<PathBuf> {
    let backup_rel = allocate_overwrite_backup_rel_path(root, dest_rel)?;
    crate::file_ops::move_path(root, dest_rel, &backup_rel)?;

    match execute_source_dest_operation(root, intent, source_rel, dest_rel) {
        Ok(result) => {
            if let Err(err) = delete_internal_abs_path(&root.join(&backup_rel)) {
                crate::debug_log::log(&format!(
                    "component=file_op_overwrite_cleanup_failed backup={} error={err}",
                    backup_rel.display()
                ));
            }
            Ok(result)
        }
        Err(err) => {
            if let Err(restore_err) = crate::file_ops::move_path(root, &backup_rel, dest_rel) {
                return Err(std::io::Error::other(format!(
                    "replace failed: {err}; restore failed: {restore_err}; backup left at {}",
                    root.join(&backup_rel).display()
                ))
                .into());
            }
            Err(err)
        }
    }
}

fn allocate_overwrite_backup_rel_path(root: &Path, dest_rel: &Path) -> Result<PathBuf> {
    let parent = dest_rel.parent().unwrap_or_else(|| Path::new(""));
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stem = dest_rel
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "path".to_string());

    for attempt in 0..128u16 {
        let candidate_name = format!(".grove-replace-backup-{stem}-{suffix}-{attempt}");
        let candidate = if parent.as_os_str().is_empty() {
            PathBuf::from(&candidate_name)
        } else {
            parent.join(&candidate_name)
        };
        if !root.join(&candidate).exists() {
            return Ok(candidate);
        }
    }

    Err(std::io::Error::other(format!(
        "unable to allocate overwrite backup for {}",
        dest_rel.display()
    ))
    .into())
}

fn delete_internal_abs_path(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn active_root(app: &App) -> Result<PathBuf> {
    app.tabs
        .get(app.active_tab)
        .map(|tab| tab.root.clone())
        .ok_or_else(|| std::io::Error::other("active tab missing").into())
}

fn copy_selected_relative_path(app: &mut App) -> Result<bool> {
    let Some(rel_path) = selected_rel_path_buf(app) else {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = "select a non-root path to copy".to_string();
        return Ok(true);
    };

    let text = crate::file_ops::relative_path_text(&rel_path);
    copy_text_with(
        app,
        &text,
        format!("copied {}", rel_path.display()),
        copy_text_to_clipboard,
    )
}

fn copy_selected_absolute_path(app: &mut App) -> Result<bool> {
    let Some(path) = selected_path_abs_path(app) else {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = "select a non-root path to copy".to_string();
        return Ok(true);
    };

    let root = active_root(app)?;
    let rel_path = selected_rel_path_buf(app).expect("absolute path requires relative selection");
    let text = crate::file_ops::absolute_path_text(&root, &rel_path);
    copy_text_with(
        app,
        &text,
        format!("copied {}", path.display()),
        copy_text_to_clipboard,
    )
}

fn copy_active_preview_selection<B: Backend>(terminal: &Terminal<B>, app: &mut App) -> Result<bool>
where
    B::Error: Display,
{
    copy_active_preview_selection_with(terminal, app, copy_text_to_clipboard)
}

fn copy_active_preview_selection_with<B: Backend, F>(
    terminal: &Terminal<B>,
    app: &mut App,
    copy: F,
) -> Result<bool>
where
    B::Error: Display,
    F: FnOnce(&str) -> Result<()>,
{
    let Some(split_ratio) = app.tabs.get(app.active_tab).map(|tab| tab.split_ratio) else {
        return Ok(false);
    };
    let (preview_width, _) = preview_viewport_dimensions(terminal, app, split_ratio)?;
    let _ = app.refresh_active_preview_render_cache(preview_width);

    let Some((text, success_message)) = app.tabs.get(app.active_tab).and_then(|tab| {
        let cache = tab.preview.render_cache.as_ref()?;
        let (start, end, success_message) =
            if let Some((selection_start, selection_end)) = tab.preview.preview_selection_range() {
                (
                    selection_start,
                    selection_end,
                    "copied preview selection".to_string(),
                )
            } else {
                let line = tab
                    .preview
                    .cursor_line
                    .min(cache.lines.len().saturating_sub(1));
                (line, line, "copied preview line".to_string())
            };
        let text = cache
            .lines
            .get(start..=end)?
            .iter()
            .map(|line| line.to_string().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        Some((text, success_message))
    }) else {
        return Ok(false);
    };

    copy_text_with(app, &text, success_message, copy)
}

fn copy_text_to_clipboard(text: &str) -> Result<()> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|err| std::io::Error::other(err.to_string()))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    Ok(())
}

fn copy_text_with<F>(app: &mut App, text: &str, success_message: String, copy: F) -> Result<bool>
where
    F: FnOnce(&str) -> Result<()>,
{
    match copy(text) {
        Ok(()) => {
            app.status.severity = StatusSeverity::Success;
            app.status.message = success_message;
        }
        Err(err) => {
            set_runtime_error_status(app, format!("copy failed: {}", runtime_error_text(&err)));
        }
    }
    Ok(true)
}

fn prompt_intent_label(intent: PromptDialogIntent) -> &'static str {
    match intent {
        PromptDialogIntent::NewFile => "create file",
        PromptDialogIntent::NewDirectory => "create directory",
        PromptDialogIntent::Rename => "rename",
        PromptDialogIntent::Duplicate => "duplicate",
        PromptDialogIntent::Move => "move",
    }
}

fn set_runtime_error_status(app: &mut App, message: String) {
    app.status.severity = StatusSeverity::Error;
    app.status.message = message;
}

fn runtime_error_text(err: &GroveError) -> String {
    match err {
        GroveError::Io(io) => io.to_string(),
        GroveError::Git(git) => git.to_string(),
    }
}

fn production_reveal_in_file_manager(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<bool> {
    let Some(path) = selected_path_abs_path(app) else {
        return Ok(false);
    };

    let command = crate::open::resolve_reveal_command(&path);
    if let Err(err) = suspend_terminal_for_command(terminal, &command) {
        app.status.severity = StatusSeverity::Error;
        app.status.message = format!("reveal failed: {err}");
    } else {
        app.status.severity = StatusSeverity::Success;
        app.status.message = format!("revealed {}", path.display());
    }
    Ok(true)
}

fn stage_selected_path(app: &mut App) -> Result<bool> {
    mutate_selected_path(app, GitMutation::Stage)
}

fn unstage_selected_path(app: &mut App) -> Result<bool> {
    mutate_selected_path(app, GitMutation::Unstage)
}

fn open_selected_directory_as_root_tab(app: &mut App) -> Result<bool> {
    let Some(root) = app.selected_directory_root_candidate() else {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = "select a directory to open as a root tab".to_string();
        return Ok(true);
    };

    if !app.open_selected_directory_as_root_tab(root.clone()) {
        return Ok(false);
    }

    let label = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("root tab");
    app.status.severity = StatusSeverity::Success;
    app.status.message = format!("opened {label} as a root tab");
    Ok(true)
}

fn activate_selected_root_or_warn(app: &mut App) -> bool {
    let Some(path) = app.selected_root_path() else {
        return false;
    };

    match path.try_exists() {
        Ok(true) => app.activate_selected_root(),
        Ok(false) => set_root_activation_warning(app, format!("missing root: {}", path.display())),
        Err(err) => set_root_activation_warning(
            app,
            format!("could not open root {}: {err}", path.display()),
        ),
    }
}

fn set_root_activation_warning(app: &mut App, message: String) -> bool {
    let changed = app.status.severity != StatusSeverity::Warning || app.status.message != message;
    app.status.severity = StatusSeverity::Warning;
    app.status.message = message;
    changed
}

fn open_root_picker(app: &mut App) -> Result<bool> {
    app.open_add_root_directory_picker()
}

fn toggle_bookmark_target(app: &mut App) -> Result<bool> {
    let removing = app.bookmark_target_is_bookmarked();
    let target_is_active_root = app.bookmark_target_is_active_root();
    if !app.toggle_bookmark_target() {
        return Ok(false);
    }

    app.save_config()?;
    app.status.severity = StatusSeverity::Success;
    app.status.message = if removing {
        if target_is_active_root {
            "active root unpinned".to_string()
        } else {
            "selected root unpinned".to_string()
        }
    } else if target_is_active_root {
        "active root pinned".to_string()
    } else {
        "selected root pinned".to_string()
    };
    Ok(true)
}

fn pin_bookmark_target(app: &mut App) -> Result<bool> {
    let target_is_active_root = app.bookmark_target_is_active_root();
    if !app.pin_bookmark_target() {
        return Ok(false);
    }

    app.save_config()?;
    app.status.severity = StatusSeverity::Success;
    app.status.message = if target_is_active_root {
        "active root pinned".to_string()
    } else {
        "selected root pinned".to_string()
    };
    Ok(true)
}

fn unpin_bookmark_target(app: &mut App) -> Result<bool> {
    let target_is_active_root = app.bookmark_target_is_active_root();
    if !app.unpin_bookmark_target() {
        return Ok(false);
    }

    app.save_config()?;
    app.status.severity = StatusSeverity::Success;
    app.status.message = if target_is_active_root {
        "active root unpinned".to_string()
    } else {
        "selected root unpinned".to_string()
    };
    Ok(true)
}

fn close_active_tab(app: &mut App) -> Result<bool> {
    let closed_label = app
        .tab_label(app.active_tab)
        .unwrap_or_else(|| "tab".to_string());
    if !app.close_active_tab() {
        return Ok(false);
    }

    app.focus = Focus::Tree;
    app.status.severity = StatusSeverity::Success;
    app.status.message = format!("closed {closed_label}");
    Ok(true)
}

fn mutate_selected_path(app: &mut App, mutation: GitMutation) -> Result<bool> {
    let Some(node) = selected_node(app) else {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = mutation.root_warning().to_string();
        return Ok(true);
    };

    if node.rel_path.as_os_str().is_empty() {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = mutation.root_warning().to_string();
        return Ok(true);
    }

    if matches!(node.kind, NodeKind::Directory | NodeKind::SymlinkDirectory) {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = mutation.root_warning().to_string();
        return Ok(true);
    }

    if node.git == GitStatus::Conflicted {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = "resolve the conflict before staging or unstaging".to_string();
        return Ok(true);
    }

    if !matches!(node.kind, NodeKind::File | NodeKind::SymlinkFile) {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = "selected item is not supported for staging or unstaging".to_string();
        return Ok(true);
    }

    let repo = match app
        .tabs
        .get(app.active_tab)
        .and_then(|tab| tab.git.repo.clone())
    {
        Some(repo) => repo,
        None => {
            app.status.severity = StatusSeverity::Warning;
            app.status.message = "git repository is unavailable for this selection".to_string();
            return Ok(true);
        }
    };
    let rel_path = node.rel_path.clone();
    let backend = LibgitBackend;
    let result = match mutation {
        GitMutation::Stage => backend.stage_path(&repo, &rel_path),
        GitMutation::Unstage => backend.unstage_path(&repo, &rel_path),
    };
    match result {
        Ok(()) => {
            app.status.severity = StatusSeverity::Success;
            app.status.message = format!("{} {}", mutation.success_verb(), rel_path.display());
            app.invalidate_active_preview();
            if let Some(tab) = app.tabs.get_mut(app.active_tab) {
                tab.git.needs_refresh = true;
            }
            Ok(true)
        }
        Err(err) => {
            app.status.severity = StatusSeverity::Error;
            app.status.message = format!("{} failed: {err}", mutation.error_prefix());
            Ok(true)
        }
    }
}

#[derive(Debug, Copy, Clone)]
enum GitMutation {
    Stage,
    Unstage,
}

impl GitMutation {
    fn success_verb(self) -> &'static str {
        match self {
            GitMutation::Stage => "staged",
            GitMutation::Unstage => "unstaged",
        }
    }

    fn error_prefix(self) -> &'static str {
        match self {
            GitMutation::Stage => "git stage",
            GitMutation::Unstage => "git unstage",
        }
    }

    fn root_warning(self) -> &'static str {
        "select a file before staging or unstaging"
    }
}

fn move_selection_up(app: &mut App) -> bool {
    let Some(tab) = app.tabs.get_mut(app.active_tab) else {
        return false;
    };
    let previous = tab.tree.selected_row;
    tab.tree.select_prev();
    tab.tree.selected_row != previous
}

fn move_selection_down(app: &mut App) -> bool {
    let Some(tab) = app.tabs.get_mut(app.active_tab) else {
        return false;
    };
    let previous = tab.tree.selected_row;
    tab.tree.select_next();
    tab.tree.selected_row != previous
}

fn move_selection_left(app: &mut App) -> bool {
    let Some(tab) = app.tabs.get_mut(app.active_tab) else {
        return false;
    };
    tab.tree.collapse_selected_or_select_parent()
}

fn move_selection_right(app: &mut App) -> bool {
    let Some(tab) = app.tabs.get_mut(app.active_tab) else {
        return false;
    };
    tab.tree.expand_selected()
}

fn path_filter_query_is_empty(app: &App) -> bool {
    app.tabs
        .get(app.active_tab)
        .map(|tab| tab.path_filter.query.is_empty())
        .unwrap_or(true)
}

fn open_target_picker<B: Backend>(
    app: &mut App,
    role: TargetRole,
    hooks: &RuntimeActionHooks<B>,
) -> Result<bool>
where
    B::Error: Display,
{
    let sessions = (hooks.list_sessions)(app)?;
    if !app.open_target_picker(role, sessions) {
        if app.status.message.is_empty() {
            app.status.severity = StatusSeverity::Warning;
            app.status.message = "no sessions available for picker".to_string();
        }
        return Ok(true);
    }
    Ok(true)
}

fn open_editor_target_picker_or_warn<B: Backend>(
    app: &mut App,
    hooks: &RuntimeActionHooks<B>,
) -> Result<bool>
where
    B::Error: Display,
{
    if selected_editor_open_request(app).is_none() {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = "select a file before choosing an editor target".to_string();
        return Ok(true);
    }

    open_target_picker(app, TargetRole::Editor, hooks)
}

fn send_selected_relative_path<B: Backend>(
    app: &mut App,
    hooks: &RuntimeActionHooks<B>,
) -> Result<bool>
where
    B::Error: Display,
{
    let relative_paths = app.active_sendable_rel_paths();
    if relative_paths.is_empty() {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = "select a file or folder to send".to_string();
        return Ok(true);
    }
    let payload = relative_paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");

    let response = (hooks.send_text)(app, SendTarget::Role(TargetRole::Ai), payload, false)?;
    match response {
        BridgeResponse::SendOk { target_session_id } => {
            app.bridge.ai_target_session_id = Some(target_session_id.clone());
            let target_label = app.bridge_target_label(TargetRole::Ai);
            app.status.severity = StatusSeverity::Success;
            app.status.message = if relative_paths.len() == 1 {
                format!("sent relative path to {target_label}")
            } else {
                format!(
                    "sent {} relative paths to {target_label}",
                    relative_paths.len()
                )
            };
            Ok(true)
        }
        BridgeResponse::ManualSelectionRequired { role } => open_target_picker(app, role, hooks),
        BridgeResponse::Error { message } => {
            app.status.severity = StatusSeverity::Error;
            app.status.message = format!("bridge send failed: {message}");
            Ok(true)
        }
        _ => {
            app.status.severity = StatusSeverity::Error;
            app.status.message = "bridge returned an unexpected send response".to_string();
            Ok(true)
        }
    }
}

fn handle_dialog_input<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    input: RuntimeInput,
    hooks: &RuntimeActionHooks<B>,
) -> Result<RuntimeControlFlow>
where
    B::Error: Display,
{
    let changed = match app.dialog_state() {
        Some(DialogState::TargetPicker(_)) => match input {
            RuntimeInput::MoveUp => app.move_target_picker_selection(-1),
            RuntimeInput::MoveDown => app.move_target_picker_selection(1),
            RuntimeInput::Enter => commit_target_picker(terminal, app, hooks)?,
            RuntimeInput::Escape => app.close_dialog(),
            _ => false,
        },
        Some(DialogState::DirectoryPicker(_)) => match input {
            RuntimeInput::MoveUp => app.move_directory_picker_selection(-1),
            RuntimeInput::MoveDown => app.move_directory_picker_selection(1),
            RuntimeInput::MoveLeft => app.move_directory_picker_to_parent()?,
            RuntimeInput::MoveRight => app.enter_directory_picker_selection()?,
            RuntimeInput::Enter => commit_directory_picker(app)?,
            RuntimeInput::Escape => app.close_directory_picker_dialog(),
            _ => false,
        },
        Some(DialogState::Prompt(_)) => match input {
            RuntimeInput::Character(ch) => app.append_prompt_dialog_char(ch),
            RuntimeInput::SetPreviewMode => app.append_prompt_dialog_char('p'),
            RuntimeInput::SetDiffMode => app.append_prompt_dialog_char('d'),
            RuntimeInput::StageSelectedPath => app.append_prompt_dialog_char('s'),
            RuntimeInput::UnstageSelectedPath => app.append_prompt_dialog_char('u'),
            RuntimeInput::FocusPathFilter => app.append_prompt_dialog_char('/'),
            RuntimeInput::Backspace => app.backspace_prompt_dialog(),
            RuntimeInput::Enter => commit_prompt_dialog(app)?,
            RuntimeInput::Escape => app.close_dialog(),
            RuntimeInput::MoveUp
            | RuntimeInput::MoveDown
            | RuntimeInput::MoveLeft
            | RuntimeInput::MoveRight
            | RuntimeInput::SetAiTarget
            | RuntimeInput::SetEditorTarget
            | RuntimeInput::SendRelativePath
            | RuntimeInput::ExtendPreviewSelectionUp
            | RuntimeInput::ExtendPreviewSelectionDown
            | RuntimeInput::PreviewClick { .. }
            | RuntimeInput::WheelUp { .. }
            | RuntimeInput::WheelDown { .. }
            | RuntimeInput::FocusNextPanel
            | RuntimeInput::OpenContentSearch
            | RuntimeInput::OpenCommandPalette
            | RuntimeInput::OpenRootPicker
            | RuntimeInput::OpenSelectedDirectoryAsRootTab
            | RuntimeInput::ToggleHidden
            | RuntimeInput::ToggleGitignore
            | RuntimeInput::PageUp
            | RuntimeInput::PageDown
            | RuntimeInput::Home
            | RuntimeInput::End
            | RuntimeInput::Ignore => false,
        },
        Some(DialogState::Confirm(_)) => match input {
            RuntimeInput::Enter => commit_confirm_dialog(app)?,
            RuntimeInput::Escape => app.close_dialog(),
            _ => false,
        },
        None => false,
    };
    if changed {
        render_shell_once(terminal, app)?;
    }
    Ok(RuntimeControlFlow::Continue)
}

fn handle_content_search_input<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    input: RuntimeInput,
) -> Result<RuntimeControlFlow>
where
    B::Error: Display,
{
    let changed = match input {
        RuntimeInput::Character(ch) => app.append_active_content_search_char(ch),
        RuntimeInput::SetPreviewMode => app.append_active_content_search_char('p'),
        RuntimeInput::SetDiffMode => app.append_active_content_search_char('d'),
        RuntimeInput::StageSelectedPath => app.append_active_content_search_char('s'),
        RuntimeInput::UnstageSelectedPath => app.append_active_content_search_char('u'),
        RuntimeInput::FocusPathFilter => app.append_active_content_search_char('/'),
        RuntimeInput::Backspace => app.backspace_active_content_search(),
        RuntimeInput::Enter => {
            if app.tabs[app.active_tab].mode == ContextMode::SearchResults
                && app.tabs[app.active_tab]
                    .content_search
                    .selected_hit_index
                    .is_some()
                && matches!(
                    app.tabs[app.active_tab].content_search.status,
                    crate::app::ContentSearchStatus::Ready
                )
            {
                app.activate_selected_content_search_hit()
            } else {
                app.submit_active_content_search()?
            }
        }
        RuntimeInput::Escape => app.close_active_content_search(),
        RuntimeInput::MoveUp => app.move_active_content_search_selection(-1),
        RuntimeInput::MoveDown => app.move_active_content_search_selection(1),
        RuntimeInput::MoveLeft
        | RuntimeInput::MoveRight
        | RuntimeInput::ExtendPreviewSelectionUp
        | RuntimeInput::ExtendPreviewSelectionDown
        | RuntimeInput::PreviewClick { .. }
        | RuntimeInput::SetAiTarget
        | RuntimeInput::SetEditorTarget
        | RuntimeInput::SendRelativePath
        | RuntimeInput::WheelUp { .. }
        | RuntimeInput::WheelDown { .. }
        | RuntimeInput::FocusNextPanel
        | RuntimeInput::ToggleHidden
        | RuntimeInput::ToggleGitignore
        | RuntimeInput::PageUp
        | RuntimeInput::PageDown
        | RuntimeInput::Home
        | RuntimeInput::End
        | RuntimeInput::OpenContentSearch
        | RuntimeInput::OpenRootPicker
        | RuntimeInput::OpenSelectedDirectoryAsRootTab
        | RuntimeInput::OpenCommandPalette
        | RuntimeInput::Ignore => false,
    };
    if changed {
        render_shell_once(terminal, app)?;
    }
    Ok(RuntimeControlFlow::Continue)
}

fn handle_command_palette_input<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    input: RuntimeInput,
    hooks: &RuntimeActionHooks<B>,
) -> Result<RuntimeControlFlow>
where
    B::Error: Display,
{
    let changed = match input {
        RuntimeInput::Character(ch) => app.append_command_palette_char(ch),
        RuntimeInput::SetPreviewMode => app.append_command_palette_char('p'),
        RuntimeInput::SetDiffMode => app.append_command_palette_char('d'),
        RuntimeInput::StageSelectedPath => app.append_command_palette_char('s'),
        RuntimeInput::UnstageSelectedPath => app.append_command_palette_char('u'),
        RuntimeInput::FocusPathFilter => app.append_command_palette_char('/'),
        RuntimeInput::Backspace => app.backspace_command_palette(),
        RuntimeInput::MoveUp => app.move_command_palette_selection(-1),
        RuntimeInput::MoveDown => app.move_command_palette_selection(1),
        RuntimeInput::Enter => {
            if let Some(action) = app.commit_command_palette_action() {
                execute_catalog_action(terminal, app, action, hooks)?
            } else {
                false
            }
        }
        RuntimeInput::Escape => app.close_command_palette(),
        RuntimeInput::MoveLeft
        | RuntimeInput::MoveRight
        | RuntimeInput::ExtendPreviewSelectionUp
        | RuntimeInput::ExtendPreviewSelectionDown
        | RuntimeInput::PreviewClick { .. }
        | RuntimeInput::SetAiTarget
        | RuntimeInput::SetEditorTarget
        | RuntimeInput::SendRelativePath
        | RuntimeInput::WheelUp { .. }
        | RuntimeInput::WheelDown { .. }
        | RuntimeInput::FocusNextPanel
        | RuntimeInput::OpenContentSearch
        | RuntimeInput::OpenRootPicker
        | RuntimeInput::OpenSelectedDirectoryAsRootTab
        | RuntimeInput::OpenCommandPalette
        | RuntimeInput::ToggleHidden
        | RuntimeInput::ToggleGitignore
        | RuntimeInput::PageUp
        | RuntimeInput::PageDown
        | RuntimeInput::Home
        | RuntimeInput::End
        | RuntimeInput::Ignore => false,
    };
    if changed {
        render_shell_once(terminal, app)?;
    }
    Ok(RuntimeControlFlow::Continue)
}

fn commit_target_picker<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    hooks: &RuntimeActionHooks<B>,
) -> Result<bool>
where
    B::Error: Display,
{
    let Some(picker) = app.target_picker_state().cloned() else {
        return Ok(false);
    };
    if picker.role == TargetRole::Editor && picker.selection == TargetPickerSelection::CurrentPane {
        if selected_editor_open_request(app).is_none() {
            app.status.severity = StatusSeverity::Warning;
            app.status.message = "select a file before choosing an editor target".to_string();
            app.close_target_picker();
            return Ok(true);
        }
        app.bridge.editor_target_session_id = None;
        app.close_target_picker();
        return (hooks.open_in_editor)(terminal, app);
    }

    let Some(session) = app.target_picker_selected_session().cloned() else {
        app.status.severity = StatusSeverity::Warning;
        app.status.message = "select a target session".to_string();
        return Ok(true);
    };

    let session_id = session.session_id.clone();
    let response = (hooks.set_role)(app, session_id.clone(), picker.role)?;
    match response {
        BridgeResponse::Pong => {
            match picker.role {
                TargetRole::Ai => app.bridge.ai_target_session_id = Some(session_id.clone()),
                TargetRole::Editor => {
                    app.bridge.editor_target_session_id = Some(session_id.clone())
                }
                TargetRole::Grove => {}
            }
            app.close_target_picker();
            if picker.role == TargetRole::Editor {
                if selected_editor_open_request(app).is_none() {
                    app.status.severity = StatusSeverity::Warning;
                    app.status.message =
                        "select a file before choosing an editor target".to_string();
                    return Ok(true);
                }
                (hooks.open_in_editor)(terminal, app)
            } else {
                let target_label = app.bridge_target_label(picker.role);
                app.status.severity = StatusSeverity::Success;
                app.status.message = format!(
                    "{} target set to {}",
                    target_role_label(picker.role),
                    target_label
                );
                Ok(true)
            }
        }
        BridgeResponse::Error { message } => {
            app.status.severity = StatusSeverity::Error;
            app.status.message = format!("target update failed: {message}");
            Ok(true)
        }
        _ => {
            app.status.severity = StatusSeverity::Error;
            app.status.message = "bridge returned an unexpected target response".to_string();
            Ok(true)
        }
    }
}

fn commit_directory_picker(app: &mut App) -> Result<bool> {
    let Some((selected_path, selected_label)) = app
        .directory_picker_state()
        .and_then(|picker| picker.entries.get(picker.selected_index))
        .map(|entry| (entry.path.clone(), entry.label.clone()))
    else {
        return Ok(false);
    };
    let normalized_target = fs::canonicalize(&selected_path).unwrap_or(selected_path.clone());
    let was_pinned = app
        .bookmark_paths()
        .iter()
        .any(|path| path == &normalized_target);
    let was_open = app.bookmark_is_open(&normalized_target);
    let success_label = directory_picker_commit_label(&selected_label, &normalized_target);

    let changed = match app.commit_directory_picker_selection() {
        Ok(changed) => changed,
        Err(error) => {
            let message = format!("could not add root {success_label}: {error}");
            if let Some(DialogState::DirectoryPicker(picker)) = app.overlays.dialog.as_mut() {
                picker.error_message = Some(message.clone());
            }
            app.status.severity = StatusSeverity::Error;
            app.status.message = message;
            return Ok(true);
        }
    };

    if !changed {
        return Ok(false);
    }

    app.status.severity = StatusSeverity::Success;
    app.status.message = match (was_pinned, was_open) {
        (true, true) => format!("activated root {success_label}"),
        (true, false) => format!("opened pinned root {success_label}"),
        (false, true) => format!("pinned existing root {success_label}"),
        (false, false) => format!("added root {success_label}"),
    };
    Ok(true)
}

fn directory_picker_commit_label(selected_label: &str, target_root: &Path) -> String {
    match selected_label {
        "." | ".." => target_root
            .file_name()
            .and_then(|name| name.to_str())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| target_root.display().to_string()),
        _ => selected_label.to_string(),
    }
}

fn target_role_label(role: TargetRole) -> &'static str {
    match role {
        TargetRole::Ai => "ai",
        TargetRole::Editor => "editor",
        TargetRole::Grove => "grove",
    }
}

fn sync_selected_row_visibility<B: Backend>(terminal: &Terminal<B>, app: &mut App) -> Result<()>
where
    B::Error: Display,
{
    let Some(split_ratio) = app.tabs.get(app.active_tab).map(|tab| tab.split_ratio) else {
        return Ok(());
    };
    let viewport_height = tree_viewport_height(terminal, app, split_ratio)?;
    let Some(tab) = app.tabs.get_mut(app.active_tab) else {
        return Ok(());
    };
    tab.tree.ensure_selected_row_visible(viewport_height);
    Ok(())
}

fn sync_active_preview_scroll<B: Backend>(terminal: &Terminal<B>, app: &mut App) -> Result<()>
where
    B::Error: Display,
{
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Ok(());
    };
    let split_ratio = tab.split_ratio;
    let (preview_width, viewport_height) = preview_viewport_dimensions(terminal, app, split_ratio)?;
    let _ = app.clamp_active_preview_scroll(preview_width, viewport_height);
    Ok(())
}

fn extend_active_preview_selection<B: Backend>(
    terminal: &Terminal<B>,
    app: &mut App,
    delta: isize,
) -> Result<bool>
where
    B::Error: Display,
{
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Ok(false);
    };
    let split_ratio = tab.split_ratio;
    let (preview_width, _) = preview_viewport_dimensions(terminal, app, split_ratio)?;
    Ok(app.extend_active_preview_selection(preview_width, delta))
}

fn set_active_preview_cursor_from_click<B: Backend>(
    terminal: &Terminal<B>,
    app: &mut App,
    column: u16,
    row: u16,
) -> Result<bool>
where
    B::Error: Display,
{
    if !mouse_targets_preview(terminal, app, column, row)? {
        return Ok(false);
    }
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Ok(false);
    };
    let split_ratio = tab.split_ratio;
    let areas = shell_areas(terminal, app)?;
    let content_top = areas.preview.y.saturating_add(1);
    let content_bottom = areas
        .preview
        .y
        .saturating_add(areas.preview.height.saturating_sub(1));
    if row < content_top || row >= content_bottom {
        return Ok(false);
    }
    let (preview_width, _) = preview_viewport_dimensions(terminal, app, split_ratio)?;
    let _ = app.refresh_active_preview_render_cache(preview_width);
    let row_offset = row.saturating_sub(content_top) as usize;
    let scroll_row = app
        .tabs
        .get(app.active_tab)
        .map(|tab| tab.preview.scroll_row)
        .unwrap_or(0);
    let target_line = scroll_row.saturating_add(row_offset);
    let focus_changed = app.focus != Focus::Preview;
    let cursor_changed = app.set_active_preview_cursor_line(preview_width, target_line);
    app.focus = Focus::Preview;
    Ok(focus_changed || cursor_changed)
}

fn mouse_targets_tree<B: Backend>(
    terminal: &Terminal<B>,
    app: &App,
    column: u16,
    row: u16,
) -> Result<bool>
where
    B::Error: Display,
{
    if app.tabs.get(app.active_tab).is_none() {
        return Ok(false);
    }
    let areas = shell_areas(terminal, app)?;
    Ok(rect_contains(areas.tree, column, row))
}

fn mouse_targets_preview<B: Backend>(
    terminal: &Terminal<B>,
    app: &App,
    column: u16,
    row: u16,
) -> Result<bool>
where
    B::Error: Display,
{
    if app.tabs.get(app.active_tab).is_none() {
        return Ok(false);
    }
    let areas = shell_areas(terminal, app)?;
    Ok(rect_contains(areas.preview, column, row))
}

fn rect_contains(area: ratatui::layout::Rect, column: u16, row: u16) -> bool {
    let right = area.x.saturating_add(area.width);
    let bottom = area.y.saturating_add(area.height);
    column >= area.x && column < right && row >= area.y && row < bottom
}

fn refresh_active_preview_render_cache<B: Backend>(
    terminal: &Terminal<B>,
    app: &mut App,
) -> Result<()>
where
    B::Error: Display,
{
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Ok(());
    };
    let split_ratio = tab.split_ratio;
    let (preview_width, _) = preview_viewport_dimensions(terminal, app, split_ratio)?;
    let _ = app.refresh_active_preview_render_cache(preview_width);
    Ok(())
}

fn tree_viewport_height<B: Backend>(
    terminal: &Terminal<B>,
    app: &App,
    split_ratio: f32,
) -> Result<usize>
where
    B::Error: Display,
{
    let _ = split_ratio;
    let areas = shell_areas(terminal, app)?;
    Ok(areas.tree.height.saturating_sub(2) as usize)
}

fn preview_viewport_dimensions<B: Backend>(
    terminal: &Terminal<B>,
    app: &App,
    split_ratio: f32,
) -> Result<(u16, usize)>
where
    B::Error: Display,
{
    let _ = split_ratio;
    let areas = shell_areas(terminal, app)?;
    Ok((
        areas.preview.width.saturating_sub(2),
        areas.preview.height.saturating_sub(2) as usize,
    ))
}

fn shell_areas<B: Backend>(
    terminal: &Terminal<B>,
    app: &App,
) -> Result<crate::ui::layout::ShellAreas>
where
    B::Error: Display,
{
    let shell_size = terminal
        .size()
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    let shell_area = ratatui::layout::Rect::new(0, 0, shell_size.width, shell_size.height);
    let split_ratio = app
        .tabs
        .get(app.active_tab)
        .map(|tab| tab.split_ratio)
        .unwrap_or(0.40);
    Ok(crate::ui::layout::compute(
        shell_area,
        split_ratio,
        app.root_navigator_panel_height(),
        app.active_preview_visible(),
    ))
}

fn production_open_in_editor(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<bool> {
    let Some(request) = selected_editor_open_request(app) else {
        return Ok(false);
    };

    match crate::open::resolve_editor_open(&app.config, &request) {
        crate::open::ResolvedEditorOpen::LocalProcess(command) => {
            if let Err(err) = suspend_terminal_for_command(terminal, &command) {
                app.status.severity = StatusSeverity::Error;
                app.status.message = format!("editor open failed: {err}");
            } else if let Some(tab) = app.tabs.get_mut(app.active_tab) {
                tab.preview.source.rel_path = None;
                tab.preview.render_cache = None;
            }
        }
        crate::open::ResolvedEditorOpen::ShellTarget(command_line) => {
            match production_send_text(
                app,
                SendTarget::Role(TargetRole::Editor),
                command_line,
                true,
            )? {
                BridgeResponse::SendOk { target_session_id } => {
                    app.bridge.editor_target_session_id = Some(target_session_id.clone());
                    let target_label = app.bridge_target_label(TargetRole::Editor);
                    app.status.severity = StatusSeverity::Success;
                    app.status.message = format!("sent editor open to {target_label}");
                }
                BridgeResponse::ManualSelectionRequired { role } => {
                    return open_target_picker(
                        app,
                        role,
                        &RuntimeActionHooks {
                            open_in_editor: production_open_in_editor,
                            open_externally: production_open_externally,
                            reveal_in_file_manager: production_reveal_in_file_manager,
                            initialize_bridge: production_initialize_bridge,
                            list_sessions: production_list_sessions,
                            send_text: production_send_text,
                            set_role: production_set_role,
                        },
                    );
                }
                BridgeResponse::Error { message } => {
                    app.status.severity = StatusSeverity::Error;
                    app.status.message = format!("editor target send failed: {message}");
                }
                _ => {
                    app.status.severity = StatusSeverity::Error;
                    app.status.message =
                        "bridge returned an unexpected editor response".to_string();
                }
            }
        }
    }

    Ok(true)
}

fn production_open_externally(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<bool> {
    let Some(path) = selected_path_abs_path(app) else {
        return Ok(false);
    };

    let command = crate::open::resolve_external_open_command(&path);
    if let Err(err) = suspend_terminal_for_command(terminal, &command) {
        app.status.severity = StatusSeverity::Error;
        app.status.message = format!("external open failed: {err}");
    }

    Ok(true)
}

fn noop_bridge_initialize(_app: &mut App) -> Result<()> {
    Ok(())
}

fn noop_list_sessions(_app: &mut App) -> Result<Vec<SessionSummary>> {
    Ok(Vec::new())
}

fn noop_set_role(_app: &mut App, _session_id: String, _role: TargetRole) -> Result<BridgeResponse> {
    Ok(BridgeResponse::Error {
        message: "bridge hooks are not configured".to_string(),
    })
}

fn noop_send_text(
    _app: &mut App,
    _target: SendTarget,
    _text: String,
    _append_newline: bool,
) -> Result<BridgeResponse> {
    Ok(BridgeResponse::Error {
        message: "bridge hooks are not configured".to_string(),
    })
}

fn production_initialize_bridge(app: &mut App) -> Result<()> {
    let instance_id = generate_instance_id();
    app.bridge.connected = false;
    app.bridge.instance_id = Some(instance_id.clone());

    if let Err(err) = emit_iterm2_user_var("groveRole", "grove")
        .and_then(|_| emit_iterm2_user_var("groveInstance", &instance_id))
    {
        app.status.severity = StatusSeverity::Error;
        app.status.message = format!("bridge self-tag failed: {err}");
        return Ok(());
    }

    let mut client =
        crate::bridge::client::BridgeClient::new(crate::bridge::client::default_socket_path());
    match client.send_command(BridgeCommand::Ping) {
        Ok(BridgeResponse::Pong) => {
            app.bridge.connected = true;
        }
        Ok(_) => {
            app.status.severity = StatusSeverity::Warning;
            app.status.message = "bridge handshake returned an unexpected response".to_string();
        }
        Err(err) => {
            app.status.severity = StatusSeverity::Warning;
            app.status.message = format!("bridge unavailable: {err}");
        }
    }

    Ok(())
}

fn production_list_sessions(app: &mut App) -> Result<Vec<SessionSummary>> {
    let Some(instance_id) = app.bridge.instance_id.clone() else {
        return Ok(Vec::new());
    };
    let mut client =
        crate::bridge::client::BridgeClient::new(crate::bridge::client::default_socket_path());
    match client.send_command(BridgeCommand::ListSessions { instance_id }) {
        Ok(BridgeResponse::SessionList(sessions)) => {
            app.bridge.connected = true;
            Ok(sessions)
        }
        Ok(BridgeResponse::Error { message }) => {
            app.status.severity = StatusSeverity::Error;
            app.status.message = format!("bridge session list failed: {message}");
            Ok(Vec::new())
        }
        Ok(_) => {
            app.status.severity = StatusSeverity::Error;
            app.status.message = "bridge returned an unexpected session-list response".to_string();
            Ok(Vec::new())
        }
        Err(err) => {
            app.bridge.connected = false;
            app.status.severity = StatusSeverity::Error;
            app.status.message = format!("bridge session list failed: {err}");
            Ok(Vec::new())
        }
    }
}

fn production_set_role(
    app: &mut App,
    session_id: String,
    role: TargetRole,
) -> Result<BridgeResponse> {
    if app.bridge.instance_id.is_none() {
        return Ok(BridgeResponse::Error {
            message: "bridge is not initialized".to_string(),
        });
    }
    let mut client =
        crate::bridge::client::BridgeClient::new(crate::bridge::client::default_socket_path());
    match client.send_command(BridgeCommand::SetRole { session_id, role }) {
        Ok(response) => {
            app.bridge.connected = true;
            Ok(response)
        }
        Err(err) => {
            app.bridge.connected = false;
            Ok(BridgeResponse::Error {
                message: err.to_string(),
            })
        }
    }
}

fn production_send_text(
    app: &mut App,
    target: SendTarget,
    text: String,
    append_newline: bool,
) -> Result<BridgeResponse> {
    let Some(instance_id) = app.bridge.instance_id.clone() else {
        return Ok(BridgeResponse::Error {
            message: "bridge is not initialized".to_string(),
        });
    };
    let mut client =
        crate::bridge::client::BridgeClient::new(crate::bridge::client::default_socket_path());
    match client.send_command(BridgeCommand::SendText {
        instance_id,
        target,
        text,
        append_newline,
    }) {
        Ok(response) => {
            app.bridge.connected = true;
            Ok(response)
        }
        Err(err) => {
            app.bridge.connected = false;
            Ok(BridgeResponse::Error {
                message: err.to_string(),
            })
        }
    }
}

fn generate_instance_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    format!("grove-{}-{nanos}", std::process::id())
}

fn emit_iterm2_user_var(name: &str, value: &str) -> Result<()> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
    let mut handle = stdout();
    write!(handle, "\u{1b}]1337;SetUserVar={name}={encoded}\u{7}")?;
    handle.flush()?;
    Ok(())
}

fn reconcile_iterm_preview_overlay(
    previous: Option<crate::app::PreviewOverlayPlacement>,
    current: Option<crate::app::PreviewOverlayPlacement>,
) -> (
    Option<crate::app::PreviewOverlayPlacement>,
    Option<crate::app::PreviewOverlayPlacement>,
) {
    let to_clear = match (previous, current) {
        (Some(previous), Some(current)) if previous == current => None,
        (Some(previous), _) => Some(previous),
        _ => None,
    };
    (to_clear, current)
}

fn clear_iterm_preview_overlay(
    placement: Option<crate::app::PreviewOverlayPlacement>,
) -> Result<()> {
    let Some(placement) = placement else {
        return Ok(());
    };
    if !crate::preview::mermaid::inline_images_supported()
        || placement.width_cells == 0
        || placement.height_lines == 0
    {
        return Ok(());
    }

    let blank = " ".repeat(placement.width_cells as usize);
    let mut handle = stdout();
    execute!(handle, SavePosition)?;
    for row in 0..placement.height_lines {
        execute!(handle, MoveTo(placement.x, placement.y.saturating_add(row)))?;
        write!(handle, "{blank}")?;
    }
    execute!(handle, RestorePosition)?;
    handle.flush()?;
    Ok(())
}

fn current_iterm_preview_overlay<B: Backend>(
    terminal: &Terminal<B>,
    app: &App,
) -> Result<Option<crate::app::PreviewOverlayPlacement>>
where
    B::Error: Display,
{
    if !crate::preview::mermaid::inline_images_supported() {
        return Ok(None);
    }

    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Ok(None);
    };
    let Some(_) = tab.active_inline_preview_image() else {
        return Ok(None);
    };
    let Some(cache) = tab.preview.render_cache.as_ref() else {
        return Ok(None);
    };
    let Some(slot) = cache.image_slot.as_ref() else {
        return Ok(None);
    };

    let areas = shell_areas(terminal, app)?;
    let content_left = areas.preview.x.saturating_add(1);
    let content_top = areas.preview.y.saturating_add(1);
    let content_width = areas.preview.width.saturating_sub(2);
    let content_height = areas.preview.height.saturating_sub(2);
    if content_width == 0 || content_height == 0 {
        return Ok(None);
    }

    let visible_start = tab.preview.scroll_row;
    let visible_end = visible_start.saturating_add(content_height as usize);
    let image_end = slot.start_line.saturating_add(slot.height_lines as usize);
    if slot.start_line < visible_start || image_end > visible_end {
        return Ok(None);
    }

    let image_row =
        content_top.saturating_add(slot.start_line.saturating_sub(visible_start) as u16);
    Ok(Some(crate::app::PreviewOverlayPlacement {
        x: content_left,
        y: image_row,
        width_cells: content_width,
        height_lines: slot.height_lines,
    }))
}

fn render_iterm_preview_overlay(
    placement: Option<crate::app::PreviewOverlayPlacement>,
    app: &App,
) -> Result<()> {
    let Some(placement) = placement else {
        return Ok(());
    };
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Ok(());
    };
    let Some((name, image_bytes)) = tab.active_inline_preview_image() else {
        return Ok(());
    };

    let mut handle = stdout();
    execute!(handle, SavePosition, MoveTo(placement.x, placement.y))?;
    emit_iterm_inline_image(
        &mut handle,
        name,
        image_bytes,
        placement.width_cells,
        placement.height_lines,
    )?;
    execute!(handle, RestorePosition)?;
    handle.flush()?;
    Ok(())
}

fn emit_iterm_inline_image<W: Write>(
    writer: &mut W,
    name: &str,
    png_bytes: &[u8],
    width_cells: u16,
    height_lines: u16,
) -> Result<()> {
    let encoded_name = base64::engine::general_purpose::STANDARD.encode(name.as_bytes());
    let encoded_bytes = base64::engine::general_purpose::STANDARD.encode(png_bytes);
    write!(
        writer,
        "\u{1b}]1337;File=name={encoded_name};size={};width={width_cells};height={height_lines};preserveAspectRatio=1;inline=1:{encoded_bytes}\u{7}",
        png_bytes.len()
    )?;
    Ok(())
}

fn suspend_terminal_for_command(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    command: &crate::open::LaunchCommand,
) -> Result<()> {
    restore_terminal_with_errors()?;

    let command_result = crate::open::launch_blocking(command);

    let reenter_result = (|| -> Result<()> {
        configure_terminal()?;
        *terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        Ok(())
    })();

    match (command_result, reenter_result) {
        (_, Err(err)) => Err(err),
        (Err(err), Ok(())) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn scroll_active_preview_up<B: Backend>(terminal: &Terminal<B>, app: &mut App) -> Result<bool>
where
    B::Error: Display,
{
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Ok(false);
    };
    let split_ratio = tab.split_ratio;
    let (preview_width, viewport_height) = preview_viewport_dimensions(terminal, app, split_ratio)?;
    Ok(app.scroll_active_preview_up(preview_width, viewport_height))
}

fn scroll_active_preview_down<B: Backend>(terminal: &Terminal<B>, app: &mut App) -> Result<bool>
where
    B::Error: Display,
{
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Ok(false);
    };
    let split_ratio = tab.split_ratio;
    let (preview_width, viewport_height) = preview_viewport_dimensions(terminal, app, split_ratio)?;
    Ok(app.scroll_active_preview_down(preview_width, viewport_height))
}

fn scroll_active_preview_page_up<B: Backend>(terminal: &Terminal<B>, app: &mut App) -> Result<bool>
where
    B::Error: Display,
{
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Ok(false);
    };
    let split_ratio = tab.split_ratio;
    let (preview_width, viewport_height) = preview_viewport_dimensions(terminal, app, split_ratio)?;
    Ok(app.scroll_active_preview_page_up(preview_width, viewport_height))
}

fn scroll_active_preview_page_down<B: Backend>(
    terminal: &Terminal<B>,
    app: &mut App,
) -> Result<bool>
where
    B::Error: Display,
{
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Ok(false);
    };
    let split_ratio = tab.split_ratio;
    let (preview_width, viewport_height) = preview_viewport_dimensions(terminal, app, split_ratio)?;
    Ok(app.scroll_active_preview_page_down(preview_width, viewport_height))
}

fn scroll_active_preview_end<B: Backend>(terminal: &Terminal<B>, app: &mut App) -> Result<bool>
where
    B::Error: Display,
{
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Ok(false);
    };
    let split_ratio = tab.split_ratio;
    let (preview_width, viewport_height) = preview_viewport_dimensions(terminal, app, split_ratio)?;
    Ok(app.scroll_active_preview_end(preview_width, viewport_height))
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        Self::enter_with_ops(configure_terminal, Terminal::new, restore_terminal)
    }

    fn enter_with_ops<Configure, BuildTerminal, Restore>(
        configure: Configure,
        build: BuildTerminal,
        restore: Restore,
    ) -> Result<Self>
    where
        Configure: FnOnce() -> Result<()>,
        BuildTerminal:
            FnOnce(CrosstermBackend<Stdout>) -> std::io::Result<Terminal<CrosstermBackend<Stdout>>>,
        Restore: FnOnce(),
    {
        configure()?;
        let backend = CrosstermBackend::new(stdout());
        match build(backend) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(err) => {
                restore();
                Err(err.into())
            }
        }
    }
}

fn configure_terminal() -> Result<()> {
    enable_raw_mode()?;
    if let Err(err) = execute!(stdout(), EnterAlternateScreen, EnableMouseCapture) {
        let _ = disable_raw_mode();
        return Err(err.into());
    }
    Ok(())
}

fn restore_terminal_with_errors() -> Result<()> {
    let mut out = stdout();
    restore_terminal_with_ops(
        || execute!(out, LeaveAlternateScreen, DisableMouseCapture, Show),
        disable_raw_mode,
    )
}

pub fn restore_terminal() {
    let _ = restore_terminal_with_errors();
}

fn restore_terminal_with_ops<RestoreScreen, DisableRaw>(
    restore_screen: RestoreScreen,
    disable_raw: DisableRaw,
) -> Result<()>
where
    RestoreScreen: FnOnce() -> std::io::Result<()>,
    DisableRaw: FnOnce() -> std::io::Result<()>,
{
    let mut first_error = None;

    if let Err(err) = restore_screen() {
        first_error = Some(err.into());
    }

    if let Err(err) = disable_raw()
        && first_error.is_none()
    {
        first_error = Some(err.into());
    }

    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

#[cfg(test)]
pub fn restore_terminal_fail_safe_probe(
    screen_restore_fails: bool,
    disable_raw_fails: bool,
) -> (bool, bool) {
    let mut disable_called = false;
    let result = restore_terminal_with_ops(
        || {
            if screen_restore_fails {
                Err(std::io::Error::other("screen restore failed"))
            } else {
                Ok(())
            }
        },
        || {
            disable_called = true;
            if disable_raw_fails {
                Err(std::io::Error::other("disable raw mode failed"))
            } else {
                Ok(())
            }
        },
    );
    (disable_called, result.is_err())
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        restore_terminal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::io::Cursor;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::app::{PathIndexStatus, TabState};
    use crate::git::backend::{
        BlameView, CommitSummary, DiffMode, GitBackend, GitError, GitPathStatus, RepoHandle,
        UnifiedDiff,
    };
    use crate::preview::model::ImageDisplay;
    use git2::{IndexAddOption, Repository, Signature};
    use ratatui::backend::TestBackend;

    #[test]
    fn restore_attempts_to_disable_raw_mode_after_screen_restore_failure() {
        let mut disable_called = false;
        let result = restore_terminal_with_ops(
            || Err(std::io::Error::other("screen restore failed")),
            || {
                disable_called = true;
                Ok(())
            },
        );

        assert!(disable_called);
        assert!(result.is_err());
    }

    #[test]
    fn terminal_constructor_failure_attempts_restore() {
        let mut restore_called = false;
        let result = TerminalSession::enter_with_ops(
            || Ok(()),
            |_backend| Err(std::io::Error::other("terminal constructor failed")),
            || {
                restore_called = true;
            },
        );

        assert!(result.is_err());
        assert!(restore_called);
    }

    #[test]
    fn git_refresh_success_clears_stale_git_status_message() {
        let mut app = App::default();
        app.status.severity = StatusSeverity::Error;
        app.status.message = "git refresh failed: boom".to_string();

        apply_git_refresh_result(&mut app, Ok(false));

        assert!(app.status.message.is_empty());
        assert_eq!(app.status.severity, StatusSeverity::Info);
    }

    #[test]
    fn render_shell_defers_git_refresh_while_path_index_is_building() {
        let root = make_temp_dir("grove-bootstrap-defer-git-refresh");
        let repo = Repository::init(&root).expect("repo should initialize");
        write_repo_file(&root, "tracked.txt", "before\n");
        commit_repo_all(&repo, "initial");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;

        let git_backend = LibgitBackend;
        app.refresh_active_git_state_with_backend(&git_backend)
            .expect("initial git refresh should succeed");
        assert!(app.tabs[0].git.initialized);
        assert!(!app.tabs[0].git.needs_refresh);

        app.tabs[0].path_index.status = PathIndexStatus::Building { indexed_paths: 128 };
        let counting_backend = CountingGitBackend::default();

        render_shell_once_with_git_backend(&mut terminal, &mut app, &counting_backend)
            .expect("render should succeed");

        assert_eq!(counting_backend.discover_calls(), 0);
        assert_eq!(counting_backend.status_map_calls(), 0);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn render_shell_skips_git_refresh_when_cached_state_is_clean_and_ready() {
        let root = make_temp_dir("grove-bootstrap-skip-clean-git-refresh");
        let repo = Repository::init(&root).expect("repo should initialize");
        write_repo_file(&root, "tracked.txt", "before\n");
        commit_repo_all(&repo, "initial");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;

        let git_backend = LibgitBackend;
        app.refresh_active_git_state_with_backend(&git_backend)
            .expect("initial git refresh should succeed");
        assert!(app.tabs[0].git.initialized);
        assert!(!app.tabs[0].git.needs_refresh);

        let counting_backend = CountingGitBackend::default();
        render_shell_once_with_git_backend(&mut terminal, &mut app, &counting_backend)
            .expect("render should succeed");

        assert_eq!(counting_backend.discover_calls(), 0);
        assert_eq!(counting_backend.status_map_calls(), 0);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn reconcile_iterm_preview_overlay_clears_a_stale_mermaid_image_when_the_next_frame_has_none() {
        let previous = crate::app::PreviewOverlayPlacement {
            x: 12,
            y: 8,
            width_cells: 40,
            height_lines: 16,
        };

        let (to_clear, next) = reconcile_iterm_preview_overlay(Some(previous), None);

        assert_eq!(to_clear, Some(previous));
        assert_eq!(next, None);
    }

    #[test]
    fn poll_runtime_background_work_applies_ready_image_preview_results() {
        let root = make_temp_dir("grove-bootstrap-image-background-poll");
        write_tiny_png(&root.join("pixel.png"));

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;
        assert!(app.tabs[0].tree.select_rel_path(Path::new("pixel.png")));
        assert!(app.refresh_active_preview());
        assert_eq!(
            app.tabs[0]
                .preview
                .payload
                .image
                .as_ref()
                .map(|image| image.display),
            Some(ImageDisplay::Pending)
        );

        let mut watcher = NoopWatcherService;
        for _ in 0..8 {
            poll_runtime_background_work(&mut terminal, &mut app, &mut watcher)
                .expect("background work poll should succeed");
            if app.tabs[0]
                .preview
                .payload
                .image
                .as_ref()
                .is_some_and(|image| image.display == ImageDisplay::Summary)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let image = app.tabs[0]
            .preview
            .payload
            .image
            .as_ref()
            .expect("image preview payload should still exist");
        assert!(
            image.display != ImageDisplay::Pending,
            "background work polling should advance image preview out of the pending state"
        );
        assert_ne!(image.status, "Image preview pending");

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn selected_editor_open_request_prefers_search_result_hit_path_and_line() {
        let root = make_temp_dir("grove-bootstrap-editor-search-hit");
        fs::write(root.join("alpha.txt"), "alpha").expect("alpha should exist");
        fs::write(root.join("beta.txt"), "beta").expect("beta should exist");

        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].mode = ContextMode::SearchResults;
        app.tabs[0].content_search.payload = crate::preview::model::SearchPayload {
            query: "beta".to_string(),
            hits: vec![crate::preview::model::SearchHit {
                path: "beta.txt".to_string(),
                line: 17,
                excerpt: "beta".to_string(),
            }],
        };
        app.tabs[0].content_search.selected_hit_index = Some(0);

        let request =
            selected_editor_open_request(&app).expect("search hit should build a request");
        assert_eq!(
            request
                .path
                .canonicalize()
                .expect("request path should resolve"),
            root.join("beta.txt")
                .canonicalize()
                .expect("expected path should resolve")
        );
        assert_eq!(request.line, 17);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn selected_editor_open_request_uses_preview_line_hint_when_available() {
        let root = make_temp_dir("grove-bootstrap-editor-preview-line");
        fs::write(root.join("alpha.txt"), "alpha").expect("alpha should exist");

        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;
        assert!(app.tabs[0].tree.select_rel_path(Path::new("alpha.txt")));
        app.tabs[0].mode = ContextMode::Diff;
        app.tabs[0].preview.editor_line_hint = Some(23);

        let request =
            selected_editor_open_request(&app).expect("selected file should build a request");
        assert_eq!(
            request
                .path
                .canonicalize()
                .expect("request path should resolve"),
            root.join("alpha.txt")
                .canonicalize()
                .expect("expected path should resolve")
        );
        assert_eq!(request.line, 23);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn selected_editor_open_request_uses_current_preview_scroll_row() {
        let root = make_temp_dir("grove-bootstrap-editor-preview-scroll");
        fs::write(root.join("alpha.txt"), "alpha").expect("alpha should exist");

        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;
        assert!(app.tabs[0].tree.select_rel_path(Path::new("alpha.txt")));
        app.tabs[0].mode = ContextMode::Preview;
        app.tabs[0].preview.editor_line_hint = Some(1);
        app.tabs[0].preview.scroll_row = 24;

        let request =
            selected_editor_open_request(&app).expect("selected file should build a request");
        assert_eq!(request.line, 25);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn confirmed_overwrite_restores_destination_when_operation_fails() {
        let root = make_temp_dir("grove-bootstrap-overwrite-restore");
        fs::write(root.join("dest.txt"), "dest").expect("dest should exist");

        let result = execute_confirmed_overwrite(
            &root,
            PromptDialogIntent::Move,
            Path::new("missing.txt"),
            Path::new("dest.txt"),
        );

        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(root.join("dest.txt")).expect("dest should be restored"),
            "dest"
        );
        let backup_entries = fs::read_dir(&root)
            .expect("root should read")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".grove-replace-backup-"))
            .collect::<Vec<_>>();
        assert!(
            backup_entries.is_empty(),
            "failed overwrite should restore the destination without leaving backup files behind"
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn copy_text_failure_sets_error_status_without_exiting() {
        let mut app = App::default();

        let result = copy_text_with(
            &mut app,
            "alpha.txt",
            "copied alpha.txt".to_string(),
            |_| Err(std::io::Error::other("clipboard unavailable").into()),
        );

        assert!(result.expect("copy helper should not bubble clipboard errors"));
        assert_eq!(app.status.severity, StatusSeverity::Error);
        assert_eq!(app.status.message, "copy failed: clipboard unavailable");
    }

    #[test]
    fn copy_active_preview_selection_uses_current_cursor_line_when_no_range_exists() {
        let root = make_temp_dir("grove-bootstrap-copy-preview-line");
        fs::write(root.join("notes.txt"), "line 00\nline 01\nline 02\n")
            .expect("should create notes file");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App {
            focus: Focus::Preview,
            ..App::default()
        };
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;
        move_selection_down(&mut app);
        render_shell_once(&mut terminal, &mut app).expect("preview render should work");

        let line_index = app.tabs[0]
            .preview
            .render_cache
            .as_ref()
            .and_then(|cache| {
                cache
                    .lines
                    .iter()
                    .position(|line| line.to_string().contains("line 00"))
            })
            .expect("line 00 should exist in preview");
        app.tabs[0].preview.cursor_line = line_index;

        let mut copied = None;
        let result = copy_active_preview_selection_with(&terminal, &mut app, |text| {
            copied = Some(text.to_string());
            Ok(())
        });

        assert!(result.expect("copy helper should succeed"));
        assert_eq!(copied.as_deref(), Some("line 00"));

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn copy_active_preview_selection_uses_selected_rendered_range() {
        let root = make_temp_dir("grove-bootstrap-copy-preview-range");
        fs::write(root.join("notes.txt"), "line 00\nline 01\nline 02\n")
            .expect("should create notes file");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App {
            focus: Focus::Preview,
            ..App::default()
        };
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;
        move_selection_down(&mut app);
        render_shell_once(&mut terminal, &mut app).expect("preview render should work");

        let (line_zero, line_one) = {
            let cache = app.tabs[0]
                .preview
                .render_cache
                .as_ref()
                .expect("preview cache should exist");
            let line_zero = cache
                .lines
                .iter()
                .position(|line| line.to_string().contains("line 00"))
                .expect("line 00 should exist");
            let line_one = cache
                .lines
                .iter()
                .position(|line| line.to_string().contains("line 01"))
                .expect("line 01 should exist");
            (line_zero, line_one)
        };
        app.tabs[0].preview.cursor_line = line_one;
        app.tabs[0].preview.selected_line_start = Some(line_zero);
        app.tabs[0].preview.selected_line_end = Some(line_one);

        let mut copied = None;
        let result = copy_active_preview_selection_with(&terminal, &mut app, |text| {
            copied = Some(text.to_string());
            Ok(())
        });

        assert!(result.expect("copy helper should succeed"));
        assert_eq!(copied.as_deref(), Some("line 00\nline 01"));

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    fn mark_bridge_initialized_for_test(app: &mut App) -> Result<()> {
        app.bridge.connected = true;
        app.bridge.instance_id = Some("instance-1".to_string());
        Ok(())
    }

    #[derive(Default)]
    struct CountingGitBackend {
        discover_calls: AtomicUsize,
        status_map_calls: AtomicUsize,
    }

    impl CountingGitBackend {
        fn discover_calls(&self) -> usize {
            self.discover_calls.load(Ordering::Relaxed)
        }

        fn status_map_calls(&self) -> usize {
            self.status_map_calls.load(Ordering::Relaxed)
        }
    }

    impl GitBackend for CountingGitBackend {
        fn discover_repo(&self, _root: &Path) -> std::result::Result<Option<RepoHandle>, GitError> {
            self.discover_calls.fetch_add(1, Ordering::Relaxed);
            Ok(None)
        }

        fn status_map(
            &self,
            _repo: &RepoHandle,
        ) -> std::result::Result<HashMap<std::path::PathBuf, GitPathStatus>, GitError> {
            self.status_map_calls.fetch_add(1, Ordering::Relaxed);
            Ok(HashMap::new())
        }

        fn diff_for_path(
            &self,
            _repo: &RepoHandle,
            _rel_path: &Path,
            _mode: DiffMode,
        ) -> std::result::Result<UnifiedDiff, GitError> {
            unreachable!("diff_for_path should not be called in this test")
        }

        fn blame_for_path(
            &self,
            _repo: &RepoHandle,
            _rel_path: &Path,
        ) -> std::result::Result<BlameView, GitError> {
            unreachable!("blame_for_path should not be called in this test")
        }

        fn history_for_path(
            &self,
            _repo: &RepoHandle,
            _rel_path: &Path,
            _limit: usize,
        ) -> std::result::Result<Vec<CommitSummary>, GitError> {
            unreachable!("history_for_path should not be called in this test")
        }

        fn stage_path(
            &self,
            _repo: &RepoHandle,
            _rel_path: &Path,
        ) -> std::result::Result<(), GitError> {
            unreachable!("stage_path should not be called in this test")
        }

        fn unstage_path(
            &self,
            _repo: &RepoHandle,
            _rel_path: &Path,
        ) -> std::result::Result<(), GitError> {
            unreachable!("unstage_path should not be called in this test")
        }
    }

    #[test]
    fn runtime_initialization_runs_bridge_hook_before_first_render() {
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        let mut watcher = NoopWatcherService;

        let result = initialize_runtime_with_hooks_and_watcher(
            &mut terminal,
            &mut app,
            &RuntimeActionHooks {
                open_in_editor: noop_runtime_action::<TestBackend>,
                open_externally: noop_runtime_action::<TestBackend>,
                reveal_in_file_manager: noop_runtime_action::<TestBackend>,
                initialize_bridge: mark_bridge_initialized_for_test,
                list_sessions: noop_list_sessions,
                send_text: noop_send_text,
                set_role: noop_set_role,
            },
            &mut watcher,
        );

        assert!(result.is_ok());
        assert!(app.bridge.connected);
        assert_eq!(app.bridge.instance_id.as_deref(), Some("instance-1"));

        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("bridge: online"));
    }

    #[test]
    fn mouse_scroll_event_maps_to_runtime_wheel_input() {
        let event = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 42,
            row: 7,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(
            runtime_input_from_event(event),
            Some(RuntimeInput::WheelDown { column: 42, row: 7 })
        );
    }

    #[test]
    fn mouse_left_click_event_maps_to_runtime_click_input() {
        let event = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 42,
            row: 7,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(
            runtime_input_from_event(event),
            Some(RuntimeInput::PreviewClick { column: 42, row: 7 })
        );
    }

    #[test]
    fn shift_down_key_maps_to_preview_selection_extension() {
        let event = KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT);

        assert_eq!(
            runtime_input_from_key_event(event),
            Some(RuntimeInput::ExtendPreviewSelectionDown)
        );
    }

    #[test]
    fn shift_up_key_maps_to_preview_selection_extension() {
        let event = KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT);

        assert_eq!(
            runtime_input_from_key_event(event),
            Some(RuntimeInput::ExtendPreviewSelectionUp)
        );
    }

    #[test]
    fn ctrl_t_key_maps_to_open_selected_directory_as_root_tab() {
        let event = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL);

        assert_eq!(
            runtime_input_from_key_event(event),
            Some(RuntimeInput::OpenSelectedDirectoryAsRootTab)
        );
    }

    #[test]
    fn ctrl_r_key_maps_to_open_root_picker() {
        let event = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);

        assert_eq!(
            runtime_input_from_key_event(event),
            Some(RuntimeInput::OpenRootPicker)
        );
    }

    #[test]
    fn ctrl_t_on_selected_directory_opens_a_new_root_tab() {
        let root = make_temp_dir("grove-bootstrap-ctrl-t-directory");
        fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
        fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;
        assert!(
            app.tabs[0].tree.select_rel_path(Path::new("alpha")),
            "directory selection should exist"
        );

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::OpenSelectedDirectoryAsRootTab,
            &RuntimeActionHooks::noops(),
        )
        .expect("ctrl+t input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active_tab, 1);
        assert_eq!(
            fs::canonicalize(&app.tabs[1].root).expect("opened tab root should canonicalize"),
            fs::canonicalize(root.join("alpha")).expect("expected root should canonicalize")
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn ctrl_r_opens_root_picker_dialog() {
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::OpenRootPicker,
            &RuntimeActionHooks::noops(),
        )
        .expect("ctrl+r input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert!(matches!(
            app.dialog_state(),
            Some(DialogState::DirectoryPicker(_))
        ));
        assert_eq!(app.focus, Focus::Dialog);
    }

    #[test]
    fn tree_multi_select_mode_toggles_on_m() {
        let root = make_temp_dir("grove-bootstrap-multi-toggle");
        fs::write(root.join("note.txt"), "hello\n").expect("should create note.txt");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Character('m'),
            &RuntimeActionHooks::noops(),
        )
        .expect("m input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert!(app.active_multi_select_mode());

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Character('m'),
            &RuntimeActionHooks::noops(),
        )
        .expect("second m input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert!(!app.active_multi_select_mode());

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn tree_space_toggles_selected_path_only_while_multi_select_mode_is_active() {
        let root = make_temp_dir("grove-bootstrap-multi-space");
        fs::write(root.join("note.txt"), "hello\n").expect("should create note.txt");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        assert!(app.tabs[0].tree.select_rel_path(Path::new("note.txt")));

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Character(' '),
            &RuntimeActionHooks::noops(),
        )
        .expect("space input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.active_multi_select_count(), 0);

        let _ = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Character('m'),
            &RuntimeActionHooks::noops(),
        )
        .expect("m input should be handled");

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Character(' '),
            &RuntimeActionHooks::noops(),
        )
        .expect("space input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.active_multi_select_count(), 1);
        assert_eq!(
            app.active_multi_selected_paths(),
            vec![PathBuf::from("note.txt")]
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn tree_x_clears_multi_select_batch() {
        let root = make_temp_dir("grove-bootstrap-multi-clear");
        fs::write(root.join("note.txt"), "hello\n").expect("should create note.txt");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        assert!(app.tabs[0].tree.select_rel_path(Path::new("note.txt")));
        assert!(app.toggle_active_multi_select_mode());
        assert!(app.toggle_selected_path_in_active_multi_select());
        assert_eq!(app.active_multi_select_count(), 1);

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Character('x'),
            &RuntimeActionHooks::noops(),
        )
        .expect("x input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.active_multi_select_count(), 0);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn tree_escape_exits_multi_select_mode_without_clearing_batch() {
        let root = make_temp_dir("grove-bootstrap-multi-escape");
        fs::write(root.join("note.txt"), "hello\n").expect("should create note.txt");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        assert!(app.tabs[0].tree.select_rel_path(Path::new("note.txt")));
        assert!(app.toggle_active_multi_select_mode());
        assert!(app.toggle_selected_path_in_active_multi_select());
        assert_eq!(app.active_multi_select_count(), 1);

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Escape,
            &RuntimeActionHooks::noops(),
        )
        .expect("escape input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert!(!app.active_multi_select_mode());
        assert_eq!(
            app.active_sendable_rel_paths(),
            vec![PathBuf::from("note.txt")]
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn root_picker_escape_closes_and_restores_focus() {
        let root = make_temp_dir("grove-bootstrap-root-picker-escape");
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App {
            focus: Focus::Preview,
            ..App::default()
        };
        app.open_add_root_directory_picker_at(root.clone())
            .expect("picker should open");

        let outcome = handle_dialog_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Escape,
            &RuntimeActionHooks::noops(),
        )
        .expect("escape input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert!(app.dialog_state().is_none());
        assert_eq!(app.focus, Focus::Preview);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn root_picker_right_enters_selected_directory_and_left_moves_to_parent() {
        let root = make_temp_dir("grove-bootstrap-root-picker-navigation");
        fs::create_dir_all(root.join("alpha").join("nested")).expect("should create nested dir");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.open_add_root_directory_picker_at(root.clone())
            .expect("picker should open");
        let alpha_index = app
            .directory_picker_state()
            .expect("picker state should exist")
            .entries
            .iter()
            .position(|entry| entry.label == "alpha")
            .expect("alpha entry should exist");
        assert!(app.set_directory_picker_selection_by_index(alpha_index));

        let outcome = handle_dialog_input(
            &mut terminal,
            &mut app,
            RuntimeInput::MoveRight,
            &RuntimeActionHooks::noops(),
        )
        .expect("right input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(
            app.directory_picker_state()
                .expect("picker state should exist")
                .current_dir,
            root.join("alpha")
                .canonicalize()
                .expect("alpha should canonicalize")
        );

        let outcome = handle_dialog_input(
            &mut terminal,
            &mut app,
            RuntimeInput::MoveLeft,
            &RuntimeActionHooks::noops(),
        )
        .expect("left input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(
            app.directory_picker_state()
                .expect("picker state should exist")
                .current_dir,
            root.canonicalize().expect("root should canonicalize")
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn root_picker_enter_pins_and_opens_selected_directory_root() {
        let root = make_temp_dir("grove-bootstrap-root-picker-enter");
        let config_path = root.join("config.toml");
        fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.set_config_path(config_path);
        app.open_add_root_directory_picker_at(root.clone())
            .expect("picker should open");
        let alpha_index = app
            .directory_picker_state()
            .expect("picker state should exist")
            .entries
            .iter()
            .position(|entry| entry.label == "alpha")
            .expect("alpha entry should exist");
        assert!(app.set_directory_picker_selection_by_index(alpha_index));

        let outcome = handle_dialog_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Enter,
            &RuntimeActionHooks::noops(),
        )
        .expect("enter input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert!(app.dialog_state().is_none());
        assert_eq!(app.focus, Focus::Tree);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active_tab, 1);
        assert_eq!(
            app.bookmark_paths().len(),
            1,
            "selected root should be pinned on commit"
        );
        assert_eq!(
            fs::canonicalize(&app.tabs[1].root).expect("opened root should canonicalize"),
            fs::canonicalize(root.join("alpha")).expect("alpha should canonicalize")
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn root_picker_enter_reuses_existing_pinned_open_root() {
        let root = make_temp_dir("grove-bootstrap-root-picker-dedupe");
        fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        assert!(app.open_or_activate_tab(root.join("alpha")));
        app.config
            .bookmarks
            .pins
            .push(fs::canonicalize(root.join("alpha")).expect("alpha root should canonicalize"));
        app.active_tab = 0;
        app.open_add_root_directory_picker_at(root.clone())
            .expect("picker should open");
        let alpha_index = app
            .directory_picker_state()
            .expect("picker state should exist")
            .entries
            .iter()
            .position(|entry| entry.label == "alpha")
            .expect("alpha entry should exist");
        assert!(app.set_directory_picker_selection_by_index(alpha_index));

        let outcome = handle_dialog_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Enter,
            &RuntimeActionHooks::noops(),
        )
        .expect("enter input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.tabs.len(), 2, "existing root should not duplicate");
        assert_eq!(app.active_tab, 1, "existing root tab should be activated");
        assert_eq!(
            app.bookmark_paths().len(),
            1,
            "existing pin should be reused"
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn root_picker_enter_uses_resolved_label_for_parent_row_commit() {
        let root = make_temp_dir("grove-bootstrap-root-picker-parent-label");
        let child = root.join("alpha");
        let grandchild = child.join("nested");
        let config_path = root.join("config.toml");
        fs::create_dir_all(&grandchild).expect("should create nested dir");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.set_config_path(config_path);
        app.open_add_root_directory_picker_at(grandchild)
            .expect("picker should open");
        let parent_index = app
            .directory_picker_state()
            .expect("picker state should exist")
            .entries
            .iter()
            .position(|entry| entry.label == "..")
            .expect("parent entry should exist");
        assert!(app.set_directory_picker_selection_by_index(parent_index));

        let outcome = handle_dialog_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Enter,
            &RuntimeActionHooks::noops(),
        )
        .expect("enter input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.status.severity, StatusSeverity::Success);
        assert!(
            !app.status.message.ends_with(" .."),
            "success message should use the resolved root label"
        );
        assert!(
            app.status
                .message
                .contains(child.file_name().and_then(|name| name.to_str()).unwrap()),
            "success message should reference the resolved target root"
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn root_picker_commit_save_failure_stays_in_dialog_and_rolls_back_pin() {
        let root = make_temp_dir("grove-bootstrap-root-picker-save-failure");
        let child = root.join("alpha");
        let config_path = root.join("config-dir");
        fs::create_dir_all(&child).expect("should create alpha dir");
        fs::create_dir_all(&config_path).expect("config path directory should exist");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.set_config_path(config_path);
        app.open_add_root_directory_picker_at(root.clone())
            .expect("picker should open");
        let child_index = app
            .directory_picker_state()
            .expect("picker state should exist")
            .entries
            .iter()
            .position(|entry| entry.label == "alpha")
            .expect("alpha entry should exist");
        assert!(app.set_directory_picker_selection_by_index(child_index));

        let outcome = handle_dialog_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Enter,
            &RuntimeActionHooks::noops(),
        )
        .expect("save failure should stay inside the dialog");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert!(matches!(
            app.dialog_state(),
            Some(DialogState::DirectoryPicker(_))
        ));
        assert_eq!(app.focus, Focus::Dialog);
        assert!(
            app.bookmark_paths().is_empty(),
            "failed save should roll back the pin"
        );
        assert_eq!(
            app.tabs.len(),
            1,
            "failed save should not open a new root tab"
        );
        assert_eq!(app.status.severity, StatusSeverity::Error);
        assert!(
            app.status.message.contains("could not add root alpha"),
            "status message should report the save failure"
        );
        assert!(
            app.directory_picker_state()
                .and_then(|picker| picker.error_message.as_deref())
                .is_some_and(|message| message.contains("could not add root alpha")),
            "the picker should surface the save failure inline"
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn ctrl_t_on_selected_file_warns_and_does_not_open_tab() {
        let root = make_temp_dir("grove-bootstrap-ctrl-t-file");
        fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha.txt");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;
        assert!(
            app.tabs[0].tree.select_rel_path(Path::new("alpha.txt")),
            "file selection should exist"
        );

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::OpenSelectedDirectoryAsRootTab,
            &RuntimeActionHooks::noops(),
        )
        .expect("ctrl+t input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.status.severity, StatusSeverity::Warning);
        assert_eq!(
            app.status.message,
            "select a directory to open as a root tab"
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn ctrl_t_on_active_root_does_not_duplicate_tab() {
        let root = make_temp_dir("grove-bootstrap-ctrl-t-root");
        fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha.txt");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::OpenSelectedDirectoryAsRootTab,
            &RuntimeActionHooks::noops(),
        )
        .expect("ctrl+t input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.status.severity, StatusSeverity::Info);
        assert!(app.status.message.is_empty());

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn b_toggles_the_active_root_bookmark_and_persists_it() {
        let root = make_temp_dir("grove-bootstrap-bookmark-toggle");
        let config_path = root.join("config.toml");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.set_config_path(config_path.clone());

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Character('b'),
            &RuntimeActionHooks::noops(),
        )
        .expect("bookmark toggle should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.bookmark_paths().len(), 1);
        assert_eq!(
            fs::canonicalize(&app.bookmark_paths()[0]).expect("bookmark path should canonicalize"),
            fs::canonicalize(&root).expect("root should canonicalize")
        );
        assert_eq!(app.status.severity, StatusSeverity::Success);
        assert_eq!(app.status.message, "active root pinned");
        let saved = fs::read_to_string(&config_path).expect("bookmark toggle should persist");
        assert!(saved.contains(&root.display().to_string()));

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Character('b'),
            &RuntimeActionHooks::noops(),
        )
        .expect("bookmark toggle should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert!(app.bookmark_paths().is_empty());
        assert_eq!(app.status.severity, StatusSeverity::Success);
        assert_eq!(app.status.message, "active root unpinned");

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn b_remains_literal_input_inside_the_path_filter() {
        let root = make_temp_dir("grove-bootstrap-bookmark-filter-literal");
        fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha.txt");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        assert!(app.focus_path_filter(), "path filter should focus");

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Character('b'),
            &RuntimeActionHooks::noops(),
        )
        .expect("literal path-filter input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert!(app.bookmark_paths().is_empty());
        assert_eq!(app.tabs[0].path_filter.query, "b");

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn wheel_down_over_tree_moves_selection() {
        let root = make_temp_dir("grove-bootstrap-wheel-tree");
        fs::write(root.join("one.txt"), "1").expect("should create one.txt");
        fs::write(root.join("two.txt"), "2").expect("should create two.txt");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;

        render_shell_once(&mut terminal, &mut app).expect("initial render should work");
        let areas = shell_areas(&terminal, &app).expect("shell areas should resolve");

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::WheelDown {
                column: areas.tree.x + 1,
                row: areas.tree.y + 1,
            },
            &RuntimeActionHooks::noops(),
        )
        .expect("wheel input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.tabs[0].tree.selected_row, 1);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn wheel_down_over_preview_scrolls_preview_content() {
        let root = make_temp_dir("grove-bootstrap-wheel-preview");
        let body = (0..40)
            .map(|idx| format!("line {idx:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(root.join("notes.txt"), body).expect("should create notes file");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;

        move_selection_down(&mut app);
        render_shell_once(&mut terminal, &mut app).expect("preview render should work");
        let areas = shell_areas(&terminal, &app).expect("shell areas should resolve");

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::WheelDown {
                column: areas.preview.x + 1,
                row: areas.preview.y + 1,
            },
            &RuntimeActionHooks::noops(),
        )
        .expect("wheel input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.tabs[0].preview.scroll_row, 1);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn click_over_preview_sets_cursor_line_and_focus() {
        let root = make_temp_dir("grove-bootstrap-click-preview-cursor");
        let body = (0..12)
            .map(|idx| format!("line {idx:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(root.join("notes.txt"), body).expect("should create notes file");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;
        move_selection_down(&mut app);

        render_shell_once(&mut terminal, &mut app).expect("preview render should work");
        let areas = shell_areas(&terminal, &app).expect("shell areas should resolve");

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::PreviewClick {
                column: areas.preview.x + 2,
                row: areas.preview.y + 4,
            },
            &RuntimeActionHooks::noops(),
        )
        .expect("click input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.focus, Focus::Preview);
        assert_eq!(app.tabs[0].preview.cursor_line, 3);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn shift_down_in_preview_extends_line_selection() {
        let root = make_temp_dir("grove-bootstrap-preview-shift-down");
        let body = (0..12)
            .map(|idx| format!("line {idx:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(root.join("notes.txt"), body).expect("should create notes file");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App {
            focus: Focus::Preview,
            ..App::default()
        };
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;
        move_selection_down(&mut app);
        render_shell_once(&mut terminal, &mut app).expect("preview render should work");
        app.tabs[0].preview.cursor_line = 3;

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::ExtendPreviewSelectionDown,
            &RuntimeActionHooks::noops(),
        )
        .expect("selection extension input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.tabs[0].preview.cursor_line, 4);
        assert_eq!(app.tabs[0].preview.preview_selection_range(), Some((3, 4)));

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn escape_in_preview_clears_selection_before_other_behavior() {
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App {
            focus: Focus::Preview,
            ..App::default()
        };
        app.tabs[0].preview.cursor_line = 4;
        app.tabs[0].preview.selected_line_start = Some(4);
        app.tabs[0].preview.selected_line_end = Some(6);

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Escape,
            &RuntimeActionHooks::noops(),
        )
        .expect("escape input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert_eq!(app.focus, Focus::Preview);
        assert_eq!(app.tabs[0].preview.preview_selection_range(), None);
    }

    #[test]
    fn copy_preview_selection_uses_selected_rendered_lines() {
        let root = make_temp_dir("grove-bootstrap-copy-preview-selection");
        let body = (0..6)
            .map(|idx| format!("line {idx:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(root.join("notes.txt"), body).expect("should create notes file");

        let backend = TestBackend::new(120, 36);
        let terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App {
            focus: Focus::Preview,
            ..App::default()
        };
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;
        move_selection_down(&mut app);
        let _ = app.refresh_active_preview();
        let _ = app.refresh_active_preview_render_cache(60);
        app.tabs[0].preview.cursor_line = 5;
        app.tabs[0].preview.selected_line_start = Some(4);
        app.tabs[0].preview.selected_line_end = Some(5);

        let mut copied = None;
        let changed = copy_active_preview_selection_with(&terminal, &mut app, |text| {
            copied = Some(text.to_string());
            Ok(())
        })
        .expect("preview copy should not fail");

        assert!(changed);
        assert_eq!(copied.as_deref(), Some("line 04\nline 05"));
        assert_eq!(app.status.severity, StatusSeverity::Success);
        assert_eq!(app.status.message, "copied preview selection");

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn backspace_outside_filter_toggles_hidden_visibility() {
        let root = make_temp_dir("grove-bootstrap-backspace-toggle-hidden");
        let config_root = make_temp_dir("grove-bootstrap-backspace-toggle-hidden-config");
        let config_path = config_root.join("grove-config.toml");
        fs::write(root.join(".env"), "secret").expect("should create hidden file");
        fs::write(root.join("visible.txt"), "visible").expect("should create visible file");

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut app = App::default();
        app.set_config_path(config_path);
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = PathIndexStatus::Ready;

        render_shell_once(&mut terminal, &mut app).expect("initial render should work");
        assert!(
            !app.tabs[0]
                .tree
                .visible_rows
                .iter()
                .filter_map(|row| app.tabs[0].tree.node(row.node_id))
                .any(|node| node.rel_path == std::path::PathBuf::from(".env")),
            "hidden file should start hidden"
        );

        let outcome = handle_runtime_input(
            &mut terminal,
            &mut app,
            RuntimeInput::Backspace,
            &RuntimeActionHooks::noops(),
        )
        .expect("backspace input should be handled");

        assert_eq!(outcome, RuntimeControlFlow::Continue);
        assert!(
            app.tabs[0]
                .tree
                .visible_rows
                .iter()
                .filter_map(|row| app.tabs[0].tree.node(row.node_id))
                .any(|node| node.rel_path == std::path::PathBuf::from(".env")),
            "backspace outside the filter should reveal hidden files"
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("{prefix}-{pid}-{nanos}"));
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    fn write_repo_file(root: &Path, rel_path: &str, contents: &str) {
        let path = root.join(rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent directory should be created");
        }
        fs::write(path, contents).expect("repo file should be written");
    }

    fn write_tiny_png(path: &Path) {
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .expect("tiny png fixture should encode");
        fs::write(path, encoded.into_inner()).expect("tiny png fixture should be written");
    }

    fn commit_repo_all(repo: &Repository, message: &str) {
        let mut index = repo.index().expect("index should open");
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .expect("repo contents should stage");
        index.write().expect("index should flush");

        let tree_oid = index.write_tree().expect("tree should write");
        let tree = repo.find_tree(tree_oid).expect("tree should load");
        let signature = Signature::now("Grove Tests", "grove-tests@example.com")
            .expect("signature should build");

        let parent = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| repo.find_commit(oid).expect("parent commit should load"));

        match parent.as_ref() {
            Some(parent_commit) => {
                repo.commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    message,
                    &tree,
                    &[parent_commit],
                )
                .expect("commit should succeed");
            }
            None => {
                repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
                    .expect("initial commit should succeed");
            }
        }
    }
}

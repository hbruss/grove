use grove::action::{Action, ActionDescriptor, CommandSurfaceContract, RootWorkflowContract};
use grove::app::{
    App, BridgeState, ContentSearchState, ContentSearchStatus, GitRepoSummary, GitTabState,
    PathFilterState, PreviewState,
};
use grove::bridge::protocol::{
    BridgeCommand, BridgeRequestEnvelope, BridgeResponse, BridgeResponseEnvelope, ResolutionSource,
    SendTarget, SessionLocationHint, SessionSummary, TargetResolution, TargetRole,
};
use grove::config::{Config, EditorMode, MultilineTransport, SortMode};
use grove::event::{AppEvent, TimerKind};
use grove::git::backend::{GitChange, GitPathStatus, RepoHandle};
use grove::preview::model::{
    GitGeneration, GitPayload, ImageDisplay, ImagePreview, MermaidDisplay, MermaidPreview,
    MermaidSource, MermaidSourceKind, PreviewGeneration, PreviewHeader, PreviewMetadataItem,
    PreviewPayload, PreviewPresentation, SearchGeneration, SearchPayload,
};
use grove::search::{SearchRequest, SearchResponse, SearchScope};
use grove::state::{
    CommandPaletteState, ContextMenuState, ContextMode, DialogState, DirectoryPickerEntry,
    DirectoryPickerEntryMode, DirectoryPickerIntent, DirectoryPickerState, Focus, MultiSelectState,
    OverlayState, PersistedState, PersistedTabState, TargetPickerSelection, TargetPickerState,
};
use grove::tree::model::{DirLoadState, IndexState, NodeId, NodeKind, TreeState};
use grove::watcher::{RefreshPlan, WatcherRuntime, WatcherService};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

#[test]
fn core_contracts_compile_and_serialize() {
    let config = Config::default();
    let serialized = toml::to_string(&config).expect("config should serialize");
    let parsed: Config = toml::from_str(&serialized).expect("config should parse");
    assert!(matches!(parsed.general.sort_by, SortMode::Name));
    assert!(matches!(
        parsed.injection.ai.multiline_transport,
        MultilineTransport::Typed
    ));
    assert!(matches!(
        parsed.injection.editor.mode,
        EditorMode::LocalProcess
    ));
    assert_eq!(parsed.preview.mermaid_command, None);
    assert_eq!(parsed.preview.mermaid_render_timeout_ms, 5_000);
    assert_eq!(parsed.preview.image_preview_max_bytes, 20 * 1_048_576);
    assert_eq!(parsed.preview.image_preview_max_pixels, 16_777_216);
    assert_eq!(parsed.watcher.debounce_ms, 125);
    assert_eq!(parsed.watcher.highlight_changes_ms, 5_000);
    assert!(parsed.watcher.poll_fallback);

    let request = BridgeRequestEnvelope {
        request_id: "req-1".to_string(),
        command: BridgeCommand::SendText {
            instance_id: "instance".to_string(),
            target: SendTarget::Role(TargetRole::Ai),
            text: "@ref src/lib.rs".to_string(),
            append_newline: false,
        },
    };
    let request_json = serde_json::to_string(&request).expect("bridge request should serialize");
    assert!(request_json.contains("\"request_id\":\"req-1\""));

    let response = BridgeResponseEnvelope {
        request_id: "req-1".to_string(),
        response: BridgeResponse::Error {
            message: "missing target".to_string(),
        },
    };
    let response_json = serde_json::to_string(&response).expect("bridge response should serialize");
    assert!(response_json.contains("\"request_id\":\"req-1\""));

    let session = SessionSummary {
        session_id: "session-1".to_string(),
        title: "Claude Code".to_string(),
        role: Some(TargetRole::Ai),
        job_name: Some("claude-code".to_string()),
        command_line: Some("claude".to_string()),
        cwd: Some("/repo".to_string()),
        location_hint: Some(SessionLocationHint {
            window_id: Some("window-1".to_string()),
            tab_id: Some("tab-1".to_string()),
            window_title: Some("Workspace".to_string()),
            tab_title: Some("AI".to_string()),
        }),
    };
    let session_json = serde_json::to_string(&session).expect("session summary should serialize");
    assert!(session_json.contains("\"tab_id\":\"tab-1\""));

    let resolution = TargetResolution {
        ai_target_session_id: Some("session-1".to_string()),
        editor_target_session_id: None,
        source: ResolutionSource::SameTab,
    };
    let resolution_json =
        serde_json::to_string(&resolution).expect("target resolution should serialize");
    assert!(resolution_json.contains("\"source\":\"same_tab\""));

    let manual_selection = BridgeResponse::ManualSelectionRequired {
        role: TargetRole::Editor,
    };
    let manual_selection_json =
        serde_json::to_string(&manual_selection).expect("manual selection should serialize");
    assert!(manual_selection_json.contains("\"manual_selection_required\""));

    let unavailable = BridgeResponse::TargetSessionUnavailable {
        session_id: "session-1".to_string(),
    };
    let unavailable_json =
        serde_json::to_string(&unavailable).expect("unavailable response should serialize");
    assert!(unavailable_json.contains("\"target_session_unavailable\""));

    let persisted = PersistedState {
        tabs: vec![PersistedTabState {
            root: "repo".into(),
            mode: ContextMode::Preview,
            expanded_directories: vec!["src".into(), "tests".into()],
        }],
        ..PersistedState::default()
    };
    let persisted_json =
        serde_json::to_string(&persisted).expect("persisted state should serialize");
    assert!(persisted_json.contains("\"expanded_directories\""));
    assert!(!persisted_json.contains("\"per_tab_mode\""));
    assert!(!persisted_json.contains("\"bookmarks\""));
    let _focus = Focus::Tree;
    let _focus = Focus::Roots;
    let _mode = ContextMode::Preview;
    let _selection = TargetPickerSelection::CurrentPane;
    let _action = Action::SendRef;
    let _action = Action::OpenExternally;
    let _action = Action::CopyRelativePath;
    let _action = Action::CopyAbsolutePath;
    let _presentation = PreviewPresentation::MermaidPending;
    let _presentation = PreviewPresentation::MermaidAscii;
    let _presentation = PreviewPresentation::MermaidImage;
    let _presentation = PreviewPresentation::MermaidRawSource;
    let _presentation = PreviewPresentation::ImagePending;
    let _presentation = PreviewPresentation::ImageInline;
    let _presentation = PreviewPresentation::ImageSummary;

    let _preview_generation = PreviewGeneration(1);
    let _search_generation = SearchGeneration(2);
    let _git_generation = GitGeneration(3);
    let preview_payload = PreviewPayload {
        title: "README.md".to_string(),
        header: PreviewHeader {
            path: Some("/tmp/README.md".to_string()),
            metadata: vec![PreviewMetadataItem {
                label: "Modified".to_string(),
                value: "2026-03-19 14:05".to_string(),
            }],
        },
        lines: vec!["Heading".to_string()],
        markdown: Some("# Heading".to_string()),
        image: Some(ImagePreview {
            display: ImageDisplay::Pending,
            status: "image preview pending".to_string(),
            format_label: "PNG".to_string(),
            dimensions: Some((1, 1)),
            body_lines: vec!["Preparing inline image preview...".to_string()],
        }),
        mermaid: Some(MermaidPreview {
            source: MermaidSource {
                kind: MermaidSourceKind::MarkdownFence,
                block_index: Some(0),
                total_blocks: 1,
                label: "Mermaid block 1 of 1".to_string(),
                raw_source: "graph TD;A-->B;".to_string(),
            },
            display: MermaidDisplay::Pending,
            status: "mermaid render pending".to_string(),
            body_lines: vec!["graph TD;A-->B;".to_string()],
        }),
    };
    let search_payload = SearchPayload::default();
    let git_payload = GitPayload::default();
    let preview_json =
        serde_json::to_string(&preview_payload).expect("preview payload should serialize");
    assert!(preview_json.contains("\"markdown\":\"# Heading\""));
    assert!(preview_json.contains("\"path\":\"/tmp/README.md\""));
    assert!(preview_json.contains("\"format_label\":\"PNG\""));
    assert!(preview_json.contains("\"display\":\"pending\""));
    assert!(preview_json.contains("\"display\":\"pending\""));
    assert!(preview_json.contains("\"kind\":\"markdown_fence\""));
    let _event = AppEvent::PreviewReady(PreviewGeneration(1), Box::new(preview_payload.clone()));
    let _event = AppEvent::SearchReady(SearchGeneration(1), search_payload.clone());
    let _event = AppEvent::GitReady(GitGeneration(1), git_payload.clone());
    let _event = AppEvent::Timer(TimerKind::UiTick);

    let request = SearchRequest {
        query: "needle".to_string(),
        scope: SearchScope::WholeRepo,
        max_results: 200,
        include_context_lines: 2,
    };
    let response = SearchResponse {
        generation: SearchGeneration(10),
        payload: search_payload,
    };
    let request_json = serde_json::to_string(&request).expect("search request should serialize");
    let response_json = serde_json::to_string(&response).expect("search response should serialize");
    assert!(request_json.contains("\"query\":\"needle\""));
    assert!(response_json.contains("\"generation\":10"));

    let _app = App {
        overlays: OverlayState {
            dialog: Some(DialogState::TargetPicker(TargetPickerState {
                role: TargetRole::Editor,
                selection: TargetPickerSelection::SessionId("session-1".to_string()),
            })),
            ..OverlayState::default()
        },
        bridge: BridgeState {
            connected: true,
            instance_id: Some("instance-1".to_string()),
            ai_target_session_id: Some("session-1".to_string()),
            editor_target_session_id: None,
            session_summaries: vec![session],
            resolved_targets: Some(resolution),
        },
        ..App::default()
    };
    let directory_picker = DirectoryPickerState {
        intent: DirectoryPickerIntent::AddRoot,
        entry_mode: DirectoryPickerEntryMode::DirectoriesOnly,
        current_dir: PathBuf::from("/tmp/grove-home"),
        selected_index: 1,
        entries: vec![
            DirectoryPickerEntry {
                path: PathBuf::from("/tmp"),
                label: "..".to_string(),
                is_parent: true,
            },
            DirectoryPickerEntry {
                path: PathBuf::from("/tmp/grove-home/Documents"),
                label: "Documents".to_string(),
                is_parent: false,
            },
        ],
        error_message: None,
        show_hidden: true,
        respect_gitignore: false,
    };
    let directory_picker_json =
        serde_json::to_value(&directory_picker).expect("directory picker should serialize");
    assert_eq!(directory_picker_json["intent"], "add_root");
    assert_eq!(directory_picker_json["entry_mode"], "directories_only");
    assert_eq!(directory_picker_json["current_dir"], "/tmp/grove-home");
    assert_eq!(directory_picker_json["entries"][0]["label"], "..");
    assert_eq!(directory_picker_json["entries"][0]["is_parent"], true);
    assert_eq!(directory_picker_json["show_hidden"], true);
    assert_eq!(directory_picker_json["respect_gitignore"], false);
    let multi_select = MultiSelectState {
        active: true,
        selected_paths: BTreeSet::from([
            PathBuf::from("docs/index.md"),
            PathBuf::from("src/app.rs"),
        ]),
    };
    let multi_select_json =
        serde_json::to_value(&multi_select).expect("multi-select state should serialize");
    assert_eq!(multi_select_json["active"], true);
    assert_eq!(multi_select_json["selected_paths"][0], "docs/index.md");
    assert_eq!(multi_select_json["selected_paths"][1], "src/app.rs");
    let mut picker_app = App::default();
    assert!(picker_app.open_directory_picker_dialog(directory_picker.clone()));
    assert!(matches!(
        picker_app.dialog_state(),
        Some(DialogState::DirectoryPicker(_))
    ));
    assert_eq!(picker_app.directory_picker_state(), Some(&directory_picker));
    let mut preview_state = PreviewState::default();
    assert_eq!(preview_state.cursor_line, 0);
    assert_eq!(preview_state.preview_selection_range(), None);
    preview_state.cursor_line = 4;
    preview_state.selected_line_start = Some(2);
    preview_state.selected_line_end = Some(6);
    assert_eq!(preview_state.preview_selection_range(), Some((2, 6)));
    assert!(preview_state.clear_preview_selection());
    assert_eq!(preview_state.preview_selection_range(), None);
    preview_state.cursor_line = 7;
    preview_state.selected_line_start = Some(11);
    preview_state.selected_line_end = Some(2);
    assert!(preview_state.clamp_preview_selection(8));
    assert_eq!(preview_state.cursor_line, 7);
    assert_eq!(preview_state.preview_selection_range(), Some((2, 7)));
    preview_state.selected_line_start = Some(3);
    preview_state.selected_line_end = None;
    assert!(preview_state.clamp_preview_selection(8));
    assert_eq!(preview_state.preview_selection_range(), None);
    let _preview_state = PreviewState::default();
    let _ = PathFilterState::default();
    let _ = ContentSearchState::default();
    let content_search = ContentSearchState {
        query: "needle".to_string(),
        generation: SearchGeneration(4),
        active: true,
        status: ContentSearchStatus::Searching,
        status_message: Some("searching".to_string()),
        payload: SearchPayload {
            query: "needle".to_string(),
            hits: vec![],
        },
        selected_hit_index: Some(0),
        ..ContentSearchState::default()
    };
    let content_search_json =
        serde_json::to_value(content_search).expect("content search state should serialize");
    assert_eq!(content_search_json["query"], "needle");
    assert_eq!(content_search_json["generation"], 4);
    assert_eq!(content_search_json["active"], true);
    assert_eq!(content_search_json["status"], "searching");
    assert_eq!(content_search_json["status_message"], "searching");
    assert_eq!(content_search_json["selected_hit_index"], 0);
    assert_eq!(content_search_json["payload"]["query"], "needle");

    let descriptor = ActionDescriptor {
        action: Action::OpenCommandPalette,
        label: "Command Palette".to_string(),
        subtitle: Some("Search actions".to_string()),
        enabled: true,
    };
    let descriptor_json =
        serde_json::to_value(&descriptor).expect("action descriptor should serialize");
    assert_eq!(descriptor_json["action"], "open_command_palette");
    assert_eq!(descriptor_json["label"], "Command Palette");
    assert_eq!(descriptor_json["subtitle"], "Search actions");
    assert_eq!(descriptor_json["enabled"], true);

    let root_tab_contract = RootWorkflowContract::OpenSelectedDirectoryAsRootTab;
    let root_tab_contract_json =
        serde_json::to_value(root_tab_contract).expect("root tab contract should serialize");
    assert_eq!(
        root_tab_contract_json,
        serde_json::json!("open_selected_directory_as_root_tab")
    );

    let bookmark_contract = RootWorkflowContract::ToggleActiveRootBookmark;
    let bookmark_contract_json =
        serde_json::to_value(bookmark_contract).expect("bookmark contract should serialize");
    assert_eq!(
        bookmark_contract_json,
        serde_json::json!("toggle_active_root_bookmark")
    );

    let command_surface_contract = CommandSurfaceContract::UnifiedPalette;
    let command_surface_contract_json = serde_json::to_value(command_surface_contract)
        .expect("command surface contract should serialize");
    assert_eq!(
        command_surface_contract_json,
        serde_json::json!("unified_palette")
    );
    let _selected_root_candidate: fn(&App) -> Option<PathBuf> =
        App::selected_directory_root_candidate;
    let _root_tab_opener: fn(&mut App, PathBuf) -> bool = App::open_selected_directory_as_root_tab;
    let _active_root_bookmarked: fn(&App) -> bool = App::active_root_is_bookmarked;
    let _bookmark_toggler: fn(&mut App) -> bool = App::toggle_active_root_bookmark;
    let _unified_surface_opener: fn(&mut App) -> bool = App::open_unified_command_surface;
    let _multi_select_mode: fn(&App) -> bool = App::active_multi_select_mode;
    let _multi_select_count: fn(&App) -> usize = App::active_multi_select_count;
    let _multi_select_paths: fn(&App) -> Vec<PathBuf> = App::active_multi_selected_paths;
    let _multi_select_mode_toggle: fn(&mut App) -> bool = App::toggle_active_multi_select_mode;
    let _multi_select_exit: fn(&mut App) -> bool = App::exit_active_multi_select_mode;
    let _multi_select_clear: fn(&mut App) -> bool = App::clear_active_multi_select;
    let _multi_select_toggle_path: fn(&mut App) -> bool =
        App::toggle_selected_path_in_active_multi_select;
    let _active_sendable_rel_paths: fn(&App) -> Vec<PathBuf> = App::active_sendable_rel_paths;
    let _directory_picker_opener: fn(&mut App, DirectoryPickerState) -> bool =
        App::open_directory_picker_dialog;
    let _directory_picker_state: fn(&App) -> Option<&DirectoryPickerState> =
        App::directory_picker_state;
    let _command_surface_focus: fn(Focus) -> bool = Focus::is_command_surface;
    let _command_surface_state: fn(&OverlayState) -> &CommandPaletteState =
        OverlayState::command_surface;
    let _git_tab_summary: fn(&GitTabState) -> Option<GitRepoSummary> = GitTabState::repo_summary;

    let overlay = OverlayState {
        command_palette: CommandPaletteState {
            query: "open".to_string(),
            selected_index: 0,
            entries: vec![descriptor.clone()],
            active: true,
        },
        context_menu: ContextMenuState {
            selected_index: 0,
            entries: vec![descriptor],
            active: false,
        },
        previous_focus: Some(Focus::Tree),
        ..OverlayState::default()
    };
    let overlay_json = serde_json::to_value(&overlay).expect("overlay state should serialize");
    assert_eq!(overlay_json["command_palette"]["query"], "open");
    assert_eq!(overlay_json["context_menu"]["active"], false);
    assert_eq!(overlay_json["previous_focus"], "tree");
    assert!(Focus::CommandPalette.is_command_surface());
    assert!(!Focus::ContextMenu.is_command_surface());
    assert_eq!(overlay.command_surface().query, "open");
    let git_status = GitPathStatus {
        index: GitChange::Modified,
        worktree: GitChange::Deleted,
        untracked: false,
        conflicted: false,
        ignored: false,
    };
    let git_status_json =
        serde_json::to_value(git_status).expect("git path status should serialize");
    assert_eq!(git_status_json["index"], "modified");
    assert_eq!(git_status_json["worktree"], "deleted");
    assert_eq!(git_status_json["untracked"], false);
    assert_eq!(git_status_json["conflicted"], false);
    assert_eq!(git_status_json["ignored"], false);

    let mut status_map = HashMap::new();
    status_map.insert("src/main.rs".into(), git_status);
    let _ = GitTabState {
        generation: GitGeneration(4),
        repo: Some(RepoHandle {
            root: "/repo".into(),
            branch_name: "main".into(),
        }),
        status_map,
        last_error: None,
        initialized: true,
        needs_refresh: false,
    };

    let summary_app = App {
        tabs: vec![grove::app::TabState {
            git: GitTabState {
                generation: GitGeneration(7),
                repo: Some(RepoHandle {
                    root: "/repo".into(),
                    branch_name: "main".into(),
                }),
                status_map: HashMap::from([
                    (
                        PathBuf::from("src/staged.rs"),
                        grove::git::backend::GitPathStatus {
                            index: grove::git::backend::GitChange::Added,
                            worktree: grove::git::backend::GitChange::Unmodified,
                            untracked: false,
                            conflicted: false,
                            ignored: false,
                        },
                    ),
                    (
                        PathBuf::from("src/unstaged.rs"),
                        grove::git::backend::GitPathStatus {
                            index: grove::git::backend::GitChange::Unmodified,
                            worktree: grove::git::backend::GitChange::Modified,
                            untracked: false,
                            conflicted: false,
                            ignored: false,
                        },
                    ),
                    (
                        PathBuf::from("src/untracked.rs"),
                        grove::git::backend::GitPathStatus {
                            index: grove::git::backend::GitChange::Unmodified,
                            worktree: grove::git::backend::GitChange::Unmodified,
                            untracked: true,
                            conflicted: false,
                            ignored: false,
                        },
                    ),
                    (
                        PathBuf::from("src/conflicted.rs"),
                        grove::git::backend::GitPathStatus {
                            index: grove::git::backend::GitChange::Unmodified,
                            worktree: grove::git::backend::GitChange::Unmodified,
                            untracked: false,
                            conflicted: true,
                            ignored: false,
                        },
                    ),
                ]),
                last_error: None,
                initialized: true,
                needs_refresh: false,
            },
            ..grove::app::TabState::default()
        }],
        active_tab: 0,
        ..App::default()
    };
    let summary: GitRepoSummary = summary_app
        .active_git_summary()
        .expect("active git summary should be exposed");
    let tab_summary = summary_app.tabs[0]
        .git
        .repo_summary()
        .expect("git tab summary should be exposed");
    assert_eq!(summary.repo_root, PathBuf::from("/repo"));
    assert_eq!(summary.branch_name, "main");
    assert_eq!(summary.staged_paths, 1);
    assert_eq!(summary.unstaged_paths, 1);
    assert_eq!(summary.untracked_paths, 1);
    assert_eq!(summary.conflicted_paths, 1);
    assert_eq!(tab_summary.repo_root, summary.repo_root);
    assert_eq!(tab_summary.branch_name, summary.branch_name);
    assert_eq!(tab_summary.staged_paths, summary.staged_paths);
    assert_eq!(tab_summary.unstaged_paths, summary.unstaged_paths);
    assert_eq!(tab_summary.untracked_paths, summary.untracked_paths);
    assert_eq!(tab_summary.conflicted_paths, summary.conflicted_paths);

    let mut bookmark_app = App::default();
    let active_root = bookmark_app.tabs[bookmark_app.active_tab].root.clone();
    bookmark_app.config.bookmarks.pins.push(active_root.clone());
    assert!(bookmark_app.active_root_is_bookmarked());

    let selected_root_candidate = bookmark_app
        .selected_directory_root_candidate()
        .expect("selected root should resolve to a root-tab candidate");
    assert_eq!(selected_root_candidate, active_root);

    let watcher_runtime =
        WatcherRuntime::new(Config::default().watcher, Box::new(NoopWatcherService));
    let _ = watcher_runtime;
    type NotifyRuntimeConstructor =
        fn(
            grove::config::WatcherConfig,
        ) -> grove::error::Result<WatcherRuntime<Box<dyn WatcherService>>>;
    let notify_runtime_constructor: NotifyRuntimeConstructor =
        WatcherRuntime::<Box<dyn WatcherService>>::new_notify;
    let _ = notify_runtime_constructor;

    let tree = TreeState::default();
    assert!(matches!(tree.index_state, IndexState::Idle));
    assert!(matches!(NodeKind::Directory, NodeKind::Directory));
    assert!(matches!(DirLoadState::Unloaded, DirLoadState::Unloaded));
    assert_eq!(NodeId(1), NodeId(1));
}

#[derive(Default)]
struct NoopWatcherService;

impl WatcherService for NoopWatcherService {
    fn reconcile_open_roots(&mut self, roots: &[PathBuf]) -> grove::error::Result<bool> {
        Ok(!roots.is_empty())
    }

    fn poll_refresh_plans(&mut self) -> grove::error::Result<Vec<RefreshPlan>> {
        Ok(Vec::new())
    }
}

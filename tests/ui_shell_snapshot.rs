use grove::app::App;
use grove::app::TabState;
use grove::bridge::protocol::{SessionLocationHint, SessionSummary, TargetRole};
use grove::git::backend::{GitChange, GitPathStatus, RepoHandle};
use grove::preview::model::{SearchHit, SearchPayload};
use grove::state::{
    ContextMode, DialogState, Focus, PromptDialogIntent, PromptDialogState, TargetPickerSelection,
    TargetPickerState,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn renders_phase_zero_shell_labels() {
    let backend = TestBackend::new(180, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let app = App::default();

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let buffer = terminal.backend().buffer().clone();
    let rendered = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    for label in [
        "Path Filter",
        "Roots",
        "Tree",
        "Preview",
        "Action Bar",
        "Status Bar",
    ] {
        assert!(
            rendered.contains(label),
            "expected rendered shell to contain {label}"
        );
    }
}

#[test]
fn renders_bridge_state_and_picker_hints() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();

    app.bridge.connected = true;
    app.bridge.ai_target_session_id = Some("ai-session".to_string());
    app.bridge.editor_target_session_id = Some("editor-session".to_string());
    app.bridge.session_summaries = vec![
        SessionSummary {
            session_id: "ai-session".to_string(),
            title: "Claude".to_string(),
            role: Some(TargetRole::Ai),
            job_name: Some("claude".to_string()),
            command_line: Some("claude".to_string()),
            cwd: Some("/repo".to_string()),
            location_hint: Some(SessionLocationHint {
                window_id: Some("window-1".to_string()),
                tab_id: Some("tab-1".to_string()),
                window_title: Some("Workspace".to_string()),
                tab_title: Some("AI".to_string()),
            }),
        },
        SessionSummary {
            session_id: "editor-session".to_string(),
            title: "Helix".to_string(),
            role: Some(TargetRole::Editor),
            job_name: Some("hx".to_string()),
            command_line: Some("hx".to_string()),
            cwd: Some("/repo".to_string()),
            location_hint: Some(SessionLocationHint {
                window_id: Some("window-1".to_string()),
                tab_id: Some("tab-2".to_string()),
                window_title: Some("Workspace".to_string()),
                tab_title: Some("Editor".to_string()),
            }),
        },
    ];
    app.overlays.dialog = Some(DialogState::TargetPicker(TargetPickerState {
        role: TargetRole::Ai,
        selection: TargetPickerSelection::SessionId("ai-session".to_string()),
    }));
    app.tabs[0].git.repo = Some(RepoHandle {
        root: "/repo".into(),
        branch_name: "main".to_string(),
    });
    app.focus = Focus::Dialog;
    app.overlays.previous_focus = Some(Focus::Tree);
    app.status.message = "ready".to_string();

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let buffer = terminal.backend().buffer().clone();
    let rendered = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("bridge: online"));
    assert!(rendered.contains("AI: Claude"));
    assert!(rendered.contains("Editor: Helix"));
    assert!(rendered.contains("git: main +0 ~0 ?0 !0"));
    assert!(rendered.contains("ready"));
    assert!(rendered.contains("Up/Down move"));
}

#[test]
fn renders_editor_picker_with_current_pane_and_status_label() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();

    app.bridge.connected = true;
    app.bridge.session_summaries = vec![SessionSummary {
        session_id: "editor-session".to_string(),
        title: "Helix".to_string(),
        role: Some(TargetRole::Editor),
        job_name: Some("hx".to_string()),
        command_line: Some("hx".to_string()),
        cwd: Some("/repo".to_string()),
        location_hint: Some(SessionLocationHint {
            window_id: Some("window-1".to_string()),
            tab_id: Some("tab-2".to_string()),
            window_title: Some("Workspace".to_string()),
            tab_title: Some("Editor".to_string()),
        }),
    }];
    app.overlays.dialog = Some(DialogState::TargetPicker(TargetPickerState {
        role: TargetRole::Editor,
        selection: TargetPickerSelection::CurrentPane,
    }));
    app.focus = Focus::Dialog;
    app.overlays.previous_focus = Some(Focus::Tree);

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Editor: current pane"));
    assert!(rendered.contains("Current pane"));
    assert!(rendered.contains("Picker: editor -> Current pane"));
}

#[test]
fn renders_primary_hints_in_action_bar_without_unavailable_git_actions() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let app = App::default();

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let buffer = terminal.backend().buffer().clone();
    let rendered = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("p: preview"));
    assert!(rendered.contains("Ctrl+F: search"));
    assert!(rendered.contains("Ctrl+P: commands"));
    assert!(!rendered.contains("Ctrl+O: menu"));
    assert!(!rendered.contains("d: diff"));
    assert!(!rendered.contains("s: stage"));
    assert!(!rendered.contains("u: unstage"));
}

#[test]
fn renders_unified_command_surface_sections() {
    let root = make_temp_dir("grove-ui-commands");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");

    let backend = TestBackend::new(140, 60);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(PathBuf::from("alpha.txt").as_path())
    );
    assert!(app.open_command_palette());

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let buffer = terminal.backend().buffer().clone();
    let rendered = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Commands"));
    assert!(rendered.contains("Selection"));
    assert!(rendered.contains("Root"));
    assert!(rendered.contains("Targets"));
    assert!(rendered.contains("View"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn renders_content_search_overlay_query_and_status() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();

    assert!(app.open_active_content_search());
    assert!(app.set_active_content_search_query("needle"));
    app.tabs[0].content_search.status = grove::app::ContentSearchStatus::Searching;
    app.tabs[0].content_search.status_message = Some("searching repository".to_string());

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let buffer = terminal.backend().buffer().clone();
    let rendered = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Content Search"));
    assert!(rendered.contains("needle"));
    assert!(rendered.contains("searching repository"));
}

#[test]
fn renders_preview_metadata_lockup_with_file_size() {
    let root = make_temp_dir("grove-ui-preview-lockup");
    fs::write(root.join("alpha.txt"), "hello").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(PathBuf::from("alpha.txt").as_path())
    );
    let _ = app.refresh_active_preview();
    let _ = app.refresh_active_preview_render_cache(60);

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let buffer = terminal.backend().buffer().clone();
    let rendered = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Size"));
    assert!(rendered.contains("5 B"));

    let path_cells = cells_for_text(&buffer, "Size").expect("metadata should render");
    let body_cells = cells_for_text(&buffer, "hello").expect("body should render");
    assert_ne!(
        path_cells[0].bg, body_cells[0].bg,
        "metadata lockup should be visually separated from the body"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn renders_preview_action_bar_hints_when_preview_is_focused() {
    let root = make_temp_dir("grove-ui-preview-action-hints");
    fs::write(root.join("alpha.txt"), "hello\nworld\n").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App {
        focus: Focus::Preview,
        ..App::default()
    };
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(PathBuf::from("alpha.txt").as_path())
    );
    let _ = app.refresh_active_preview();
    let _ = app.refresh_active_preview_render_cache(60);

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Shift+Up/Down select"));
    assert!(rendered.contains("c copy"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn renders_prompt_dialog_when_dialog_focus_is_active() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let app = App {
        focus: Focus::Dialog,
        overlays: grove::state::OverlayState {
            dialog: Some(DialogState::Prompt(PromptDialogState {
                title: "Rename".to_string(),
                subtitle: Some("Enter the new sibling name".to_string()),
                value: "beta.txt".to_string(),
                confirm_label: "rename".to_string(),
                intent: PromptDialogIntent::Rename,
            })),
            ..grove::state::OverlayState::default()
        },
        ..App::default()
    };

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let buffer = terminal.backend().buffer().clone();
    let rendered = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Rename"));
    assert!(rendered.contains("beta.txt"));
    assert!(rendered.contains("Enter rename"));
}

#[test]
fn renders_add_root_directory_picker_modal() {
    let root = make_temp_dir("grove-ui-add-root-picker");
    fs::create_dir_all(root.join("sel-one")).expect("should create sel-one dir");
    fs::create_dir_all(root.join("other-two")).expect("should create other-two dir");

    let backend = TestBackend::new(140, 42);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.config.general.show_hidden = false;
    app.config.general.respect_gitignore = true;
    app.open_add_root_directory_picker_at(root.clone())
        .expect("picker should open");

    let selected_index = app
        .directory_picker_state()
        .expect("picker should exist")
        .entries
        .iter()
        .position(|entry| entry.label == "sel-one")
        .expect("sel-one entry should exist");
    assert!(app.set_directory_picker_selection_by_index(selected_index));

    if let Some(DialogState::DirectoryPicker(picker)) = app.overlays.dialog.as_mut() {
        picker.error_message = Some("could not open restricted: permission denied".to_string());
    }

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let buffer = terminal.backend().buffer().clone();
    let rendered = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Add Root"));
    assert!(rendered.contains("Path:"));
    assert!(
        rendered.contains(root.file_name().and_then(|name| name.to_str()).unwrap()),
        "the picker header should show the current directory path context"
    );
    assert!(rendered.contains("H off"));
    assert!(rendered.contains("G on"));
    assert!(rendered.contains(" ."));
    assert!(rendered.contains(" .."));
    assert!(rendered.contains(" sel-one"));
    assert!(rendered.contains(" other-two"));
    assert!(rendered.contains("could not open restricted: permission denied"));
    assert!(rendered.contains("Left parent"));
    assert!(rendered.contains("Right open folder"));
    assert!(rendered.contains("Enter pin + open"));

    let selected_cells = cells_for_text(&buffer, "sel-one").expect("selected row should render");
    let unselected_cells =
        cells_for_text(&buffer, "other-two").expect("unselected row should render");
    assert_ne!(
        selected_cells[0].bg, unselected_cells[0].bg,
        "selected directory row should render with distinct emphasis"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn renders_search_results_panel_hits() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();

    app.tabs[0].mode = ContextMode::SearchResults;
    app.tabs[0].content_search.payload = SearchPayload {
        query: "needle".to_string(),
        hits: vec![
            SearchHit {
                path: "docs/alpha.txt".to_string(),
                line: 2,
                excerpt: "needle one".to_string(),
            },
            SearchHit {
                path: "notes/beta.txt".to_string(),
                line: 1,
                excerpt: "needle two".to_string(),
            },
        ],
    };
    app.tabs[0].content_search.selected_hit_index = Some(0);
    app.tabs[0].content_search.status = grove::app::ContentSearchStatus::Ready;
    app.tabs[0].content_search.status_message = Some("2 results".to_string());
    let _ = app.refresh_active_preview();
    let _ = app.refresh_active_preview_render_cache(60);

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let buffer = terminal.backend().buffer().clone();
    let rendered = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Search Results"));
    assert!(rendered.contains("docs/alpha.txt:2"));
    assert!(rendered.contains("needle one"));
}

#[test]
fn renders_tree_with_icons_and_git_dots_without_raw_badges() {
    let root = make_temp_dir("grove-ui-tree-icons");
    fs::create_dir_all(root.join("docs")).expect("should create docs dir");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].git.status_map.insert(
        PathBuf::from("alpha.txt"),
        GitPathStatus {
            worktree: GitChange::Modified,
            ..GitPathStatus::default()
        },
    );
    assert!(app.tabs[0].sync_git_badges());

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains(""));
    assert!(rendered.contains(""));
    assert!(rendered.contains(" alpha.txt"));
    assert!(!rendered.contains("M alpha.txt"));
    assert!(!rendered.contains("??"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn renders_multi_select_tree_title_hints_status_and_row_states() {
    let root = make_temp_dir("grove-ui-multi-select");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta file");
    fs::write(root.join("gamma.txt"), "gamma").expect("should create gamma file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.focus = Focus::Tree;
    assert!(app.toggle_active_multi_select_mode());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(PathBuf::from("alpha.txt").as_path())
    );
    assert!(app.toggle_selected_path_in_active_multi_select());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(PathBuf::from("beta.txt").as_path())
    );
    assert!(app.toggle_selected_path_in_active_multi_select());

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let buffer = terminal.backend().buffer().clone();
    let rendered = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Tree [multi] *"));
    assert!(rendered.contains("multi-select: 2 paths"));
    assert!(rendered.contains("Space toggle"));
    assert!(rendered.contains("x clear"));
    assert!(rendered.contains("m done"));
    assert!(rendered.contains("Ctrl+Y send"));

    let batched_only_cells = cells_for_text(&buffer, "alpha.txt").expect("alpha should render");
    let selected_batched_cells = cells_for_text(&buffer, "beta.txt").expect("beta should render");
    let unbatched_cells = cells_for_text(&buffer, "gamma.txt").expect("gamma should render");
    assert_ne!(
        batched_only_cells[0].bg, unbatched_cells[0].bg,
        "batched rows should render distinctly from unbatched rows"
    );
    assert_ne!(
        selected_batched_cells[0].bg, batched_only_cells[0].bg,
        "cursor + batched rows should render distinctly from batched-only rows"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn renders_persistent_multi_select_status_and_hints_outside_tree_focus() {
    let root = make_temp_dir("grove-ui-multi-select-persistent");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.focus = Focus::Preview;
    assert!(app.toggle_active_multi_select_mode());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(PathBuf::from("alpha.txt").as_path())
    );
    assert!(app.toggle_selected_path_in_active_multi_select());
    app.status.message = "sent 1 relative path to ai-session".to_string();

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Multi-select on"));
    assert!(rendered.contains("Tab to tree"));
    assert!(rendered.contains("Ctrl+Y send"));
    assert!(rendered.contains("multi-select: 1 path"));
    assert!(!rendered.contains("x clear"));
    assert!(!rendered.contains("m done"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn renders_repo_git_summary_in_tree_strip_and_status_bar() {
    let backend = TestBackend::new(140, 40);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();

    app.tabs[0].git.repo = Some(RepoHandle {
        root: "/repo".into(),
        branch_name: "main".to_string(),
    });
    app.tabs[0].git.status_map.insert(
        PathBuf::from("staged.txt"),
        GitPathStatus {
            index: GitChange::Added,
            ..GitPathStatus::default()
        },
    );
    app.tabs[0].git.status_map.insert(
        PathBuf::from("unstaged.txt"),
        GitPathStatus {
            worktree: GitChange::Modified,
            ..GitPathStatus::default()
        },
    );
    app.tabs[0].git.status_map.insert(
        PathBuf::from("both.txt"),
        GitPathStatus {
            index: GitChange::Modified,
            worktree: GitChange::Modified,
            ..GitPathStatus::default()
        },
    );
    app.tabs[0].git.status_map.insert(
        PathBuf::from("untracked.txt"),
        GitPathStatus {
            untracked: true,
            ..GitPathStatus::default()
        },
    );
    app.tabs[0].git.status_map.insert(
        PathBuf::from("conflicted.txt"),
        GitPathStatus {
            conflicted: true,
            ..GitPathStatus::default()
        },
    );

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("git: main +2 ~2 ?1 !1"));
    assert!(rendered.contains("main"));
    assert!(rendered.contains("+2 staged"));
    assert!(rendered.contains("~2 unstaged"));
    assert!(rendered.contains("?1 untracked"));
    assert!(rendered.contains("!1 conflict"));
}

#[test]
fn renders_root_navigator_with_disambiguated_colliding_root_labels() {
    let root = make_temp_dir("grove-ui-root-labels");
    let alpha_root = root.join("workspace-a").join("client");
    let beta_root = root.join("workspace-b").join("client");
    fs::create_dir_all(&alpha_root).expect("should create alpha root");
    fs::create_dir_all(&beta_root).expect("should create beta root");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(alpha_root.clone());
    assert!(app.open_or_activate_tab(beta_root.clone()));
    app.active_tab = 0;
    app.config.bookmarks.pins = vec![
        fs::canonicalize(&alpha_root).expect("alpha root should canonicalize"),
        fs::canonicalize(&beta_root).expect("beta root should canonicalize"),
    ];

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Pinned"));
    assert!(
        !rendered.contains("Open"),
        "empty Open section should collapse when every open root is already pinned"
    );
    assert!(
        !rendered.contains("open"),
        "pinned roots should not carry a redundant open badge once the empty Open section collapses"
    );
    assert!(rendered.contains("workspace-a"));
    assert!(rendered.contains("workspace-b"));
    assert!(rendered.contains("client"));
    assert!(
        !rendered.contains(" open"),
        "pinned rows should not repeat open state with a text badge"
    );

    let buffer = terminal.backend().buffer().clone();
    let client_cells = cells_for_text(&buffer, "client").expect("client label should render");
    let disambiguator_cells =
        cells_for_text(&buffer, "workspace-a").expect("workspace-a disambiguator should render");
    assert_ne!(
        client_cells[0].fg, disambiguator_cells[0].fg,
        "disambiguator should render dimmer than the primary label"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn roots_navigator_keeps_the_selected_root_visible_when_list_exceeds_panel_height() {
    let root = make_temp_dir("grove-ui-roots-scroll");
    let root_paths = (0..12)
        .map(|index| {
            let path = root.join(format!("root-{index:02}"));
            fs::create_dir_all(&path).expect("root should be created");
            path
        })
        .collect::<Vec<_>>();

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(root_paths[0].clone());
    for path in root_paths.iter().skip(1) {
        assert!(app.open_or_activate_tab(path.clone()));
    }
    app.focus = Focus::Roots;
    let _ = app.select_root_path(&root_paths[11]);

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        rendered.contains("root-11"),
        "selected root should stay visible when the roots panel scrolls"
    );
    assert!(
        !rendered.contains("Pinned"),
        "empty Pinned section should collapse when there are no pinned roots"
    );
    assert!(
        !rendered.contains("root-00"),
        "old entries should scroll out once the selected root moves past the panel height"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn roots_navigator_renders_open_section_without_empty_pinned_section() {
    let root = make_temp_dir("grove-ui-roots-open-only");
    let alpha_root = root.join("alpha");
    let beta_root = root.join("beta");
    fs::create_dir_all(&alpha_root).expect("alpha root should be created");
    fs::create_dir_all(&beta_root).expect("beta root should be created");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(alpha_root.clone());
    assert!(app.open_or_activate_tab(beta_root));

    terminal
        .draw(|frame| grove::ui::render(frame, &app))
        .expect("shell should render");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        rendered.contains("Open"),
        "unpinned session roots should render under the Open section"
    );
    assert!(
        !rendered.contains("Pinned"),
        "empty Pinned section should collapse when there are no pinned roots"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

fn make_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("{prefix}-{pid}-{nanos}"));
    fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}

fn cells_for_text(buffer: &Buffer, text: &str) -> Option<Vec<Cell>> {
    let width = buffer.area.width as usize;
    let text_width = text.chars().count();
    if text_width == 0 || text_width > width {
        return None;
    }

    for row in 0..buffer.area.height as usize {
        for column in 0..=width - text_width {
            let start = row * width + column;
            let cells = buffer.content[start..start + text_width].to_vec();
            if cells.iter().map(|cell| cell.symbol()).collect::<String>() == text {
                return Some(cells);
            }
        }
    }

    None
}

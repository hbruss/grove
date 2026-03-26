use std::ffi::OsString;
use std::fs;
use std::io::Cursor;
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use git2::{IndexAddOption, Repository, Signature, build::CheckoutBuilder};
use grove::action::Action;
use grove::app::{App, TabState};
use grove::bootstrap;
use grove::bridge::protocol::{
    BridgeResponse, SendTarget, SessionLocationHint, SessionSummary, TargetRole,
};
use grove::preview::model::{ImageDisplay, MermaidDisplay, MermaidSourceKind};
use grove::search::content::{ContentSearchRequest, start_background_content_search};
use grove::state::{
    ContextMode, DirectoryPickerEntryMode, DirectoryPickerIntent, Focus, TargetPickerSelection,
};
use grove::tree::indexer::{PathIndexEvent, build_snapshot_with_visibility};
use grove::tree::model::VisibilitySettings;
use grove::watcher::{RefreshPlan, WatcherRuntime, WatcherService, normalize_watched_root};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::style::Color;

#[test]
fn shell_exits_when_q_is_received() {
    let mut input = Cursor::new(b"q".to_vec());
    let result = bootstrap::run_shell_with_reader(&mut input);
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn runtime_seam_renders_shell_then_exits_on_q() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(b"q".to_vec());
    let mut app = App::default();

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");

    let buffer = terminal.backend().buffer().clone();
    let rendered = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Roots"));
    assert!(rendered.contains("Status Bar"));
}

#[test]
fn runtime_seam_moves_selection_down_and_redraws_tree_marker() {
    let root = make_temp_dir("grove-bootstrap-nav");
    fs::write(root.join("one.txt"), "1").expect("should create one.txt");
    fs::write(root.join("two.txt"), "2").expect("should create two.txt");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.tabs[0].tree.selected_row, 1);

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("▎"));
    assert!(rendered.contains("one.txt"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_seam_expands_directory_on_right_arrow() {
    let root = make_temp_dir("grove-bootstrap-expand");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x1b, b'[', b'C', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert!(
        app.tabs[0]
            .tree
            .path_to_id
            .contains_key(&std::path::PathBuf::from("alpha/nested.txt")),
        "right arrow should lazy-load the selected directory"
    );

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        rendered.contains(""),
        "expanded directories should render with an expanded disclosure marker"
    );
    assert!(rendered.contains(""));
    assert!(rendered.contains("alpha"));
    assert!(rendered.contains("nested.txt"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn root_navigator_panel_height_collapses_empty_open_section() {
    let root = make_temp_dir("grove-roots-panel-height");
    let root_paths = (0..4)
        .map(|index| {
            let path = root.join(format!("root-{index}"));
            fs::create_dir_all(&path).expect("root should be created");
            path
        })
        .collect::<Vec<_>>();

    let mut app = App::default();
    app.tabs[0] = TabState::new(root_paths[0].clone());
    app.config.bookmarks.pins = root_paths
        .iter()
        .map(|path| fs::canonicalize(path).expect("root should canonicalize"))
        .collect();
    for path in root_paths.iter().skip(1) {
        assert!(app.open_or_activate_tab(path.clone()));
    }

    assert_eq!(
        app.root_navigator_panel_height(),
        7,
        "empty Open section should not reserve an extra header and placeholder row"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_seam_left_arrow_selects_parent_for_child_row() {
    let root = make_temp_dir("grove-bootstrap-left-parent");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![
        0x1b, b'[', b'B', 0x1b, b'[', b'C', 0x1b, b'[', b'B', 0x1b, b'[', b'D', b'q',
    ]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");

    let selected_row = app.tabs[0]
        .tree
        .visible_rows
        .get(app.tabs[0].tree.selected_row)
        .expect("selected row should exist");
    let selected_node = app.tabs[0]
        .tree
        .node(selected_row.node_id)
        .expect("selected node should exist");
    assert_eq!(selected_node.rel_path, std::path::PathBuf::from("alpha"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_keeps_selected_row_visible_when_navigating_beyond_viewport() {
    let root = make_temp_dir("grove-bootstrap-scroll");
    for idx in 0..32_u8 {
        fs::write(root.join(format!("file-{idx:03}.txt")), "x").expect("should create file");
    }

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Vec::new();
    for _ in 0..24 {
        input.extend_from_slice(&[0x1b, b'[', b'B']);
    }
    input.push(b'q');

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result =
        bootstrap::run_with_terminal_and_reader(&mut terminal, &mut Cursor::new(input), &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert!(
        app.tabs[0].tree.scroll_row > 0,
        "scroll row should advance when selection moves beyond viewport"
    );

    let selected_row = app.tabs[0]
        .tree
        .visible_rows
        .get(app.tabs[0].tree.selected_row)
        .expect("selected row should exist");
    let selected_node = app.tabs[0]
        .tree
        .node(selected_row.node_id)
        .expect("selected node should exist");
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        rendered.contains(&selected_node.name),
        "selected node name should be visible in the rendered viewport"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_seam_left_arrow_collapses_selected_directory() {
    let root = make_temp_dir("grove-bootstrap-left-collapse");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![
        0x1b, b'[', b'B', 0x1b, b'[', b'C', 0x1b, b'[', b'D', b'q',
    ]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.tabs[0].tree.visible_rows.len(), 2);
    assert!(
        !app.tabs[0].tree.visible_rows.iter().any(|row| app.tabs[0]
            .tree
            .node(row.node_id)
            .is_some_and(|node| node.name == "nested.txt")),
        "left arrow on an expanded directory should hide its descendants"
    );
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        rendered.contains(""),
        "collapsed directories should render with a collapsed disclosure marker"
    );
    assert!(rendered.contains(""));
    assert!(rendered.contains("alpha"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn add_root_picker_opens_at_requested_directory_with_parent_and_filtered_directory_entries() {
    let root = make_temp_dir("grove-add-root-picker-filtered");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::create_dir_all(root.join(".hidden-dir")).expect("should create hidden dir");
    fs::create_dir_all(root.join("ignored-dir")).expect("should create ignored dir");
    fs::write(root.join(".gitignore"), "ignored-dir/\n").expect("should create .gitignore");
    fs::write(root.join("notes.txt"), "not a directory").expect("should create file");
    Repository::init(&root).expect("repo should initialize");

    let mut app = App::default();
    assert!(
        app.open_add_root_directory_picker_at(root.clone())
            .expect("picker should open"),
    );

    let picker = app
        .directory_picker_state()
        .expect("picker state should be open");
    assert_eq!(picker.intent, DirectoryPickerIntent::AddRoot);
    assert_eq!(picker.entry_mode, DirectoryPickerEntryMode::DirectoriesOnly);
    assert_eq!(
        picker.current_dir,
        root.canonicalize().expect("root should canonicalize")
    );
    let labels = picker
        .entries
        .iter()
        .map(|entry| entry.label.as_str())
        .collect::<Vec<_>>();
    assert_eq!(labels.first().copied(), Some("."));
    assert_eq!(labels.get(1).copied(), Some(".."));
    assert!(labels.contains(&"alpha"));
    assert!(!labels.contains(&".hidden-dir"));
    assert!(!labels.contains(&"ignored-dir"));
    assert!(!labels.contains(&"notes.txt"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn add_root_picker_inherits_global_hidden_and_gitignore_visibility_settings() {
    let root = make_temp_dir("grove-add-root-picker-visibility");
    fs::create_dir_all(root.join(".hidden-dir")).expect("should create hidden dir");
    fs::create_dir_all(root.join("ignored-dir")).expect("should create ignored dir");
    fs::write(root.join(".gitignore"), "ignored-dir/\n").expect("should create .gitignore");
    Repository::init(&root).expect("repo should initialize");

    let mut app = App::default();
    app.config.general.show_hidden = true;
    app.config.general.respect_gitignore = false;
    assert!(
        app.open_add_root_directory_picker_at(root.clone())
            .expect("picker should open"),
    );

    let picker = app
        .directory_picker_state()
        .expect("picker state should be open");
    let labels = picker
        .entries
        .iter()
        .map(|entry| entry.label.as_str())
        .collect::<Vec<_>>();
    assert!(labels.contains(&".hidden-dir"));
    assert!(labels.contains(&"ignored-dir"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn add_root_picker_enters_children_moves_to_parent_and_retains_inline_errors() {
    let root = make_temp_dir("grove-add-root-picker-navigation");
    fs::create_dir_all(root.join("alpha").join("nested")).expect("should create nested dir");

    let mut app = App::default();
    assert!(
        app.open_add_root_directory_picker_at(root.clone())
            .expect("picker should open"),
    );

    let alpha_index = app
        .directory_picker_state()
        .expect("picker state should be open")
        .entries
        .iter()
        .position(|entry| entry.label == "alpha")
        .expect("alpha entry should exist");
    assert!(app.set_directory_picker_selection_by_index(alpha_index));
    assert!(
        app.enter_directory_picker_selection()
            .expect("entering selected directory should work")
    );
    assert_eq!(
        app.directory_picker_state()
            .expect("picker state should remain open")
            .current_dir,
        root.join("alpha")
            .canonicalize()
            .expect("alpha should canonicalize")
    );

    assert!(
        app.move_directory_picker_to_parent()
            .expect("moving to parent should work")
    );
    assert_eq!(
        app.directory_picker_state()
            .expect("picker state should remain open")
            .current_dir,
        root.canonicalize().expect("root should canonicalize")
    );

    let alpha_index = app
        .directory_picker_state()
        .expect("picker state should be open")
        .entries
        .iter()
        .position(|entry| entry.label == "alpha")
        .expect("alpha entry should exist");
    assert!(app.set_directory_picker_selection_by_index(alpha_index));
    fs::remove_dir_all(root.join("alpha")).expect("alpha should be removed");
    assert!(
        app.enter_directory_picker_selection()
            .expect("missing selected directory should surface an inline error")
    );
    let picker = app
        .directory_picker_state()
        .expect("picker state should remain open");
    assert_eq!(
        picker.current_dir,
        root.canonicalize().expect("root should canonicalize")
    );
    assert!(
        picker
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("alpha")),
        "missing-entry error should remain visible inline"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_t_opens_selected_directory_as_root_tab() {
    let root = make_temp_dir("grove-bootstrap-open-root-tab");
    fs::create_dir_all(root.join("project")).expect("should create project dir");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x14, b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("project")),
        "project directory should be selectable before Ctrl+T"
    );
    let expected_root = root
        .join("project")
        .canonicalize()
        .expect("project dir should canonicalize");
    assert_eq!(
        app.selected_directory_root_candidate(),
        Some(expected_root.clone())
    );

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.active_tab, 1);
    assert_eq!(app.tabs[1].root, expected_root);
    assert!(app.status.message.contains("root tab"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_t_warns_on_file_selection_and_leaves_state_unchanged() {
    let root = make_temp_dir("grove-bootstrap-open-root-tab-file");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x14, b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    let expected_root = root.canonicalize().expect("root should canonicalize");

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.tabs[0].root, expected_root);
    assert_eq!(app.status.severity, grove::state::StatusSeverity::Warning);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_t_does_not_duplicate_the_current_root_tab() {
    let root = make_temp_dir("grove-bootstrap-open-root-tab-current");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x14, b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    let expected_root = root.canonicalize().expect("root should canonicalize");

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.tabs[0].root, expected_root);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_t_activates_an_existing_inactive_root_tab() {
    let root = make_temp_dir("grove-bootstrap-open-root-tab-existing");
    fs::create_dir_all(root.join("project")).expect("should create project dir");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x14, b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("project")),
        "project directory should be selectable before Ctrl+T"
    );

    let existing_root = root
        .join("project")
        .canonicalize()
        .expect("project dir should canonicalize");
    assert!(app.open_or_activate_tab(existing_root.clone()));
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.active_tab, 1);
    app.active_tab = 0;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.active_tab, 1);
    assert_eq!(app.tabs[1].root, existing_root);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_tab_focus_can_switch_back_to_the_previous_root_tab() {
    let root = make_temp_dir("grove-bootstrap-tab-focus-switch-root");
    fs::create_dir_all(root.join("project")).expect("should create project dir");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x14, 0x09, 0x1b, b'[', b'A', 0x0d, b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("project")),
        "project directory should be selectable before Ctrl+T"
    );

    let original_root = root.canonicalize().expect("root should canonicalize");

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.tabs[0].root, original_root);
    assert_eq!(app.focus, Focus::Roots);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_escape_clears_non_empty_path_filter_and_keeps_focus() {
    let root = make_temp_dir("grove-bootstrap-path-filter");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("alpha").join("nested.txt"), "nested").expect("should create nested file");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![b'/', b'n', b'e', b's', b't', 0x1b]);
    let mut app = App::default();
    app.tabs[0] = grove::app::TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.focus, grove::state::Focus::PathFilter);
    assert_eq!(app.tabs[0].path_filter.query, "");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("alpha"));
    assert!(rendered.contains("beta.txt"));
    assert!(!rendered.contains("nested.txt"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_escape_on_empty_path_filter_returns_focus_to_tree() {
    let root = make_temp_dir("grove-bootstrap-path-filter-empty-escape");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![b'/', 0x1b]);
    let mut app = App::default();
    app.tabs[0] = grove::app::TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.focus, grove::state::Focus::Tree);
    assert_eq!(app.tabs[0].path_filter.query, "");

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_f_focuses_content_search_without_mutating_path_filter() {
    let root = make_temp_dir("grove-bootstrap-content-search-open");
    fs::write(root.join("alpha.txt"), "needle").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x06, b'n', b'e', b'e', b'd', b'l', b'e']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.focus, Focus::ContentSearch);
    assert!(app.tabs[0].content_search.active);
    assert_eq!(app.tabs[0].content_search.query, "needle");
    assert_eq!(app.tabs[0].path_filter.query, "");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Content Search"));
    assert!(rendered.contains("needle"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_enter_in_content_search_enters_search_results_mode() {
    let root = make_temp_dir("grove-bootstrap-content-search-submit");
    fs::write(root.join("alpha.txt"), "needle").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x06, b'n', b'e', b'e', b'd', b'l', b'e', 0x0d]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.tabs[0].mode, ContextMode::SearchResults);
    assert_eq!(app.tabs[0].content_search.query, "needle");
    assert!(matches!(
        app.tabs[0].content_search.status,
        grove::app::ContentSearchStatus::Searching | grove::app::ContentSearchStatus::Ready
    ));

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Search Results"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_content_search_overlay_is_modal_for_q_and_arrows() {
    let root = make_temp_dir("grove-bootstrap-content-search-modal");
    fs::write(root.join("alpha.txt"), "needle").expect("should create alpha file");
    fs::write(root.join("beta.txt"), "more").expect("should create beta file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x06, b'n', 0x1b, b'[', b'B', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::ContentSearch);
    assert_eq!(app.tabs[0].tree.selected_row, 0);
    assert_eq!(app.tabs[0].content_search.query, "nq");

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_p_opens_command_palette_and_filters_actions() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x10, b'p', b'r', b'e', b'v']);
    let mut app = App::default();

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::CommandPalette);
    assert!(app.overlays.command_palette.active);
    assert_eq!(app.overlays.command_palette.query, "prev");
    assert!(
        app.overlays
            .command_palette
            .entries
            .iter()
            .any(|entry| entry.action == Action::SetContextModePreview)
    );
    assert!(
        !app.overlays
            .command_palette
            .entries
            .iter()
            .any(|entry| entry.action == Action::SetAiTarget)
    );

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Commands"));
    assert!(rendered.contains("preview"));
}

#[test]
fn runtime_ctrl_p_opens_unified_commands_with_selection_actions_first() {
    let root = make_temp_dir("grove-bootstrap-unified-commands-open");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x10]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::CommandPalette);
    assert!(app.overlays.command_palette.active);
    assert_eq!(app.overlays.command_palette.query, "");
    assert_eq!(
        app.overlays
            .command_palette
            .entries
            .first()
            .map(|entry| &entry.action),
        Some(&Action::OpenInEditor)
    );
    assert!(
        app.overlays
            .command_palette
            .entries
            .iter()
            .take(4)
            .all(|entry| entry.action != Action::SetAiTarget),
        "global target actions should not lead the empty-query selection section: {:?}",
        app.overlays.command_palette.entries
    );

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Commands"));
    assert!(rendered.contains("Selection"));
    assert!(rendered.contains("Root"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_command_palette_enter_executes_ai_target_action() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x10, b'a', b'i', b' ', b'p', b'a', b'n', b'e', 0x0d]);
    let mut app = App::default();

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: ignore_send_text,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Dialog);
    assert!(app.dialog_state().is_some());
    assert_eq!(
        app.target_picker_state().map(|picker| picker.role),
        Some(TargetRole::Ai)
    );
    assert!(!app.overlays.command_palette.active);
}

#[test]
fn runtime_ctrl_o_does_not_open_a_separate_command_surface() {
    let root = make_temp_dir("grove-bootstrap-context-menu-open");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x0f]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Tree);
    assert!(!app.overlays.context_menu.active);
    assert!(!app.overlays.command_palette.active);

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!rendered.contains("Context Menu"));
    assert!(!rendered.contains("Commands"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_command_palette_enter_executes_editor_action_from_empty_query() {
    let root = make_temp_dir("grove-bootstrap-context-menu-commit");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x10, 0x0d]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: ignore_bridge_initialize,
            list_sessions: ignore_list_sessions,
            send_text: ignore_send_text,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.status.message, "editor opened");
    assert!(!app.overlays.command_palette.active);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_content_search_navigation_and_enter_activates_second_hit() {
    let root = make_temp_dir("grove-bootstrap-content-search-activate");
    fs::write(root.join("alpha.txt"), "needle one").expect("should create alpha file");
    fs::write(root.join("beta.txt"), "needle two").expect("should create beta file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x0d]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].mode = ContextMode::SearchResults;
    app.tabs[0].content_search.active = true;
    app.tabs[0].content_search.query = "needle".to_string();
    app.tabs[0].content_search.status = grove::app::ContentSearchStatus::Ready;
    app.tabs[0].content_search.status_message = Some("2 results".to_string());
    app.tabs[0].content_search.payload = grove::preview::model::SearchPayload {
        query: "needle".to_string(),
        hits: vec![
            grove::preview::model::SearchHit {
                path: "alpha.txt".to_string(),
                line: 1,
                excerpt: "needle one".to_string(),
            },
            grove::preview::model::SearchHit {
                path: "beta.txt".to_string(),
                line: 1,
                excerpt: "needle two".to_string(),
            },
        ],
    };
    app.tabs[0].content_search.selected_hit_index = Some(0);
    app.focus = Focus::ContentSearch;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Preview);
    assert_eq!(app.tabs[0].mode, ContextMode::Preview);
    assert_eq!(
        app.tabs[0].tree.selected_rel_path().as_deref(),
        Some(std::path::Path::new("beta.txt"))
    );
    assert!(!app.tabs[0].content_search.active);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_command_palette_escape_restores_previous_focus() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x09, 0x09, 0x10, 0x1b]);
    let mut app = App::default();

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Preview);
    assert!(!app.overlays.command_palette.active);
}

#[test]
fn runtime_command_palette_query_executes_editor_target_action() {
    let root = make_temp_dir("grove-bootstrap-editor-target-palette");
    fs::write(root.join("note.txt"), "note").expect("should create note file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![
        0x10, b'e', b'd', b'i', b't', b'o', b'r', b' ', b'p', b'a', b'n', b'e', 0x0d,
    ]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("note.txt"))
    );

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: ignore_send_text,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Dialog);
    assert!(app.dialog_state().is_some());
    assert_eq!(
        app.target_picker_state().map(|picker| picker.role),
        Some(TargetRole::Editor)
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_command_palette_new_file_prompt_accepts_reserved_chars_and_creates_file() {
    let root = make_temp_dir("grove-bootstrap-new-file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![
        0x10, b'n', b'e', b'w', b' ', b'f', b'i', b'l', b'e', 0x0d, b'd', b'o', b'c', b's', b'/',
        b's', b'p', b'u', b'd', b'.', b't', b'x', b't', 0x0d,
    ]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert!(root.join("docs/spud.txt").exists());
    assert_eq!(
        app.tabs[0].tree.selected_rel_path().as_deref(),
        Some(std::path::Path::new("docs/spud.txt"))
    );
    assert_eq!(app.status.message, "created docs/spud.txt");
    assert_eq!(app.focus, Focus::Tree);
    assert!(app.dialog_state().is_none());

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_new_file_prompt_preserves_leading_and_trailing_spaces() {
    let root = make_temp_dir("grove-bootstrap-new-file-spaces");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![
        0x10, b'n', b'e', b'w', b' ', b'f', b'i', b'l', b'e', 0x0d, b' ', b'r', b'e', b'p', b'o',
        b'r', b't', b'.', b't', b'x', b't', b' ', 0x0d,
    ]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert!(root.join(" report.txt ").exists());

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_new_file_prompt_invalid_path_surfaces_error_without_exiting() {
    let root = make_temp_dir("grove-bootstrap-new-file-invalid");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![
        0x10, b'n', b'e', b'w', b' ', b'f', b'i', b'l', b'e', 0x0d, b'.', b'.', b'/', b'b', b'a',
        b'd', b'.', b't', b'x', b't', 0x0d,
    ]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert!(!root.join("../bad.txt").exists());
    assert_eq!(app.focus, Focus::Dialog);
    assert!(app.prompt_dialog_state().is_some());
    assert_eq!(app.status.severity, grove::state::StatusSeverity::Error);
    assert!(
        app.status
            .message
            .contains("path cannot escape the active root")
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_command_palette_trash_confirm_deletes_selected_path() {
    let root = make_temp_dir("grove-bootstrap-trash-file");
    let trash_dir = root.join(".test-trash");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");

    with_test_trash_dir(&trash_dir, || {
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should build");
        let mut input = Cursor::new(vec![0x10, b't', b'r', b'a', b's', b'h', 0x0d, 0x0d]);
        let mut app = App::default();
        app.tabs[0] = TabState::new(root.clone());
        app.tabs[0].path_index.receiver = None;
        app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
        assert!(
            app.tabs[0]
                .tree
                .select_rel_path(std::path::Path::new("alpha.txt"))
        );

        let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
        assert!(result.is_ok(), "{result:?}");
        assert!(!root.join("alpha.txt").exists());
        assert_eq!(app.status.message, "trashed alpha.txt");
        assert_eq!(app.focus, Focus::Tree);
        assert!(app.dialog_state().is_none());
        assert!(trash_dir.exists());
    });

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_rename_overwrite_confirmation_replaces_existing_destination() {
    let root = make_temp_dir("grove-bootstrap-rename-overwrite");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = vec![0x10, b'r', b'e', b'n', b'a', b'm', b'e', 0x0d];
    input.extend(std::iter::repeat_n(0x7f, "alpha.txt".len()));
    input.extend_from_slice(b"beta.txt");
    input.extend_from_slice(&[0x0d, 0x0d]);

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("alpha.txt"))
    );

    let result =
        bootstrap::run_with_terminal_and_reader(&mut terminal, &mut Cursor::new(input), &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert!(!root.join("alpha.txt").exists());
    assert_eq!(
        fs::read_to_string(root.join("beta.txt")).expect("beta should exist"),
        "alpha"
    );
    assert_eq!(app.status.message, "renamed to beta.txt");
    assert!(app.dialog_state().is_none());

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_palette_commit_then_picker_escape_restores_original_focus() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![
        0x10, b'a', b'i', b' ', b'p', b'a', b'n', b'e', 0x0d, 0x1b,
    ]);
    let mut app = App {
        focus: Focus::Preview,
        ..App::default()
    };

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: ignore_send_text,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Preview);
    assert!(!app.overlays.command_palette.active);
    assert!(app.dialog_state().is_none());
}

#[test]
fn runtime_command_palette_down_enter_executes_external_action() {
    let root = make_temp_dir("grove-bootstrap-context-menu-down-enter");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x10, 0x1b, b'[', b'B', 0x0d]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: mark_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: ignore_bridge_initialize,
            list_sessions: ignore_list_sessions,
            send_text: ignore_send_text,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.status.message, "external open");
    assert!(!app.overlays.command_palette.active);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_dialog_precedence_blocks_palette_open() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x01, 0x10]);
    let mut app = App::default();

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: ignore_send_text,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Dialog);
    assert!(app.dialog_state().is_some());
    assert!(!app.overlays.command_palette.active);
}

#[test]
fn runtime_renders_path_filter_indexing_status() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(b"q".to_vec());
    let mut app = App::default();
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Building { indexed_paths: 42 };

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("indexing 42"));
}

#[test]
fn runtime_ctrl_h_reveals_hidden_files() {
    let root = make_temp_dir("grove-bootstrap-hidden-toggle");
    fs::write(root.join(".env"), "secret").expect("should create hidden file");
    fs::write(root.join("visible.txt"), "visible").expect("should create visible file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x08, b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.set_config_path(root.join("config.toml"));
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    assert!(
        !visible_rel_paths(&app.tabs[0])
            .iter()
            .any(|path| path == &std::path::PathBuf::from(".env")),
        "hidden files should be excluded before the toggle is used"
    );

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert!(
        visible_rel_paths(&app.tabs[0])
            .iter()
            .any(|path| path == &std::path::PathBuf::from(".env")),
        "Ctrl+H should reveal hidden files"
    );

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains(".env"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_h_backspaces_inside_filter_without_toggling_hidden() {
    let root = make_temp_dir("grove-bootstrap-hidden-backspace");
    fs::write(root.join(".env"), "secret").expect("should create hidden file");
    fs::write(root.join("visible.txt"), "visible").expect("should create visible file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![b'/', b'a', b'b', 0x08]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.tabs[0].path_filter.query, "a");
    assert!(
        !visible_rel_paths(&app.tabs[0])
            .iter()
            .any(|path| path == &std::path::PathBuf::from(".env")),
        "backspace in the filter should not toggle hidden-file visibility"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_h_persists_visibility_config() {
    let root = make_temp_dir("grove-bootstrap-hidden-persist");
    let config_home = make_temp_dir("grove-bootstrap-hidden-config-home");
    let config_path = config_home.join("grove").join("config.toml");
    fs::write(root.join(".env"), "secret").expect("should create hidden file");
    fs::write(root.join("visible.txt"), "visible").expect("should create visible file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x08, b'q']);
    let mut app = App::default();
    app.set_config_path(config_path.clone());
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let saved = fs::read_to_string(&config_path).expect("Ctrl+H should persist config");
    assert!(
        saved.contains("show_hidden = true"),
        "saved config should persist show_hidden = true, got:\n{saved}"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
    fs::remove_dir_all(config_home).expect("config home should be removed");
}

#[test]
fn runtime_ctrl_h_preserves_deep_tree_selection_and_expansion() {
    let root = make_temp_dir("grove-bootstrap-hidden-deep-selection");
    fs::create_dir_all(root.join("alpha").join("nested")).expect("should create nested dirs");
    fs::write(root.join(".env"), "secret").expect("should create hidden file");
    fs::write(root.join("alpha").join("nested").join("deep.txt"), "deep")
        .expect("should create deep file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![
        0x1b, b'[', b'B', 0x1b, b'[', b'C', 0x1b, b'[', b'B', 0x1b, b'[', b'C', 0x1b, b'[', b'B',
        0x08, b'q',
    ]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.set_config_path(root.join("config.toml"));
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        app.tabs[0].tree.selected_rel_path(),
        Some(std::path::PathBuf::from("alpha/nested/deep.txt"))
    );
    assert!(
        visible_rel_paths(&app.tabs[0])
            .iter()
            .any(|path| path == &std::path::PathBuf::from("alpha/nested/deep.txt")),
        "deep file should remain visible after the visibility rebuild"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_g_reveals_gitignored_files() {
    let root = make_temp_dir("grove-bootstrap-gitignore-toggle");
    fs::create_dir_all(root.join(".git")).expect("should create synthetic git dir");
    fs::write(root.join(".gitignore"), "ignored.txt\n").expect("should create .gitignore");
    fs::write(root.join("ignored.txt"), "ignored").expect("should create ignored file");
    fs::write(root.join("visible.txt"), "visible").expect("should create visible file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x07, b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.set_config_path(root.join("config.toml"));
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    assert!(
        !visible_rel_paths(&app.tabs[0])
            .iter()
            .any(|path| path == &std::path::PathBuf::from("ignored.txt")),
        "gitignored files should be excluded before the toggle is used"
    );

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert!(
        visible_rel_paths(&app.tabs[0])
            .iter()
            .any(|path| path == &std::path::PathBuf::from("ignored.txt")),
        "Ctrl+G should reveal gitignored files"
    );

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("ignored.txt"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_g_persists_visibility_config() {
    let root = make_temp_dir("grove-bootstrap-gitignore-persist");
    let config_home = make_temp_dir("grove-bootstrap-gitignore-config-home");
    let config_path = config_home.join("grove").join("config.toml");
    fs::create_dir_all(root.join(".git")).expect("should create synthetic git dir");
    fs::write(root.join(".gitignore"), "ignored.txt\n").expect("should create .gitignore");
    fs::write(root.join("ignored.txt"), "ignored").expect("should create ignored file");
    fs::write(root.join("visible.txt"), "visible").expect("should create visible file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x07, b'q']);
    let mut app = App::default();
    app.set_config_path(config_path.clone());
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let saved = fs::read_to_string(&config_path).expect("Ctrl+G should persist config");
    assert!(
        saved.contains("respect_gitignore = false"),
        "saved config should persist respect_gitignore = false, got:\n{saved}"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
    fs::remove_dir_all(config_home).expect("config home should be removed");
}

#[test]
fn runtime_renders_directory_preview_for_selected_root() {
    let root = make_temp_dir("grove-bootstrap-directory-preview");
    fs::create_dir_all(root.join("alpha")).expect("should create alpha dir");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(b"q".to_vec());
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Folder"));
    assert!(rendered.contains("Children 2"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_updates_preview_when_selection_changes_to_file() {
    let root = make_temp_dir("grove-bootstrap-file-preview");
    fs::write(root.join("note.txt"), "hello from file\nsecond line\n")
        .expect("should create note.txt");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("hello from file"));
    assert!(rendered.contains("second line"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_right_on_file_invokes_editor_open_hook() {
    let root = make_temp_dir("grove-bootstrap-open-in-editor");
    fs::write(root.join("note.txt"), "hello from file\n").expect("should create note.txt");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x1b, b'[', b'C', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: ignore_bridge_initialize,
            list_sessions: ignore_list_sessions,
            send_text: ignore_send_text,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.status.message, "editor opened");

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_o_on_file_invokes_external_open_hook() {
    let root = make_temp_dir("grove-bootstrap-open-externally");
    fs::write(root.join("index.html"), "<html><body>hi</body></html>")
        .expect("should create index.html");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'o', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: mark_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: ignore_bridge_initialize,
            list_sessions: ignore_list_sessions,
            send_text: ignore_send_text,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.status.message, "external open");

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_o_appends_to_path_filter_query_without_external_open() {
    let root = make_temp_dir("grove-bootstrap-open-filter");
    fs::write(root.join("notes.txt"), "notes").expect("should create notes file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![b'/', b'o']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: mark_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: ignore_bridge_initialize,
            list_sessions: ignore_list_sessions,
            send_text: ignore_send_text,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.tabs[0].path_filter.query, "o");
    assert_ne!(app.status.message, "external open");

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_initializes_bridge_state_through_hook() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(b"q".to_vec());
    let mut app = App::default();

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: send_relative_path_ok,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert!(app.bridge.connected);
    assert_eq!(app.bridge.instance_id.as_deref(), Some("instance-1"));
}

#[test]
fn runtime_detects_git_repo_and_renders_tree_badge_for_modified_file() {
    let root = make_temp_dir("grove-bootstrap-git-repo");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "before\n");
    commit_repo_all(&repo, "initial");
    write_repo_file(&root, "tracked.txt", "after\n");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(b"q".to_vec());
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(
        app.tabs[0].git.repo.as_ref().map(|repo| repo
            .root
            .canonicalize()
            .expect("repo root should canonicalize")),
        Some(root.canonicalize().expect("root should canonicalize"))
    );
    assert!(
        app.tabs[0]
            .git
            .status_map
            .contains_key(std::path::Path::new("tracked.txt"))
    );
    let summary = app
        .active_git_summary()
        .expect("repo summary should be available");
    assert_eq!(summary.staged_paths, 0);
    assert_eq!(summary.unstaged_paths, 1);
    assert_eq!(summary.untracked_paths, 0);
    assert_eq!(summary.conflicted_paths, 0);

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains(&format!("git: {} +0 ~1 ?0 !0", summary.branch_name)));
    assert!(rendered.contains("~1 unstaged"));
    assert!(rendered.contains(" tracked.txt"));
    assert!(!rendered.contains("M tracked.txt"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_outside_repo_keeps_git_state_inactive() {
    let root = make_temp_dir("grove-bootstrap-non-repo");
    fs::write(root.join("plain.txt"), "plain\n").expect("should create plain.txt");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(b"q".to_vec());
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert!(app.tabs[0].git.repo.is_none());
    assert!(app.tabs[0].git.status_map.is_empty());
    assert!(app.tabs[0].git.last_error.is_none());

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("git: none"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn render_shell_refreshes_git_badges_when_git_state_is_dirty() {
    let root = make_temp_dir("grove-bootstrap-git-refresh");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "before\n");
    commit_repo_all(&repo, "initial");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    bootstrap::render_shell_once(&mut terminal, &mut app).expect("initial render should work");
    assert!(
        !app.tabs[0]
            .git
            .status_map
            .contains_key(std::path::Path::new("tracked.txt"))
    );

    write_repo_file(&root, "tracked.txt", "after\n");
    app.tabs[0].git.needs_refresh = true;

    bootstrap::render_shell_once(&mut terminal, &mut app).expect("second render should work");
    let tracked = app.tabs[0]
        .git
        .status_map
        .get(std::path::Path::new("tracked.txt"))
        .expect("tracked file should refresh into git status");
    assert_eq!(tracked.worktree, grove::git::backend::GitChange::Modified);

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains(" tracked.txt"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_badges_collapsed_directory_with_changed_descendant() {
    let root = make_temp_dir("grove-bootstrap-git-collapsed-dir");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "src/lib.rs", "before\n");
    commit_repo_all(&repo, "initial");
    write_repo_file(&root, "src/lib.rs", "after\n");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    bootstrap::render_shell_once(&mut terminal, &mut app).expect("render should work");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains(" src"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_d_switches_to_diff_mode_for_modified_file() {
    let root = make_temp_dir("grove-bootstrap-diff-mode");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "before\n");
    commit_repo_all(&repo, "initial");
    write_repo_file(&root, "tracked.txt", "after\n");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'd', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.tabs[0].mode, ContextMode::Diff);

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Diff"));
    assert!(rendered.contains("-before"));
    assert!(rendered.contains("+after"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_d_on_directory_shows_explicit_diff_message() {
    let root = make_temp_dir("grove-bootstrap-diff-directory");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "before\n");
    commit_repo_all(&repo, "initial");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![b'd', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.tabs[0].mode, ContextMode::Preview);
    assert_eq!(
        app.status.message,
        "diff unavailable: select a modified or untracked file"
    );

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!rendered.contains("Git diff unavailable"));
    assert!(rendered.contains("tracked.txt"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_d_on_clean_file_stays_in_preview_mode_and_sets_status_message() {
    let root = make_temp_dir("grove-bootstrap-diff-clean-file");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "before\n");
    commit_repo_all(&repo, "initial");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'd', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.tabs[0].mode, ContextMode::Preview);
    assert_eq!(
        app.status.message,
        "diff unavailable: select a modified or untracked file"
    );

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("tracked.txt"));
    assert!(!rendered.contains("Git diff unavailable"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_p_switches_back_to_preview_mode_after_diff() {
    let root = make_temp_dir("grove-bootstrap-preview-mode");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "before\n");
    commit_repo_all(&repo, "initial");
    write_repo_file(&root, "tracked.txt", "after\n");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'd', b'p', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.tabs[0].mode, ContextMode::Preview);

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("tracked.txt"));
    assert!(!rendered.contains("+after"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_d_and_p_stay_literal_inside_path_filter() {
    let root = make_temp_dir("grove-bootstrap-diff-preview-filter");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha.txt");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![b'/', b'd', b'p']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.tabs[0].path_filter.query, "dp");
    assert_eq!(app.tabs[0].mode, ContextMode::Preview);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_s_and_u_stay_literal_inside_path_filter() {
    let root = make_temp_dir("grove-bootstrap-stage-unstage-filter");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha.txt");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![b'/', b's', b'u']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.tabs[0].path_filter.query, "su");
    assert_eq!(app.tabs[0].mode, ContextMode::Preview);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_s_stages_selected_modified_file_and_clears_diff_view() {
    let root = make_temp_dir("grove-bootstrap-stage");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "before\n");
    commit_repo_all(&repo, "initial");
    write_repo_file(&root, "tracked.txt", "after\n");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'd', b's', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.tabs[0].mode, ContextMode::Diff);
    assert_eq!(app.status.message, "staged tracked.txt");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Diff"));
    assert!(rendered.contains("no unstaged diff"));
    assert!(!rendered.contains("+after"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_u_restores_unstaged_diff_for_selected_file() {
    let root = make_temp_dir("grove-bootstrap-unstage");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "before\n");
    commit_repo_all(&repo, "initial");
    write_repo_file(&root, "tracked.txt", "after\n");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'd', b's', b'u', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.tabs[0].mode, ContextMode::Diff);
    assert_eq!(app.status.message, "unstaged tracked.txt");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Diff"));
    assert!(rendered.contains("+after"));
    assert!(rendered.contains("-before"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_s_and_u_warn_when_selection_is_not_a_file() {
    let root = make_temp_dir("grove-bootstrap-stage-root");
    fs::create_dir_all(root.join("folder")).expect("should create folder");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![b's', b'u', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(
        app.status.message,
        "select a file before staging or unstaging"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_s_and_u_warn_when_selection_is_a_directory() {
    let root = make_temp_dir("grove-bootstrap-stage-directory");
    fs::create_dir_all(root.join("folder")).expect("should create folder");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b's', b'u', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(
        app.status.message,
        "select a file before staging or unstaging"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_s_and_u_warn_when_selection_is_conflicted() {
    let root = make_temp_dir("grove-bootstrap-stage-conflicted");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "base\n");
    commit_repo_all(&repo, "initial");
    create_merge_conflict(&repo, &root, "tracked.txt");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b's', b'u', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(
        app.status.message,
        "resolve the conflict before staging or unstaging"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_y_sends_selected_relative_path_to_ai_target() {
    let root = make_temp_dir("grove-bootstrap-send-relative-path");
    fs::write(root.join("note.txt"), "hello from file\n").expect("should create note.txt");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x19]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: send_relative_path_ok,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(
        app.bridge.ai_target_session_id.as_deref(),
        Some("ai-session")
    );
    assert_eq!(app.status.message, "sent relative path to ai-session");

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_y_uses_explicit_ai_target_session_binding() {
    let root = make_temp_dir("grove-bootstrap-send-relative-path-explicit-ai-target");
    fs::write(root.join("note.txt"), "hello from file\n").expect("should create note.txt");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x19]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    app.bridge.ai_target_session_id = Some("bound-ai-session".to_string());

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: send_relative_path_to_explicit_ai_session_ok,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(
        app.bridge.ai_target_session_id.as_deref(),
        Some("bound-ai-session")
    );
    assert_eq!(app.status.message, "sent relative path to bound-ai-session");

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_y_clears_stale_explicit_ai_target_and_reopens_picker() {
    let root = make_temp_dir("grove-bootstrap-send-relative-path-stale-ai-target");
    fs::write(root.join("note.txt"), "hello from file\n").expect("should create note.txt");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x19]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    app.bridge.ai_target_session_id = Some("stale-ai-session".to_string());

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: send_explicit_ai_target_unavailable,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.bridge.ai_target_session_id, None);
    assert_eq!(app.focus, Focus::Dialog);
    assert_eq!(
        app.target_picker_state().map(|picker| picker.role),
        Some(TargetRole::Ai)
    );
    assert_eq!(
        app.status.message,
        "ai target stale-ai-session is no longer available; choose a new target"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_y_sends_multi_select_paths_as_newline_separated_payload() {
    let _guard = send_capture_lock()
        .lock()
        .expect("send capture lock should acquire");
    clear_sent_texts();

    let root = make_temp_dir("grove-bootstrap-send-relative-path-batch");
    fs::create_dir_all(root.join("docs")).expect("should create docs dir");
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::write(root.join("docs").join("index.md"), "# docs\n").expect("should create docs/index.md");
    fs::write(root.join("src").join("app.rs"), "fn main() {}\n").expect("should create src/app.rs");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .expand_rel_path(std::path::Path::new("docs"))
    );
    assert!(
        app.tabs[0]
            .tree
            .expand_rel_path(std::path::Path::new("src"))
    );
    assert!(app.toggle_active_multi_select_mode());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("src/app.rs"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("docs/index.md"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());
    assert!(app.exit_active_multi_select_mode());

    let mut input = Cursor::new(vec![0x19]);
    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: capture_relative_paths_ok,
            set_role: ignore_set_role,
        },
    );

    assert!(result.is_ok());
    assert_eq!(
        take_sent_texts(),
        vec!["docs/index.md\nsrc/app.rs".to_string()]
    );
    assert_eq!(
        app.bridge.ai_target_session_id.as_deref(),
        Some("ai-session")
    );
    assert_eq!(app.status.message, "sent 2 relative paths to ai-session");

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_y_opens_manual_selection_state_for_multi_select_batch_when_ai_target_is_unresolved()
{
    let _guard = send_capture_lock()
        .lock()
        .expect("send capture lock should acquire");
    clear_sent_texts();

    let root = make_temp_dir("grove-bootstrap-send-relative-path-batch-unresolved");
    fs::create_dir_all(root.join("docs")).expect("should create docs dir");
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::write(root.join("docs").join("index.md"), "# docs\n").expect("should create docs/index.md");
    fs::write(root.join("src").join("app.rs"), "fn main() {}\n").expect("should create src/app.rs");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .expand_rel_path(std::path::Path::new("docs"))
    );
    assert!(
        app.tabs[0]
            .tree
            .expand_rel_path(std::path::Path::new("src"))
    );
    assert!(app.toggle_active_multi_select_mode());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("src/app.rs"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("docs/index.md"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());

    let mut input = Cursor::new(vec![0x19]);
    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: capture_relative_paths_requires_manual_ai,
            set_role: ignore_set_role,
        },
    );

    assert!(result.is_ok());
    assert_eq!(
        take_sent_texts(),
        vec!["docs/index.md\nsrc/app.rs".to_string()]
    );
    assert_eq!(app.focus, Focus::Dialog);
    assert!(app.target_picker_state().is_some());

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn app_multi_select_helpers_collect_sorted_paths_and_fall_back_to_current_selection() {
    let root = make_temp_dir("grove-bootstrap-multi-select-helpers");
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::create_dir_all(root.join("docs")).expect("should create docs dir");
    fs::write(root.join("src").join("app.rs"), "fn main() {}\n").expect("should create src/app.rs");
    fs::write(root.join("docs").join("index.md"), "# docs\n").expect("should create docs/index.md");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    assert!(!app.active_multi_select_mode());
    assert!(app.toggle_active_multi_select_mode());
    assert!(app.active_multi_select_mode());
    assert_eq!(app.active_multi_select_count(), 0);

    assert!(
        app.tabs[0]
            .tree
            .expand_rel_path(std::path::Path::new("src"))
    );
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("src/app.rs"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());

    assert!(
        app.tabs[0]
            .tree
            .expand_rel_path(std::path::Path::new("docs"))
    );
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("docs/index.md"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());

    assert_eq!(app.active_multi_select_count(), 2);
    assert_eq!(
        app.active_multi_selected_paths(),
        vec![
            std::path::PathBuf::from("docs/index.md"),
            std::path::PathBuf::from("src/app.rs"),
        ]
    );
    assert_eq!(
        app.active_sendable_rel_paths(),
        vec![
            std::path::PathBuf::from("docs/index.md"),
            std::path::PathBuf::from("src/app.rs"),
        ]
    );

    assert!(app.clear_active_multi_select());
    assert_eq!(app.active_multi_select_count(), 0);
    assert_eq!(
        app.active_sendable_rel_paths(),
        vec![std::path::PathBuf::from("docs/index.md")]
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn app_multi_select_helpers_reject_root_and_preserve_batch_on_mode_exit() {
    let root = make_temp_dir("grove-bootstrap-multi-select-root");
    fs::write(root.join("note.txt"), "hello\n").expect("should create note.txt");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    assert!(app.toggle_active_multi_select_mode());
    assert!(app.active_multi_select_mode());
    assert!(!app.toggle_selected_path_in_active_multi_select());
    assert_eq!(app.active_multi_select_count(), 0);

    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("note.txt"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());
    assert_eq!(
        app.active_multi_selected_paths(),
        vec![std::path::PathBuf::from("note.txt")]
    );

    assert!(app.exit_active_multi_select_mode());
    assert!(!app.active_multi_select_mode());
    assert!(!app.toggle_selected_path_in_active_multi_select());
    assert_eq!(
        app.active_sendable_rel_paths(),
        vec![std::path::PathBuf::from("note.txt")]
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn multi_select_survives_visibility_rebuild_for_existing_hidden_path() {
    let root = make_temp_dir("grove-bootstrap-multi-select-hidden-visibility");
    fs::write(root.join(".secret"), "shh\n").expect("should create .secret");
    fs::write(root.join("note.txt"), "hello\n").expect("should create note.txt");

    let mut app = App::default();
    app.tabs[0] = TabState::new_with_visibility(
        root.clone(),
        VisibilitySettings {
            show_hidden: true,
            respect_gitignore: true,
        },
    );

    assert!(app.toggle_active_multi_select_mode());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new(".secret"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());
    assert_eq!(
        app.active_multi_selected_paths(),
        vec![std::path::PathBuf::from(".secret")]
    );

    assert!(
        app.toggle_show_hidden()
            .expect("toggling hidden visibility should succeed")
    );
    assert_eq!(
        app.active_multi_selected_paths(),
        vec![std::path::PathBuf::from(".secret")]
    );
    assert!(
        !app.tabs[0]
            .tree
            .visible_rows
            .iter()
            .filter_map(|row| app.tabs[0].tree.node(row.node_id))
            .any(|node| node.rel_path == std::path::Path::new(".secret")),
        "hidden path should disappear from visible rows while remaining batched"
    );

    assert!(
        app.toggle_show_hidden()
            .expect("restoring hidden visibility should succeed")
    );
    assert_eq!(
        app.active_multi_selected_paths(),
        vec![std::path::PathBuf::from(".secret")]
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn file_op_refresh_prunes_multi_select_paths_that_disappeared() {
    let root = make_temp_dir("grove-bootstrap-multi-select-file-op-prune");
    fs::write(root.join("alpha.txt"), "alpha\n").expect("should create alpha.txt");
    fs::write(root.join("beta.txt"), "beta\n").expect("should create beta.txt");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    assert!(app.toggle_active_multi_select_mode());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("alpha.txt"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("beta.txt"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());
    assert_eq!(app.active_multi_select_count(), 2);

    fs::remove_file(root.join("alpha.txt")).expect("should remove alpha.txt");
    app.refresh_active_tab_after_file_op(Some(std::path::Path::new("beta.txt")))
        .expect("file-op refresh should succeed");

    assert_eq!(
        app.active_multi_selected_paths(),
        vec![std::path::PathBuf::from("beta.txt")]
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_prunes_multi_select_paths_that_disappeared() {
    let root = make_temp_dir("grove-bootstrap-multi-select-watcher-prune");
    fs::create_dir_all(root.join("docs")).expect("should create docs dir");
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::write(root.join("docs").join("index.md"), "# docs\n").expect("should create docs/index.md");
    fs::write(root.join("src").join("app.rs"), "fn main() {}\n").expect("should create src/app.rs");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());

    assert!(
        app.tabs[0]
            .tree
            .expand_rel_path(std::path::Path::new("docs"))
    );
    assert!(
        app.tabs[0]
            .tree
            .expand_rel_path(std::path::Path::new("src"))
    );
    assert!(app.toggle_active_multi_select_mode());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("docs/index.md"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("src/app.rs"))
    );
    assert!(app.toggle_selected_path_in_active_multi_select());
    assert_eq!(app.active_multi_select_count(), 2);

    fs::remove_file(root.join("docs").join("index.md")).expect("should remove docs/index.md");
    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            changed_paths: vec![std::path::PathBuf::from("docs/index.md")],
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should rebuild the active tab");
    assert_eq!(
        app.active_multi_selected_paths(),
        vec![std::path::PathBuf::from("src/app.rs")]
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_y_opens_manual_selection_state_when_ai_target_is_unresolved() {
    let root = make_temp_dir("grove-bootstrap-send-relative-path-unresolved");
    fs::write(root.join("note.txt"), "hello from file\n").expect("should create note.txt");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x19]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: send_requires_manual_ai,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Dialog);
    assert!(app.dialog_state().is_some());
    assert_eq!(
        app.target_picker_state().map(|picker| picker.role),
        Some(TargetRole::Ai)
    );
    assert_eq!(app.bridge.session_summaries.len(), 2);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_a_opens_ai_target_picker_overlay() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x01]);
    let mut app = App::default();

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: send_relative_path_ok,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Dialog);
    assert!(app.dialog_state().is_some());
    assert_eq!(
        app.target_picker_state().map(|picker| picker.role),
        Some(TargetRole::Ai)
    );

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Select AI Target"));
    assert!(rendered.contains("Claude"));
    assert!(
        !rendered.contains("Current pane"),
        "AI target picker should not include the local Grove pane"
    );
}

#[test]
fn runtime_ctrl_e_opens_editor_picker_with_current_pane_selected_first() {
    let root = make_temp_dir("grove-bootstrap-editor-picker-current-pane");
    fs::write(root.join("note.md"), "# note\n").expect("should create markdown file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x05]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("note.md"))
    );

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: send_relative_path_ok,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Dialog);
    assert_eq!(
        app.target_picker_state().map(|picker| picker.role),
        Some(TargetRole::Editor)
    );
    assert_eq!(
        app.target_picker_state()
            .map(|picker| picker.selection.clone()),
        Some(TargetPickerSelection::CurrentPane)
    );
    assert_eq!(app.target_picker_selected_index(), Some(0));
    assert!(app.target_picker_selected_session().is_none());

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Select Editor Target"));
    assert!(rendered.contains("Current pane"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_ctrl_a_with_no_sessions_renders_picker_warning() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x01]);
    let mut app = App::default();

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: ignore_list_sessions,
            send_text: send_relative_path_ok,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Tree);
    assert!(app.dialog_state().is_none());
    assert_eq!(app.status.message, "no sessions available for picker");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("no sessions available for picker"));
}

#[test]
fn runtime_ctrl_a_preserves_specific_bridge_list_error() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x01]);
    let mut app = App::default();

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: bridge_list_error,
            send_text: send_relative_path_ok,
            set_role: ignore_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Tree);
    assert!(app.dialog_state().is_none());
    assert_eq!(app.status.message, "bridge session list failed: boom");

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("bridge session list failed: boom"));
}

#[test]
fn runtime_picker_navigation_and_enter_sets_ai_target() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x01, 0x1b, b'[', b'B', b'\n']);
    let mut app = App::default();

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: send_relative_path_ok,
            set_role: set_ai_target_ok,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Tree);
    assert!(app.dialog_state().is_none());
    assert_eq!(
        app.bridge.ai_target_session_id.as_deref(),
        Some("editor-session")
    );
    assert_eq!(app.status.message, "ai target set to Helix");
}

#[test]
fn runtime_ctrl_e_on_root_warns_without_opening_picker() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x05]);
    let mut app = App::default();

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: send_relative_path_ok,
            set_role: panic_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Tree);
    assert!(app.dialog_state().is_none());
    assert_eq!(
        app.status.message,
        "select a file before choosing an editor target"
    );
}

#[test]
fn runtime_editor_picker_enter_on_current_pane_opens_selected_file() {
    let root = make_temp_dir("grove-bootstrap-editor-picker-enter-current");
    fs::write(root.join("note.md"), "# note\n").expect("should create markdown file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x05, b'\n']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("note.md"))
    );
    app.bridge.editor_target_session_id = Some("editor-session".to_string());

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: ignore_list_sessions,
            send_text: send_relative_path_ok,
            set_role: panic_set_role,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Tree);
    assert!(app.dialog_state().is_none());
    assert_eq!(app.bridge.editor_target_session_id, None);
    assert_eq!(app.status.message, "editor opened");

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_picker_escape_restores_previous_focus() {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x01, 0x1b]);
    let mut app = App {
        focus: Focus::Preview,
        ..App::default()
    };

    let result = bootstrap::run_with_terminal_and_reader_and_hooks(
        &mut terminal,
        &mut input,
        &mut app,
        bootstrap::RuntimeActionHooks {
            open_in_editor: mark_editor_open,
            open_externally: ignore_external_open,
            reveal_in_file_manager: ignore_external_open,
            initialize_bridge: mark_bridge_initialized,
            list_sessions: list_target_sessions,
            send_text: send_relative_path_ok,
            set_role: set_ai_target_ok,
        },
    );
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Preview);
    assert!(app.dialog_state().is_none());
}

#[test]
fn runtime_renders_markdown_preview_without_raw_markup() {
    let root = make_temp_dir("grove-bootstrap-markdown-preview");
    fs::write(
        root.join("README.md"),
        "# Heading\n\nA paragraph.\n\n- item one\n\n[OpenAI](https://openai.com)\n",
    )
    .expect("should create markdown file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Heading"));
    assert!(rendered.contains("A paragraph."));
    assert!(rendered.contains("item one"));
    assert!(rendered.contains("OpenAI (https://openai.com)"));
    assert!(!rendered.contains("Links"));
    assert!(!rendered.contains("# Heading"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_renders_markdown_preview_with_styled_heading() {
    let root = make_temp_dir("grove-bootstrap-markdown-styled-preview");
    fs::write(
        root.join("README.md"),
        "# Heading\n\nA paragraph.\n\n[OpenAI](https://openai.com)\n",
    )
    .expect("should create markdown file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let buffer = terminal.backend().buffer().clone();
    let heading_cells = cells_for_ascii_text(&buffer, "Heading").expect("heading should render");
    assert!(
        heading_cells.iter().any(|cell| cell.fg != Color::Reset),
        "markdown heading cells should use non-default foreground styling"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_detects_native_mermaid_files_and_surfaces_pending_fallback_state() {
    let root = make_temp_dir("grove-bootstrap-native-mermaid-preview");
    fs::write(root.join("diagram.mmd"), "graph TD\n    Start --> Finish\n")
        .expect("should create mermaid file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let mermaid = app.tabs[0]
        .preview
        .payload
        .mermaid
        .as_ref()
        .expect("native mermaid file should populate Mermaid preview metadata");
    assert_eq!(mermaid.source.kind, MermaidSourceKind::NativeFile);
    assert_eq!(mermaid.display, MermaidDisplay::Pending);
    assert_eq!(mermaid.source.block_index, None);
    assert_eq!(mermaid.source.total_blocks, 1);
    assert!(
        mermaid
            .body_lines
            .iter()
            .any(|line| line.contains("Start --> Finish")),
        "pending fallback should retain the raw Mermaid source"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_detects_supported_image_files_and_surfaces_pending_image_preview_state() {
    let root = make_temp_dir("grove-bootstrap-image-preview");
    write_tiny_png(&root.join("pixel.png"));

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("pixel.png")),
        "image file should be selectable before preview refresh"
    );

    assert!(app.refresh_active_preview());

    let image = app.tabs[0]
        .preview
        .payload
        .image
        .as_ref()
        .expect("supported image file should populate image preview metadata");
    assert_eq!(image.display, ImageDisplay::Pending);
    assert_eq!(image.format_label, "PNG");
    assert!(
        image.status.to_ascii_lowercase().contains("pending"),
        "image preview should start in a pending state before inline rendering finishes"
    );
    assert!(app.tabs[0].preview.payload.mermaid.is_none());

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_detects_markdown_mermaid_fences_and_surfaces_pending_fallback_state() {
    let root = make_temp_dir("grove-bootstrap-markdown-mermaid-preview");
    fs::write(
        root.join("README.md"),
        "# Diagram\n\n```mermaid\ngraph TD\n    A --> B\n```\n",
    )
    .expect("should create markdown file with Mermaid fence");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let mermaid = app.tabs[0]
        .preview
        .payload
        .mermaid
        .as_ref()
        .expect("markdown Mermaid fence should populate Mermaid preview metadata");
    assert_eq!(mermaid.source.kind, MermaidSourceKind::MarkdownFence);
    assert_eq!(mermaid.display, MermaidDisplay::Pending);
    assert_eq!(mermaid.source.block_index, Some(0));
    assert_eq!(mermaid.source.total_blocks, 1);
    assert!(
        mermaid
            .body_lines
            .iter()
            .any(|line| line.contains("A --> B")),
        "pending fallback should retain the Mermaid fence source"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_pretty_prints_json_preview() {
    let root = make_temp_dir("grove-bootstrap-json-preview");
    fs::write(
        root.join("data.json"),
        "{\"name\":\"grove\",\"items\":[1,2]}",
    )
    .expect("should create json file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("\"name\": \"grove\""));
    assert!(rendered.contains("\"items\": ["));
    assert!(!rendered.contains("{\"name\":\"grove\",\"items\":[1,2]}"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_renders_binary_preview_summary() {
    let root = make_temp_dir("grove-bootstrap-binary-preview");
    fs::write(
        root.join("image.bin"),
        [0_u8, 1_u8, 2_u8, 65_u8, 66_u8, 67_u8],
    )
    .expect("should create binary file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Binary"));
    assert!(rendered.contains("00000000"));
    assert!(rendered.contains("41 42 43"));
    assert!(rendered.contains("ABC"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_tab_can_focus_preview_panel() {
    let root = make_temp_dir("grove-bootstrap-preview-focus");
    fs::write(root.join("notes.txt"), "line 00\nline 01\n").expect("should create notes file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x09, 0x09, b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Preview);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_v_hides_preview_keeps_metadata_visible_and_returns_focus_to_tree() {
    let root = make_temp_dir("grove-bootstrap-preview-hide");
    fs::write(root.join("notes.txt"), "line 00\nline 01\n").expect("should create notes file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x09, 0x09, b'v', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.focus, Focus::Tree);

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Metadata"));
    assert!(!rendered.contains("line 00"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_hidden_preview_layout_lets_tree_span_full_body_width() {
    let root = make_temp_dir("grove-bootstrap-preview-width");
    let long_name = "alpha-preview-width-expansion-marker-visible-only-when-tree-spans-wide.txt";
    fs::write(root.join(long_name), "wide\n").expect("should create wide-name file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', b'v', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok(), "{result:?}");

    let buffer = terminal.backend().buffer().clone();
    let positions = ascii_text_positions(&buffer, "visible-only-when-tree-spans-wide")
        .expect("expanded tree should expose the filename tail below the metadata band");
    assert!(
        positions.iter().any(|(row, _)| *row > 8),
        "expected the long filename tail to render in the tree area after preview is hidden"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_roots_focus_can_activate_a_pinned_root_as_a_tab() {
    let root = make_temp_dir("grove-bootstrap-bookmark-root");
    fs::write(root.join("notes.txt"), "line 00\n").expect("should create notes file");
    let bookmark_root = make_temp_dir("grove-bootstrap-bookmark-target");
    fs::write(bookmark_root.join("todo.txt"), "todo\n").expect("should create todo file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x09, 0x0d, b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.config.bookmarks.pins = vec![bookmark_root.clone()];

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Roots);
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.active_tab, 1);
    assert_eq!(
        app.tabs[1].root,
        fs::canonicalize(&bookmark_root).expect("bookmark root should canonicalize")
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
    fs::remove_dir_all(bookmark_root).expect("bookmark root should be removed");
}

#[test]
fn runtime_roots_focus_enter_on_missing_pinned_root_warns_without_exiting() {
    let root = make_temp_dir("grove-bootstrap-missing-bookmark-root");
    fs::write(root.join("notes.txt"), "line 00\n").expect("should create notes file");
    let missing_root = root.join("alpha");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x09, 0x0d]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.config.bookmarks.pins = vec![missing_root.clone()];

    let mut watcher = WatcherRuntime::new(
        app.config.watcher.clone(),
        FakeWatcherService {
            fail_on_missing_root: true,
            ..FakeWatcherService::default()
        },
    );
    let result = bootstrap::run_with_terminal_and_reader_and_watcher(
        &mut terminal,
        &mut input,
        &mut app,
        &mut watcher,
    );

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.focus, Focus::Roots);
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.active_tab, 0);
    assert!(
        app.status.message.contains("missing root"),
        "missing pinned root should surface a warning, got: {}",
        app.status.message
    );
    assert!(
        watcher
            .service_ref()
            .sync_history
            .iter()
            .all(|roots| roots == &vec![normalize_watched_root(&root)]),
        "watcher should never try to watch the missing pinned root"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn root_labels_share_collision_disambiguation_for_tabs_and_bookmarks() {
    let root = make_temp_dir("grove-bootstrap-root-labels");
    let alpha_root = root.join("workspace-a").join("client");
    let beta_root = root.join("workspace-b").join("client");
    fs::create_dir_all(&alpha_root).expect("should create alpha root");
    fs::create_dir_all(&beta_root).expect("should create beta root");

    let mut app = App::default();
    app.tabs[0] = TabState::new(alpha_root.clone());
    assert!(app.open_or_activate_tab(beta_root.clone()));
    app.config.bookmarks.pins = vec![alpha_root.clone(), beta_root.clone()];

    let first_tab = app.tab_label(0).expect("first tab label should exist");
    let second_tab = app.tab_label(1).expect("second tab label should exist");
    let first_bookmark = app
        .bookmark_label(0)
        .expect("first bookmark label should exist");
    let second_bookmark = app
        .bookmark_label(1)
        .expect("second bookmark label should exist");

    assert_eq!(first_tab, first_bookmark);
    assert_eq!(second_tab, second_bookmark);
    assert_ne!(first_tab, second_tab);
    assert!(first_tab.contains("client"));
    assert!(second_tab.contains("client"));
    assert!(first_tab.contains("workspace-a"));
    assert!(second_tab.contains("workspace-b"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_b_toggles_the_active_root_bookmark_and_persists_it() {
    let root = make_temp_dir("grove-bootstrap-bookmark-toggle-runtime");
    let config_home = make_temp_dir("grove-bootstrap-bookmark-config-home");
    let config_path = config_home.join("grove").join("config.toml");
    fs::write(root.join("notes.txt"), "notes\n").expect("should create notes file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.set_config_path(config_path.clone());
    app.tabs[0] = TabState::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("root should canonicalize");

    let result = bootstrap::run_with_terminal_and_reader(
        &mut terminal,
        &mut Cursor::new(vec![b'b']),
        &mut app,
    );
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(app.bookmark_paths(), std::slice::from_ref(&canonical_root));
    assert_eq!(app.status.severity, grove::state::StatusSeverity::Success);
    assert_eq!(app.status.message, "active root pinned");

    let saved = fs::read_to_string(&config_path).expect("bookmark toggle should persist");
    assert!(
        saved.contains(&canonical_root.display().to_string()),
        "saved config should contain the pinned root, got:\n{saved}"
    );

    let result = bootstrap::run_with_terminal_and_reader(
        &mut terminal,
        &mut Cursor::new(vec![b'b']),
        &mut app,
    );
    assert!(result.is_ok(), "{result:?}");
    assert!(app.bookmark_paths().is_empty());
    assert_eq!(app.status.severity, grove::state::StatusSeverity::Success);
    assert_eq!(app.status.message, "active root unpinned");

    let saved = fs::read_to_string(&config_path).expect("bookmark removal should persist");
    assert!(
        !saved.contains(&canonical_root.display().to_string()),
        "saved config should remove the pinned root, got:\n{saved}"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
    fs::remove_dir_all(config_home).expect("config home should be removed");
}

#[test]
fn runtime_b_stays_root_based_even_with_a_file_selected() {
    let root = make_temp_dir("grove-bootstrap-bookmark-root-based");
    let config_home = make_temp_dir("grove-bootstrap-bookmark-root-based-config");
    let config_path = config_home.join("grove").join("config.toml");
    fs::write(root.join("alpha.txt"), "alpha\n").expect("should create alpha file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.set_config_path(config_path);
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("alpha.txt")),
        "file selection should exist"
    );

    let result = bootstrap::run_with_terminal_and_reader(
        &mut terminal,
        &mut Cursor::new(vec![b'b']),
        &mut app,
    );
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        app.bookmark_paths(),
        &[fs::canonicalize(&root).expect("root should canonicalize")]
    );
    assert_ne!(
        app.bookmark_paths()[0],
        fs::canonicalize(root.join("alpha.txt")).expect("file should canonicalize")
    );
    assert_eq!(app.status.severity, grove::state::StatusSeverity::Success);
    assert_eq!(app.status.message, "active root pinned");

    fs::remove_dir_all(root).expect("temp root should be removed");
    fs::remove_dir_all(config_home).expect("config home should be removed");
}

#[test]
fn runtime_b_pins_the_selected_directory_as_a_root_bookmark() {
    let root = make_temp_dir("grove-bootstrap-bookmark-selected-directory");
    let config_home = make_temp_dir("grove-bootstrap-bookmark-selected-directory-config");
    let config_path = config_home.join("grove").join("config.toml");
    fs::create_dir_all(root.join("docs")).expect("docs directory should exist");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.set_config_path(config_path.clone());
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("docs")),
        "directory selection should exist"
    );

    let result = bootstrap::run_with_terminal_and_reader(
        &mut terminal,
        &mut Cursor::new(vec![b'b']),
        &mut app,
    );
    assert!(result.is_ok(), "{result:?}");

    let canonical_root = fs::canonicalize(&root).expect("root should canonicalize");
    let canonical_docs = fs::canonicalize(root.join("docs")).expect("docs should canonicalize");
    assert_eq!(app.bookmark_paths(), std::slice::from_ref(&canonical_docs));
    assert_ne!(app.bookmark_paths()[0], canonical_root);
    assert_eq!(app.status.severity, grove::state::StatusSeverity::Success);
    assert_eq!(app.status.message, "selected root pinned");

    let saved = fs::read_to_string(&config_path).expect("bookmark toggle should persist");
    assert!(
        saved.contains(&canonical_docs.display().to_string()),
        "saved config should contain the pinned selected root, got:\n{saved}"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
    fs::remove_dir_all(config_home).expect("config home should be removed");
}

#[test]
fn runtime_preview_focus_scrolls_long_file_down() {
    let root = make_temp_dir("grove-bootstrap-preview-scroll-down");
    let body = (0..40)
        .map(|idx| format!("line {idx:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(root.join("notes.txt"), body).expect("should create notes file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x1b, b'[', b'B', 0x09, 0x09, 0x1b, b'[', b'B', b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Preview);
    assert_eq!(app.tabs[0].preview.scroll_row, 1);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_preview_scrolling_preserves_preview_payload_semantics() {
    let root = make_temp_dir("grove-bootstrap-preview-semantics");
    let body = (0..60)
        .map(|idx| format!("line {idx:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(root.join("notes.txt"), &body).expect("should create notes file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![
        0x1b, b'[', b'B', 0x09, 0x09, 0x1b, b'[', b'B', 0x1b, b'[', b'B', 0x1b, b'[', b'A', b'q',
    ]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());

    let preview = &app.tabs[0].preview;
    assert_eq!(preview.payload.title, "notes.txt");
    assert!(preview.payload.markdown.is_none());
    assert!(
        preview
            .payload
            .lines
            .iter()
            .any(|line| line.contains("line 00")),
        "preview payload should still contain the first body line after repeated scrolling"
    );
    assert!(
        preview
            .payload
            .lines
            .iter()
            .any(|line| line.contains("line 59")),
        "preview payload should still contain the last body line after repeated scrolling"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_preview_page_down_scrolls_by_more_than_one_line() {
    let root = make_temp_dir("grove-bootstrap-preview-page-down");
    let body = (0..80)
        .map(|idx| format!("line {idx:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(root.join("notes.txt"), body).expect("should create notes file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![
        0x1b, b'[', b'B', 0x09, 0x09, 0x1b, b'[', b'6', b'~', b'q',
    ]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Preview);
    assert!(app.tabs[0].preview.scroll_row > 1);

    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    let expected_visible_line = format!("line {:02}", app.tabs[0].preview.scroll_row);
    assert!(
        rendered.contains(&expected_visible_line),
        "page-down preview should render a later body line, expected {expected_visible_line}"
    );
    assert!(
        !rendered.contains("line 00"),
        "page-down preview should advance past the first body line"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_preview_end_jumps_to_bottom_and_home_returns_to_top() {
    let root = make_temp_dir("grove-bootstrap-preview-home-end");
    let body = (0..80)
        .map(|idx| format!("line {idx:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(root.join("notes.txt"), body).expect("should create notes file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![
        0x1b, b'[', b'B', 0x09, 0x09, 0x1b, b'[', b'F', 0x1b, b'[', b'H', b'q',
    ]);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;

    let result = bootstrap::run_with_terminal_and_reader(&mut terminal, &mut input, &mut app);
    assert!(result.is_ok());
    assert_eq!(app.focus, Focus::Preview);
    assert_eq!(app.tabs[0].preview.scroll_row, 0);

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_missing_selected_path_falls_back_to_nearest_sibling_with_status() {
    let root = make_temp_dir("grove-bootstrap-watcher-sibling-fallback");
    fs::write(root.join("alpha.txt"), "alpha").expect("should create alpha");
    fs::write(root.join("beta.txt"), "beta").expect("should create beta");
    fs::write(root.join("gamma.txt"), "gamma").expect("should create gamma");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("beta.txt"))
    );

    fs::remove_file(root.join("beta.txt")).expect("beta should be removed");
    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            removed_paths: vec![std::path::PathBuf::from("beta.txt")],
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should update app state");
    assert_eq!(
        app.tabs[0].tree.selected_rel_path(),
        Some(std::path::PathBuf::from("gamma.txt"))
    );
    assert!(app.status.message.contains("beta.txt"));
    assert!(app.status.message.contains("gamma.txt"));

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_invalidates_and_rebuilds_the_selected_preview_file() {
    let root = make_temp_dir("grove-bootstrap-watcher-preview-refresh");
    fs::write(root.join("notes.txt"), "before\nline two").expect("should create notes file");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("notes.txt"))
    );
    assert!(app.refresh_active_preview());
    assert!(
        app.tabs[0]
            .preview
            .payload
            .lines
            .iter()
            .any(|line| line.contains("before"))
    );

    fs::write(root.join("notes.txt"), "after\nline two").expect("should update notes file");
    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            changed_paths: vec![std::path::PathBuf::from("notes.txt")],
            git_dirty: true,
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should update app state");
    assert!(app.tabs[0].git.needs_refresh);
    assert!(
        app.tabs[0].preview.source.rel_path.is_none(),
        "watcher refresh should invalidate the selected preview before the next render"
    );
    assert!(
        app.refresh_active_preview(),
        "watcher refresh should allow the preview to rebuild"
    );
    assert_eq!(
        app.tabs[0].preview.source.rel_path,
        Some(std::path::PathBuf::from("notes.txt"))
    );
    assert!(
        app.tabs[0]
            .preview
            .payload
            .lines
            .iter()
            .any(|line| line.contains("after"))
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_keeps_preview_stable_when_selected_target_is_untouched() {
    let root = make_temp_dir("grove-bootstrap-watcher-preview-stable");
    fs::write(root.join("notes.txt"), "before\nline two").expect("should create notes file");
    fs::write(root.join("other.txt"), "other\n").expect("should create sibling file");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("notes.txt"))
    );
    assert!(app.refresh_active_preview());
    assert_eq!(
        app.tabs[0].preview.source.rel_path,
        Some(std::path::PathBuf::from("notes.txt"))
    );

    fs::write(root.join("other.txt"), "changed\n").expect("should update sibling file");
    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            changed_paths: vec![std::path::PathBuf::from("other.txt")],
            git_dirty: true,
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should still update app state");
    assert_eq!(
        app.tabs[0].preview.source.rel_path,
        Some(std::path::PathBuf::from("notes.txt")),
        "unrelated file changes should not evict the selected preview"
    );
    assert!(
        !app.refresh_active_preview(),
        "stable preview should not need to rebuild for unrelated watcher changes"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_on_unrelated_path_preserves_selected_preview_context() {
    let root = make_temp_dir("grove-bootstrap-watcher-preview-unrelated");
    fs::write(root.join("notes.txt"), "before\nline two").expect("should create notes file");
    fs::write(root.join("other.txt"), "other").expect("should create sibling file");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("notes.txt"))
    );
    assert!(app.refresh_active_preview());
    app.tabs[0].preview.scroll_row = 1;
    app.tabs[0].preview.cursor_line = 1;

    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            changed_paths: vec![std::path::PathBuf::from("other.txt")],
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should update app state");
    assert_eq!(
        app.tabs[0].preview.source.rel_path,
        Some(std::path::PathBuf::from("notes.txt"))
    );
    assert_eq!(app.tabs[0].preview.scroll_row, 1);
    assert_eq!(app.tabs[0].preview.cursor_line, 1);
    assert!(
        !app.refresh_active_preview(),
        "unrelated watcher refreshes should not invalidate the selected preview"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_falls_back_from_diff_mode_when_the_file_loses_its_diff() {
    let root = make_temp_dir("grove-bootstrap-watcher-diff-fallback");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "before\n");
    commit_repo_all(&repo, "initial");
    write_repo_file(&root, "tracked.txt", "after\n");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("tracked.txt"))
    );
    app.refresh_active_git_state()
        .expect("git state should load");
    assert!(app.activate_diff_mode_if_available());
    assert!(app.refresh_active_preview());
    assert_eq!(app.tabs[0].mode, ContextMode::Diff);
    assert!(app.tabs[0].preview.payload.title.starts_with("Diff "));
    assert!(
        app.tabs[0]
            .preview
            .payload
            .lines
            .iter()
            .any(|line| line.contains("+after"))
    );

    write_repo_file(&root, "tracked.txt", "before\n");
    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            changed_paths: vec![std::path::PathBuf::from("tracked.txt")],
            git_dirty: true,
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should update app state");
    assert_eq!(app.tabs[0].mode, ContextMode::Preview);
    assert_eq!(
        app.status.message,
        "diff unavailable: select a modified or untracked file"
    );
    assert!(
        app.tabs[0].preview.source.rel_path.is_none(),
        "watcher refresh should invalidate the stale diff preview"
    );
    assert!(app.refresh_active_preview());
    assert_eq!(app.tabs[0].preview.payload.title, "tracked.txt");
    assert!(
        app.tabs[0]
            .preview
            .payload
            .lines
            .iter()
            .any(|line| line.contains("before"))
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_falls_back_from_diff_mode_after_git_index_change() {
    let root = make_temp_dir("grove-bootstrap-watcher-diff-index-change");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "before\n");
    commit_repo_all(&repo, "initial");
    write_repo_file(&root, "tracked.txt", "after\n");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("tracked.txt"))
    );
    app.refresh_active_git_state()
        .expect("git state should load");
    assert!(app.activate_diff_mode_if_available());
    assert!(app.refresh_active_preview());
    assert_eq!(app.tabs[0].mode, ContextMode::Diff);

    stage_repo_path(&repo, "tracked.txt");
    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            changed_paths: vec![std::path::PathBuf::from(".git/index")],
            git_dirty: true,
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should update app state");
    assert_eq!(app.tabs[0].mode, ContextMode::Preview);
    assert_eq!(
        app.status.message,
        "diff unavailable: select a modified or untracked file"
    );
    assert!(
        app.tabs[0].preview.source.rel_path.is_none(),
        "stale diff preview should be invalidated after git index changes"
    );
    assert!(app.refresh_active_preview());
    assert_eq!(app.tabs[0].preview.payload.title, "tracked.txt");
    assert!(
        app.tabs[0]
            .preview
            .payload
            .lines
            .iter()
            .any(|line| line.contains("after"))
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn render_shell_falls_back_from_diff_mode_after_git_only_watcher_refresh() {
    let root = make_temp_dir("grove-bootstrap-watcher-diff-git-only");
    let repo = Repository::init(&root).expect("repo should initialize");
    write_repo_file(&root, "tracked.txt", "before\n");
    commit_repo_all(&repo, "initial");
    write_repo_file(&root, "tracked.txt", "after\n");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("tracked.txt"))
    );
    app.refresh_active_git_state()
        .expect("git state should load");
    assert!(app.activate_diff_mode_if_available());
    assert!(app.refresh_active_preview());
    assert_eq!(app.tabs[0].mode, ContextMode::Diff);

    let mut index = repo.index().expect("index should open");
    index
        .add_path(std::path::Path::new("tracked.txt"))
        .expect("tracked file should stage");
    index.write().expect("index should flush");

    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            git_dirty: true,
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should update app state");
    bootstrap::render_shell_once(&mut terminal, &mut app).expect("render should succeed");

    assert_eq!(app.tabs[0].mode, ContextMode::Preview);
    assert_eq!(
        app.status.message,
        "diff unavailable: select a modified or untracked file"
    );
    assert_eq!(app.tabs[0].preview.payload.title, "tracked.txt");
    assert!(
        app.tabs[0]
            .preview
            .payload
            .lines
            .iter()
            .any(|line| line.contains("after"))
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_invalidates_content_search_results_without_clearing_the_query() {
    let root = make_temp_dir("grove-bootstrap-watcher-search-refresh");
    fs::write(root.join("notes.txt"), "needle in haystack\nother line\n")
        .expect("should create notes file");

    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    app.tabs[0].content_search.active = true;
    app.tabs[0].content_search.query = "needle".to_string();
    app.tabs[0].content_search.status = grove::app::ContentSearchStatus::Ready;
    app.tabs[0].content_search.status_message = Some("1 result".to_string());
    app.tabs[0].content_search.payload = grove::preview::model::SearchPayload {
        query: "needle".to_string(),
        hits: vec![grove::preview::model::SearchHit {
            path: "notes.txt".to_string(),
            line: 1,
            excerpt: "needle in haystack".to_string(),
        }],
    };
    app.tabs[0].content_search.selected_hit_index = Some(0);
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("notes.txt"))
    );

    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: root.clone(),
            changed_paths: vec![std::path::PathBuf::from("notes.txt")],
            git_dirty: true,
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should succeed");

    assert!(changed, "watcher refresh should update app state");
    assert_eq!(app.tabs[0].content_search.query, "needle");
    assert!(app.tabs[0].content_search.payload.hits.is_empty());
    assert_eq!(app.tabs[0].content_search.selected_hit_index, None);
    assert!(app.tabs[0].content_search.active);
    assert_eq!(
        app.tabs[0].content_search.status,
        grove::app::ContentSearchStatus::Idle
    );
    assert!(app.tabs[0].content_search.status_message.is_none());
    assert!(app.tabs[0].content_search.runtime.worker.is_none());

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_watcher_reconciles_open_roots_and_applies_refresh_plans() {
    let root = make_temp_dir("grove-bootstrap-watcher-runtime-root");
    let second_root = make_temp_dir("grove-bootstrap-watcher-runtime-second-root");
    let bookmarked_root = make_temp_dir("grove-bootstrap-watcher-runtime-bookmarked-root");
    fs::write(root.join("keep.txt"), "keep").expect("should create keep file");
    fs::write(root.join("gone.txt"), "gone").expect("should create selected file");
    fs::write(second_root.join("other.txt"), "other").expect("should create second root file");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(b"q".to_vec());
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(app.open_selected_directory_as_root_tab(second_root.clone()));
    app.active_tab = 0;
    app.config.bookmarks.pins.push(bookmarked_root.clone());
    app.tabs[0].path_index.receiver = None;
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Ready;
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("gone.txt"))
    );
    assert!(app.refresh_active_preview());

    let mut watcher =
        WatcherRuntime::new(app.config.watcher.clone(), FakeWatcherService::default());
    watcher.service_mut().plans.push(RefreshPlan {
        root: root.clone(),
        removed_paths: vec![std::path::PathBuf::from("gone.txt")],
        git_dirty: true,
        ..RefreshPlan::default()
    });
    fs::remove_file(root.join("gone.txt")).expect("selected file should be removed");

    let result = bootstrap::run_with_terminal_and_reader_and_watcher(
        &mut terminal,
        &mut input,
        &mut app,
        &mut watcher,
    );
    assert!(result.is_ok(), "{result:?}");

    assert_eq!(
        watcher.service_ref().synced_roots,
        vec![
            normalize_watched_root(&root),
            normalize_watched_root(&second_root),
        ]
    );
    assert!(
        !watcher
            .service_ref()
            .synced_roots
            .iter()
            .any(|watched| watched == &bookmarked_root),
        "bookmarks that are not open should not be watched"
    );
    assert_eq!(
        app.tabs[0].tree.selected_rel_path(),
        Some(std::path::PathBuf::from("keep.txt"))
    );
    assert!(
        app.status
            .message
            .contains("watcher refresh recovered selection")
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
    fs::remove_dir_all(second_root).expect("temp root should be removed");
    fs::remove_dir_all(bookmarked_root).expect("temp root should be removed");
}

#[test]
fn runtime_watcher_reconciles_open_roots_after_ctrl_t_creates_a_new_root_tab() {
    let root = make_temp_dir("grove-bootstrap-watcher-runtime-ctrl-t");
    let docs_root = root.join("docs");
    fs::create_dir_all(&docs_root).expect("docs root should exist");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(vec![0x14, b'q']);
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("docs")),
        "docs directory should be selectable before Ctrl+T"
    );

    let mut watcher =
        WatcherRuntime::new(app.config.watcher.clone(), FakeWatcherService::default());
    let result = bootstrap::run_with_terminal_and_reader_and_watcher(
        &mut terminal,
        &mut input,
        &mut app,
        &mut watcher,
    );
    assert!(result.is_ok(), "{result:?}");

    let expected_initial = vec![normalize_watched_root(&root)];
    let expected_after_open = vec![
        normalize_watched_root(&root),
        normalize_watched_root(&docs_root),
    ];
    assert_eq!(
        watcher.service_ref().sync_history.first(),
        Some(&expected_initial)
    );
    assert!(
        watcher
            .service_ref()
            .sync_history
            .iter()
            .any(|roots| roots == &expected_after_open),
        "watcher reconciliation should include the newly opened root tab"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

#[test]
fn runtime_watcher_closes_missing_active_root_and_reconciles_watched_roots() {
    let root = make_temp_dir("grove-bootstrap-watcher-runtime-missing-root");
    let second_root = make_temp_dir("grove-bootstrap-watcher-runtime-missing-root-second");
    fs::write(root.join("keep.txt"), "keep").expect("root file should exist");
    fs::write(second_root.join("other.txt"), "other").expect("second root file should exist");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(b"q".to_vec());
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(app.open_selected_directory_as_root_tab(second_root.clone()));
    app.active_tab = 0;

    let mut watcher =
        WatcherRuntime::new(app.config.watcher.clone(), FakeWatcherService::default());
    watcher.service_mut().plans.push(RefreshPlan {
        root: root.clone(),
        ..RefreshPlan::default()
    });
    fs::remove_dir_all(&root).expect("missing root should be removed");

    let result = bootstrap::run_with_terminal_and_reader_and_watcher(
        &mut terminal,
        &mut input,
        &mut app,
        &mut watcher,
    );
    assert!(result.is_ok(), "{result:?}");

    assert_eq!(app.tabs.len(), 1, "missing root tab should be removed");
    assert_eq!(app.active_tab, 0, "remaining tab should become active");
    assert_eq!(app.tabs[0].root, normalize_watched_root(&second_root));
    assert!(
        watcher
            .service_ref()
            .sync_history
            .iter()
            .any(|roots| roots == &vec![normalize_watched_root(&second_root)]),
        "watcher reconciliation should drop the missing root immediately"
    );
    assert!(
        app.status
            .message
            .contains("watcher refresh closed missing root")
    );

    fs::remove_dir_all(second_root).expect("temp root should be removed");
}

#[test]
fn watcher_refresh_recovers_last_missing_root_to_surviving_parent() {
    let workspace_root = make_temp_dir("grove-bootstrap-watcher-last-missing-root");
    let missing_root = workspace_root.join("docs");
    fs::create_dir_all(&missing_root).expect("missing root should exist");
    fs::write(missing_root.join("notes.txt"), "hello").expect("missing-root file should exist");

    let mut app = App::default();
    app.tabs[0] = TabState::new(missing_root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("notes.txt"))
    );
    assert!(app.refresh_active_preview());

    fs::remove_dir_all(&missing_root).expect("missing root should be removed");

    let changed = app
        .apply_watcher_refresh_plan(&RefreshPlan {
            root: missing_root.clone(),
            ..RefreshPlan::default()
        })
        .expect("watcher refresh should recover the missing last root");

    assert!(changed, "watcher refresh should update app state");
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].root, normalize_watched_root(&workspace_root));
    assert_eq!(
        app.tabs[0].tree.selected_rel_path(),
        Some(std::path::PathBuf::from("."))
    );
    assert!(
        app.status
            .message
            .contains("watcher refresh recovered missing root")
    );

    fs::remove_dir_all(workspace_root).expect("temp root should be removed");
}

#[test]
fn runtime_watcher_tick_runs_path_index_content_search_and_refresh_plan_work_together() {
    let root = make_temp_dir("grove-bootstrap-watcher-runtime-background-work");
    fs::write(root.join("keep.txt"), "needle in keep\n").expect("keep file should exist");
    fs::write(root.join("gone.txt"), "gone\n").expect("selected file should exist");

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    let mut input = Cursor::new(b"q".to_vec());
    let mut app = App::default();
    app.tabs[0] = TabState::new(root.clone());
    assert!(
        app.tabs[0]
            .tree
            .select_rel_path(std::path::Path::new("gone.txt"))
    );
    assert!(app.refresh_active_preview());

    let (sender, receiver) = mpsc::channel();
    sender
        .send(PathIndexEvent::Complete)
        .expect("path-index completion should queue");
    app.tabs[0].path_index.receiver = Some(receiver);
    app.tabs[0].path_index.status = grove::app::PathIndexStatus::Building { indexed_paths: 1 };
    app.tabs[0].path_index.snapshot =
        build_snapshot_with_visibility(&root, app.tabs[0].tree.visibility_settings())
            .expect("path-index snapshot should build");

    app.tabs[0].content_search.active = true;
    app.tabs[0].content_search.query = "needle".to_string();
    app.tabs[0].content_search.status = grove::app::ContentSearchStatus::Searching;
    let search_worker = start_background_content_search();
    assert!(
        search_worker.submit(ContentSearchRequest {
            generation: app.tabs[0].content_search.generation,
            root_abs: root.clone(),
            snapshot: app.tabs[0].path_index.snapshot.clone(),
            query: app.tabs[0].content_search.query.clone(),
            max_results: 10,
        }),
        "content-search work should queue"
    );
    app.tabs[0].content_search.runtime.worker = Some(search_worker);

    let mut watcher =
        WatcherRuntime::new(app.config.watcher.clone(), FakeWatcherService::default());
    watcher.service_mut().plans.push(RefreshPlan {
        root: root.clone(),
        removed_paths: vec![std::path::PathBuf::from("gone.txt")],
        git_dirty: true,
        ..RefreshPlan::default()
    });
    fs::remove_file(root.join("gone.txt")).expect("selected file should be removed");

    let result = bootstrap::run_with_terminal_and_reader_and_watcher(
        &mut terminal,
        &mut input,
        &mut app,
        &mut watcher,
    );
    assert!(result.is_ok(), "{result:?}");

    assert_eq!(
        app.tabs[0].path_index.status,
        grove::app::PathIndexStatus::Ready
    );
    assert_eq!(
        app.tabs[0].tree.selected_rel_path(),
        Some(std::path::PathBuf::from("keep.txt"))
    );
    assert_eq!(app.tabs[0].content_search.query, "needle");
    assert!(
        app.tabs[0].content_search.active,
        "the active content-search workflow should survive the watcher-driven runtime tick"
    );
    assert!(
        !matches!(
            app.tabs[0].content_search.status,
            grove::app::ContentSearchStatus::Error
        ),
        "watcher polling alongside content-search work must not drop the search into an error state"
    );

    fs::remove_dir_all(root).expect("temp root should be removed");
}

fn visible_rel_paths(tab: &TabState) -> Vec<std::path::PathBuf> {
    tab.tree
        .visible_rows
        .iter()
        .filter_map(|row| tab.tree.node(row.node_id).map(|node| node.rel_path.clone()))
        .collect()
}

fn cells_for_ascii_text<'a>(buffer: &'a Buffer, needle: &str) -> Option<Vec<&'a Cell>> {
    let width = buffer.area.width as usize;

    for row in buffer.content.chunks(width) {
        let rendered = row.iter().map(|cell| cell.symbol()).collect::<String>();
        if let Some(start) = rendered.find(needle) {
            let end = start + needle.len();
            return Some(row[start..end].iter().collect());
        }
    }

    None
}

fn ascii_text_positions(buffer: &Buffer, needle: &str) -> Option<Vec<(usize, usize)>> {
    let width = buffer.area.width as usize;
    let mut positions = Vec::new();

    for (row_index, row) in buffer.content.chunks(width).enumerate() {
        let rendered = row.iter().map(|cell| cell.symbol()).collect::<String>();
        let mut search_start = 0usize;
        while let Some(offset) = rendered[search_start..].find(needle) {
            let column = search_start + offset;
            positions.push((row_index, column));
            search_start = column + needle.len();
        }
    }

    if positions.is_empty() {
        None
    } else {
        Some(positions)
    }
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

fn write_tiny_png(path: &std::path::Path) {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+iP1kAAAAASUVORK5CYII=")
        .expect("tiny png fixture should decode");
    fs::write(path, bytes).expect("tiny png fixture should be written");
}

fn with_test_trash_dir<T>(trash_dir: &std::path::Path, f: impl FnOnce() -> T) -> T {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _lock = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("trash-dir lock should not be poisoned");
    let previous = std::env::var_os("GROVE_TRASH_DIR");
    unsafe { std::env::set_var("GROVE_TRASH_DIR", trash_dir) };
    let restore = EnvRestore {
        key: "GROVE_TRASH_DIR",
        previous,
    };
    let result = f();
    drop(restore);
    result
}

struct EnvRestore {
    key: &'static str,
    previous: Option<OsString>,
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            unsafe { std::env::set_var(self.key, previous) };
        } else {
            unsafe { std::env::remove_var(self.key) };
        }
    }
}

fn write_repo_file(root: &std::path::Path, rel_path: &str, contents: &str) {
    let path = root.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, contents).expect("repo file should be written");
}

#[derive(Default)]
struct FakeWatcherService {
    synced_roots: Vec<std::path::PathBuf>,
    sync_history: Vec<Vec<std::path::PathBuf>>,
    plans: Vec<RefreshPlan>,
    fail_on_missing_root: bool,
}

impl WatcherService for FakeWatcherService {
    fn reconcile_open_roots(&mut self, roots: &[std::path::PathBuf]) -> grove::error::Result<bool> {
        if self.fail_on_missing_root
            && let Some(root) = roots.iter().find(|root| !root.exists())
        {
            return Err(
                std::io::Error::other(format!("missing watched root: {}", root.display())).into(),
            );
        }
        let next_roots = roots.to_vec();
        let changed = self.synced_roots != next_roots;
        self.synced_roots = next_roots.clone();
        self.sync_history.push(next_roots);
        Ok(changed)
    }

    fn poll_refresh_plans(&mut self) -> grove::error::Result<Vec<RefreshPlan>> {
        Ok(std::mem::take(&mut self.plans))
    }
}

fn commit_repo_all(repo: &Repository, message: &str) {
    let mut index = repo.index().expect("index should open");
    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .expect("repo contents should stage");
    index.write().expect("index should flush");

    let tree_oid = index.write_tree().expect("tree should write");
    let tree = repo.find_tree(tree_oid).expect("tree should load");
    let signature =
        Signature::now("Grove Tests", "grove-tests@example.com").expect("signature should build");

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

fn stage_repo_path(repo: &Repository, rel_path: &str) {
    let mut index = repo.index().expect("index should open");
    index
        .add_path(std::path::Path::new(rel_path))
        .expect("path should stage");
    index.write().expect("index should flush");
}

fn create_merge_conflict(repo: &Repository, root: &std::path::Path, rel_path: &str) {
    let base_ref = repo
        .head()
        .expect("head should exist")
        .name()
        .expect("head name should exist")
        .to_string();
    let base_commit = repo
        .head()
        .expect("head should exist")
        .peel_to_commit()
        .expect("head commit should load");
    repo.branch("feature-conflict", &base_commit, false)
        .expect("feature branch should be created");

    write_repo_file(root, rel_path, "main change\n");
    commit_repo_all(repo, "main change");

    repo.set_head("refs/heads/feature-conflict")
        .expect("head should switch to feature");
    repo.checkout_head(Some(CheckoutBuilder::new().force()))
        .expect("feature checkout should succeed");
    write_repo_file(root, rel_path, "feature change\n");
    commit_repo_all(repo, "feature change");

    repo.set_head(&base_ref)
        .expect("head should switch back to base branch");
    repo.checkout_head(Some(CheckoutBuilder::new().force()))
        .expect("base checkout should succeed");

    let feature_commit = repo
        .find_reference("refs/heads/feature-conflict")
        .expect("feature ref should exist")
        .peel_to_commit()
        .expect("feature commit should load");
    let annotated = repo
        .find_annotated_commit(feature_commit.id())
        .expect("annotated commit should load");
    repo.merge(&[&annotated], None, None)
        .expect("merge should produce conflicts");

    assert!(
        repo.index()
            .expect("index should open after merge")
            .has_conflicts(),
        "repo should contain an index conflict"
    );
}

fn mark_editor_open<B: ratatui::backend::Backend>(
    _terminal: &mut Terminal<B>,
    app: &mut App,
) -> grove::error::Result<bool>
where
    B::Error: std::fmt::Display,
{
    app.status.message = "editor opened".to_string();
    Ok(true)
}

fn mark_external_open<B: ratatui::backend::Backend>(
    _terminal: &mut Terminal<B>,
    app: &mut App,
) -> grove::error::Result<bool>
where
    B::Error: std::fmt::Display,
{
    app.status.message = "external open".to_string();
    Ok(true)
}

fn ignore_external_open<B: ratatui::backend::Backend>(
    _terminal: &mut Terminal<B>,
    _app: &mut App,
) -> grove::error::Result<bool>
where
    B::Error: std::fmt::Display,
{
    Ok(false)
}

fn ignore_bridge_initialize(_app: &mut App) -> grove::error::Result<()> {
    Ok(())
}

fn ignore_list_sessions(_app: &mut App) -> grove::error::Result<Vec<SessionSummary>> {
    Ok(Vec::new())
}

fn bridge_list_error(app: &mut App) -> grove::error::Result<Vec<SessionSummary>> {
    app.status.message = "bridge session list failed: boom".to_string();
    Ok(Vec::new())
}

fn ignore_send_text(
    _app: &mut App,
    _target: SendTarget,
    _text: String,
    _append_newline: bool,
) -> grove::error::Result<BridgeResponse> {
    Ok(BridgeResponse::Error {
        message: "ignored".to_string(),
    })
}

fn ignore_set_role(
    _app: &mut App,
    _session_id: String,
    _role: TargetRole,
) -> grove::error::Result<BridgeResponse> {
    Ok(BridgeResponse::Error {
        message: "ignored".to_string(),
    })
}

fn panic_set_role(
    _app: &mut App,
    _session_id: String,
    _role: TargetRole,
) -> grove::error::Result<BridgeResponse> {
    panic!("set_role should not be called for current-pane editor target selection")
}

fn mark_bridge_initialized(app: &mut App) -> grove::error::Result<()> {
    app.bridge.connected = true;
    app.bridge.instance_id = Some("instance-1".to_string());
    Ok(())
}

fn list_target_sessions(_app: &mut App) -> grove::error::Result<Vec<SessionSummary>> {
    Ok(vec![
        SessionSummary {
            session_id: "ai-session".to_string(),
            title: "Claude".to_string(),
            role: Some(TargetRole::Ai),
            job_name: Some("claude".to_string()),
            command_line: Some("claude".to_string()),
            cwd: Some("/repo".to_string()),
            location_hint: Some(SessionLocationHint {
                window_id: Some("window-1".to_string()),
                tab_id: Some("tab-2".to_string()),
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
                tab_id: Some("tab-3".to_string()),
                window_title: Some("Workspace".to_string()),
                tab_title: Some("Editor".to_string()),
            }),
        },
    ])
}

fn send_relative_path_ok(
    _app: &mut App,
    target: SendTarget,
    text: String,
    append_newline: bool,
) -> grove::error::Result<BridgeResponse> {
    assert_eq!(target, SendTarget::Role(TargetRole::Ai));
    assert_eq!(text, "note.txt");
    assert!(!append_newline);
    Ok(BridgeResponse::SendOk {
        target_session_id: "ai-session".to_string(),
    })
}

fn send_relative_path_to_explicit_ai_session_ok(
    _app: &mut App,
    target: SendTarget,
    text: String,
    append_newline: bool,
) -> grove::error::Result<BridgeResponse> {
    assert_eq!(
        target,
        SendTarget::SessionId("bound-ai-session".to_string())
    );
    assert_eq!(text, "note.txt");
    assert!(!append_newline);
    Ok(BridgeResponse::SendOk {
        target_session_id: "bound-ai-session".to_string(),
    })
}

fn send_explicit_ai_target_unavailable(
    _app: &mut App,
    target: SendTarget,
    text: String,
    append_newline: bool,
) -> grove::error::Result<BridgeResponse> {
    assert_eq!(
        target,
        SendTarget::SessionId("stale-ai-session".to_string())
    );
    assert_eq!(text, "note.txt");
    assert!(!append_newline);
    Ok(BridgeResponse::TargetSessionUnavailable {
        session_id: "stale-ai-session".to_string(),
    })
}

fn send_capture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn sent_texts() -> &'static Mutex<Vec<String>> {
    static TEXTS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    TEXTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn clear_sent_texts() {
    sent_texts()
        .lock()
        .expect("sent texts lock should acquire")
        .clear();
}

fn take_sent_texts() -> Vec<String> {
    let mut guard = sent_texts().lock().expect("sent texts lock should acquire");
    std::mem::take(&mut *guard)
}

fn capture_relative_paths_ok(
    _app: &mut App,
    target: SendTarget,
    text: String,
    append_newline: bool,
) -> grove::error::Result<BridgeResponse> {
    assert_eq!(target, SendTarget::Role(TargetRole::Ai));
    assert!(!append_newline);
    sent_texts()
        .lock()
        .expect("sent texts lock should acquire")
        .push(text);
    Ok(BridgeResponse::SendOk {
        target_session_id: "ai-session".to_string(),
    })
}

fn capture_relative_paths_requires_manual_ai(
    _app: &mut App,
    target: SendTarget,
    text: String,
    append_newline: bool,
) -> grove::error::Result<BridgeResponse> {
    assert_eq!(target, SendTarget::Role(TargetRole::Ai));
    assert!(!append_newline);
    sent_texts()
        .lock()
        .expect("sent texts lock should acquire")
        .push(text);
    Ok(BridgeResponse::ManualSelectionRequired {
        role: TargetRole::Ai,
    })
}

fn send_requires_manual_ai(
    _app: &mut App,
    target: SendTarget,
    text: String,
    append_newline: bool,
) -> grove::error::Result<BridgeResponse> {
    assert_eq!(target, SendTarget::Role(TargetRole::Ai));
    assert_eq!(text, "note.txt");
    assert!(!append_newline);
    Ok(BridgeResponse::ManualSelectionRequired {
        role: TargetRole::Ai,
    })
}

fn set_ai_target_ok(
    app: &mut App,
    session_id: String,
    role: TargetRole,
) -> grove::error::Result<BridgeResponse> {
    assert_eq!(role, TargetRole::Ai);
    app.bridge.ai_target_session_id = Some(session_id.clone());
    Ok(BridgeResponse::Pong)
}

use grove::app::{App, ContentSearchStatus, PathIndexState, PathIndexStatus, TabState};
use grove::preview::model::{SearchGeneration, SearchHit, SearchPayload};
use grove::search::SearchResponse;
use grove::search::content::{
    ContentSearchRequest, ContentSearchWorker, start_background_content_search,
};
use grove::state::ContextMode;
use grove::tree::indexer::{PathIndexEntry, PathIndexSnapshot};
use grove::tree::model::NodeKind;
use nucleo_matcher::Utf32String;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static TEMP_REPO_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn whole_repo_text_search_returns_path_line_and_excerpt_hits() {
    let repo = TempRepo::new();
    repo.write_file("docs/alpha.txt", "first line\nneedle one\nlast line");
    repo.write_file("notes/beta.txt", "needle two\nstill here");

    let worker = start_background_content_search();
    let submitted = worker.submit(ContentSearchRequest {
        generation: SearchGeneration(1),
        root_abs: repo.root.clone(),
        snapshot: repo.snapshot_for(&["docs/alpha.txt", "notes/beta.txt"]),
        query: "needle".to_string(),
        max_results: 10,
    });
    assert!(submitted, "search request should be queued");

    let response = wait_for_response(&worker);
    assert_eq!(response.generation, SearchGeneration(1));
    assert_eq!(response.payload.query, "needle");
    assert_eq!(response.payload.hits.len(), 2);
    assert_eq!(response.payload.hits[0].path, "docs/alpha.txt");
    assert_eq!(response.payload.hits[0].line, 2);
    assert_eq!(response.payload.hits[0].excerpt, "needle one");
    assert_eq!(response.payload.hits[1].path, "notes/beta.txt");
    assert_eq!(response.payload.hits[1].line, 1);
    assert_eq!(response.payload.hits[1].excerpt, "needle two");
}

#[test]
fn search_excerpt_preserves_leading_indentation() {
    let repo = TempRepo::new();
    repo.write_file("src/main.rs", "fn main() {\n    needle();\n}");

    let worker = start_background_content_search();
    let submitted = worker.submit(ContentSearchRequest {
        generation: SearchGeneration(1),
        root_abs: repo.root.clone(),
        snapshot: repo.snapshot_for(&["src/main.rs"]),
        query: "needle".to_string(),
        max_results: 10,
    });
    assert!(submitted, "search request should be queued");

    let response = wait_for_response(&worker);
    assert_eq!(response.payload.hits.len(), 1);
    assert_eq!(response.payload.hits[0].excerpt, "    needle();");
}

#[test]
fn app_submit_search_enters_results_mode_and_applies_background_hits() {
    let (repo, mut tab) = temp_tab();
    tab.complete_path_index();
    tab.path_index.snapshot = repo.snapshot_for(&["docs/alpha.txt", "notes/beta.txt"]);

    let mut app = App {
        tabs: vec![tab],
        active_tab: 0,
        ..App::default()
    };
    assert!(app.open_active_content_search());
    assert!(app.set_active_content_search_query("needle"));

    assert!(
        app.submit_active_content_search()
            .expect("submit content search")
    );
    assert_eq!(app.tabs[0].mode, ContextMode::SearchResults);
    assert!(matches!(
        app.tabs[0].content_search.status,
        ContentSearchStatus::Searching
    ));

    let deadline = Instant::now() + Duration::from_secs(2);
    while !matches!(
        app.tabs[0].content_search.status,
        ContentSearchStatus::Ready
    ) {
        assert!(
            Instant::now() < deadline,
            "content search did not complete before timeout"
        );
        let _ = app
            .poll_active_tab_content_search()
            .expect("poll content search");
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(app.tabs[0].content_search.payload.query, "needle");
    assert_eq!(app.tabs[0].content_search.payload.hits.len(), 2);
    assert_eq!(app.tabs[0].content_search.selected_hit_index, Some(0));
    assert!(app.select_active_content_search_hit(1));
    assert_eq!(app.tabs[0].content_search.selected_hit_index, Some(1));
}

#[test]
fn opening_content_search_starts_background_index_when_snapshot_is_idle() {
    let (repo, mut tab) = temp_tab();
    tab.path_index = PathIndexState::default();

    assert!(tab.open_content_search());
    assert!(
        tab.path_index.receiver.is_some(),
        "content search should start background indexing when no fresh snapshot exists"
    );
    assert!(
        matches!(tab.path_index.status, PathIndexStatus::Building { .. }),
        "starting content-search indexing should move the path index into building state"
    );

    fs::remove_dir_all(repo.root.clone()).expect("temp root should be removed");
}

#[test]
fn stale_search_generations_are_ignored_and_matching_results_apply() {
    let (_repo, mut tab) = temp_tab();
    assert!(tab.open_content_search());
    assert!(tab.set_content_search_query("needle"));

    let stale = SearchResponse {
        generation: SearchGeneration(0),
        payload: SearchPayload {
            query: "needle".to_string(),
            hits: vec![hit("docs/alpha.txt", 1, "stale result")],
        },
    };
    assert!(!tab.apply_content_search_response(stale));
    assert!(tab.content_search.payload.hits.is_empty());
    assert_eq!(tab.content_search.selected_hit_index, None);

    let fresh = SearchResponse {
        generation: tab.content_search.generation,
        payload: SearchPayload {
            query: "needle".to_string(),
            hits: vec![hit("docs/alpha.txt", 2, "needle one")],
        },
    };
    assert!(tab.apply_content_search_response(fresh));
    assert!(matches!(
        tab.content_search.status,
        ContentSearchStatus::Ready
    ));
    assert_eq!(tab.content_search.selected_hit_index, Some(0));
    assert_eq!(tab.content_search.payload.hits.len(), 1);
}

#[test]
fn selected_hit_clamps_when_results_shrink() {
    let (_repo, mut tab) = temp_tab();
    assert!(tab.open_content_search());
    assert!(tab.set_content_search_query("needle"));

    assert!(tab.apply_content_search_response(SearchResponse {
        generation: tab.content_search.generation,
        payload: SearchPayload {
            query: "needle".to_string(),
            hits: vec![
                hit("docs/alpha.txt", 2, "needle one"),
                hit("notes/beta.txt", 1, "needle two"),
            ],
        },
    }));
    assert!(tab.select_content_search_hit(1));
    assert_eq!(tab.content_search.selected_hit_index, Some(1));

    assert!(tab.apply_content_search_response(SearchResponse {
        generation: tab.content_search.generation,
        payload: SearchPayload {
            query: "needle".to_string(),
            hits: vec![hit("docs/alpha.txt", 2, "needle one")],
        },
    }));
    assert_eq!(tab.content_search.selected_hit_index, Some(0));
}

#[test]
fn activating_selected_search_hit_reveals_path_and_returns_preview_mode() {
    let repo = TempRepo::new();
    repo.write_file("nested/alpha.txt", "first line\nneedle one\nlast line");

    let mut tab = TabState::new(repo.root.clone());
    tab.complete_path_index();
    let snapshot = repo.snapshot_for(&["nested/alpha.txt"]);
    let entries = snapshot.entries.clone();
    tab.path_index.snapshot = snapshot;
    assert!(tab.ingest_path_index_batch(entries));
    assert!(tab.open_content_search());
    assert!(tab.set_content_search_query("needle"));
    assert!(tab.apply_content_search_response(SearchResponse {
        generation: tab.content_search.generation,
        payload: SearchPayload {
            query: "needle".to_string(),
            hits: vec![hit("nested/alpha.txt", 2, "needle one")],
        },
    }));

    let mut app = App {
        tabs: vec![tab],
        active_tab: 0,
        ..App::default()
    };

    assert!(app.activate_selected_content_search_hit());
    assert_eq!(app.tabs[0].mode, ContextMode::Preview);
    assert_eq!(
        app.tabs[0].tree.selected_rel_path().as_deref(),
        Some(Path::new("nested/alpha.txt"))
    );
    assert_eq!(app.tabs[0].preview.scroll_row, 1);
    assert_eq!(app.focus, grove::state::Focus::Preview);
    assert!(!app.tabs[0].content_search.active);
}

#[test]
fn activating_search_hit_preserves_preview_jump_after_render() {
    let repo = TempRepo::new();
    let mut lines = (0..80)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>();
    lines[39] = "needle one".to_string();
    repo.write_file("nested/alpha.txt", &lines.join("\n"));

    let mut tab = TabState::new(repo.root.clone());
    tab.complete_path_index();
    let snapshot = repo.snapshot_for(&["nested/alpha.txt"]);
    let entries = snapshot.entries.clone();
    tab.path_index.snapshot = snapshot;
    assert!(tab.ingest_path_index_batch(entries));
    assert!(tab.open_content_search());
    assert!(tab.set_content_search_query("needle"));
    assert!(tab.apply_content_search_response(SearchResponse {
        generation: tab.content_search.generation,
        payload: SearchPayload {
            query: "needle".to_string(),
            hits: vec![hit("nested/alpha.txt", 40, "needle one")],
        },
    }));

    let mut app = App {
        tabs: vec![tab],
        active_tab: 0,
        ..App::default()
    };
    assert!(app.activate_selected_content_search_hit());

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("test terminal should build");
    grove::bootstrap::render_shell_once(&mut terminal, &mut app).expect("render should succeed");

    assert_eq!(app.tabs[0].preview.scroll_row, 39);
}

fn wait_for_response(worker: &ContentSearchWorker) -> SearchResponse {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match worker.try_recv() {
            Ok(response) => return response,
            Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("expected content search response, got {err:?}"),
        }
    }
}

fn temp_tab() -> (TempRepo, TabState) {
    let repo = TempRepo::new();
    repo.write_file("docs/alpha.txt", "first line\nneedle one\nlast line");
    repo.write_file("notes/beta.txt", "needle two\nstill here");
    let tab = TabState::new(repo.root.clone());
    (repo, tab)
}

fn hit(path: &str, line: usize, excerpt: &str) -> SearchHit {
    SearchHit {
        path: path.to_string(),
        line,
        excerpt: excerpt.to_string(),
    }
}

struct TempRepo {
    root: PathBuf,
}

impl TempRepo {
    fn new() -> Self {
        let sequence = TEMP_REPO_COUNTER.fetch_add(1, Ordering::Relaxed);
        let unique = format!(
            "grove-content-search-{}-{}-{}",
            std::process::id(),
            sequence,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).expect("create temp repo root");
        Self { root }
    }

    fn write_file(&self, rel_path: &str, contents: &str) {
        let path = self.root.join(rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(path, contents).expect("write temp repo file");
    }

    fn snapshot_for(&self, rel_paths: &[&str]) -> PathIndexSnapshot {
        PathIndexSnapshot {
            entries: rel_paths
                .iter()
                .map(|rel_path| {
                    let rel_path = Path::new(rel_path).to_path_buf();
                    let name = rel_path
                        .file_name()
                        .expect("file name")
                        .to_string_lossy()
                        .to_string();
                    PathIndexEntry {
                        utf32_path: Utf32String::from(rel_path.to_string_lossy().to_string()),
                        rel_path,
                        name,
                        kind: NodeKind::File,
                    }
                })
                .collect(),
        }
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

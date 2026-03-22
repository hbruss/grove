use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, mpsc};

use crate::preview::model::{SearchGeneration, SearchHit, SearchPayload};
use crate::search::SearchResponse;
use crate::tree::indexer::PathIndexSnapshot;
use crate::tree::model::NodeKind;

#[derive(Debug, Clone)]
pub struct ContentSearchRequest {
    pub generation: SearchGeneration,
    pub root_abs: PathBuf,
    pub snapshot: PathIndexSnapshot,
    pub query: String,
    pub max_results: usize,
}

#[derive(Debug)]
pub struct ContentSearchWorker {
    shared: Arc<WorkerShared>,
    responses: mpsc::Receiver<SearchResponse>,
}

impl ContentSearchWorker {
    pub fn submit(&self, request: ContentSearchRequest) -> bool {
        let Ok(mut state) = self.shared.state.lock() else {
            return false;
        };
        if state.closed {
            return false;
        }
        state.pending = Some(request);
        self.shared.wake.notify_one();
        true
    }

    pub fn try_recv(&self) -> Result<SearchResponse, mpsc::TryRecvError> {
        self.responses.try_recv()
    }
}

impl Drop for ContentSearchWorker {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.closed = true;
            state.pending = None;
        }
        self.shared.wake.notify_one();
    }
}

pub fn start_background_content_search() -> ContentSearchWorker {
    let shared = Arc::new(WorkerShared {
        state: Mutex::new(WorkerState::default()),
        wake: Condvar::new(),
    });
    let (response_sender, responses) = mpsc::channel();
    let thread_shared = Arc::clone(&shared);

    std::thread::spawn(move || {
        loop {
            let request = {
                let Ok(mut state) = thread_shared.state.lock() else {
                    break;
                };
                while state.pending.is_none() && !state.closed {
                    state = match thread_shared.wake.wait(state) {
                        Ok(state) => state,
                        Err(_) => return,
                    };
                }

                if state.closed {
                    return;
                }

                state.pending.take()
            };

            let Some(request) = request else {
                continue;
            };
            let response = search_request(request);
            if response_sender.send(response).is_err() {
                return;
            }
        }
    });

    ContentSearchWorker { shared, responses }
}

#[derive(Debug, Default)]
struct WorkerState {
    pending: Option<ContentSearchRequest>,
    closed: bool,
}

#[derive(Debug)]
struct WorkerShared {
    state: Mutex<WorkerState>,
    wake: Condvar,
}

fn search_request(request: ContentSearchRequest) -> SearchResponse {
    let query = request.query;
    if query.is_empty() || request.max_results == 0 {
        return SearchResponse {
            generation: request.generation,
            payload: SearchPayload {
                query,
                hits: Vec::new(),
            },
        };
    }

    let mut hits = Vec::new();
    for entry in request.snapshot.entries {
        if !matches!(entry.kind, NodeKind::File | NodeKind::SymlinkFile) {
            continue;
        }
        if hits.len() >= request.max_results {
            break;
        }

        let path_abs = request.root_abs.join(&entry.rel_path);
        let Ok(bytes) = fs::read(&path_abs) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }

        let contents = String::from_utf8_lossy(&bytes);
        for (index, line) in contents.lines().enumerate() {
            if !line.contains(&query) {
                continue;
            }

            hits.push(SearchHit {
                path: entry.rel_path.to_string_lossy().to_string(),
                line: index + 1,
                excerpt: line.trim_end().to_string(),
            });
            if hits.len() >= request.max_results {
                break;
            }
        }
    }

    SearchResponse {
        generation: request.generation,
        payload: SearchPayload { query, hits },
    }
}

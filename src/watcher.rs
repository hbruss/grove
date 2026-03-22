use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use notify::event::ModifyKind;
use notify::{
    Config as NotifyConfig, Event as NotifyEvent, EventKind as NotifyEventKind,
    PollWatcher as NotifyPollWatcher, RecommendedWatcher, RecursiveMode, Watcher as NotifyWatcher,
};

use crate::config::WatcherConfig;
use crate::debug_log;
use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEventKind {
    Create,
    Change,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEvent {
    pub root: PathBuf,
    pub kind: WatchEventKind,
    pub path: PathBuf,
}

impl WatchEvent {
    pub fn new(root: &Path, kind: WatchEventKind, path: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            kind,
            path: path.to_path_buf(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefreshPlan {
    pub root: PathBuf,
    pub created_paths: Vec<PathBuf>,
    pub changed_paths: Vec<PathBuf>,
    pub removed_paths: Vec<PathBuf>,
    pub git_dirty: bool,
}

pub trait WatcherService {
    fn reconcile_open_roots(&mut self, roots: &[PathBuf]) -> Result<bool>;
    fn poll_refresh_plans(&mut self) -> Result<Vec<RefreshPlan>>;
}

impl<T> WatcherService for Box<T>
where
    T: WatcherService + ?Sized,
{
    fn reconcile_open_roots(&mut self, roots: &[PathBuf]) -> Result<bool> {
        self.as_mut().reconcile_open_roots(roots)
    }

    fn poll_refresh_plans(&mut self) -> Result<Vec<RefreshPlan>> {
        self.as_mut().poll_refresh_plans()
    }
}

pub struct WatcherRuntime<S> {
    config: WatcherConfig,
    service: S,
}

impl<S> WatcherRuntime<S> {
    pub fn new(config: WatcherConfig, service: S) -> Self {
        Self { config, service }
    }

    pub fn service_ref(&self) -> &S {
        &self.service
    }

    pub fn service_mut(&mut self) -> &mut S {
        &mut self.service
    }
}

impl<S: WatcherService> WatcherService for WatcherRuntime<S> {
    fn reconcile_open_roots(&mut self, roots: &[PathBuf]) -> Result<bool> {
        let normalized_roots = normalize_roots(roots);
        let normalized_root_vec = normalized_roots.iter().cloned().collect::<Vec<_>>();
        let changed = self.service.reconcile_open_roots(&normalized_root_vec)?;
        if changed {
            debug_log::log_component(
                "watcher",
                &format!(
                    "reconciled roots={} debounce_ms={} poll_fallback={}",
                    normalized_roots.len(),
                    self.config.debounce_ms,
                    self.config.poll_fallback
                ),
            );
        }
        Ok(changed)
    }

    fn poll_refresh_plans(&mut self) -> Result<Vec<RefreshPlan>> {
        self.service.poll_refresh_plans()
    }
}

impl WatcherRuntime<Box<dyn WatcherService>> {
    pub fn new_notify(config: WatcherConfig) -> Result<Self> {
        let service: Box<dyn WatcherService> = Box::new(NotifyWatcherService::new(&config)?);
        Ok(Self::new(config, service))
    }
}

pub fn normalize_watched_root(path: &Path) -> PathBuf {
    normalize_event_path(path)
}

pub fn coalesce_refresh_plans(events: Vec<WatchEvent>) -> Vec<RefreshPlan> {
    let mut plans_by_root: BTreeMap<PathBuf, BTreeMap<PathBuf, WatchEventKind>> = BTreeMap::new();

    for event in events {
        let root = normalize_watched_root(&event.root);
        let normalized_path = normalize_event_path(&event.path);
        let Some(rel_path) = relative_path_for_event(&root, &normalized_path) else {
            continue;
        };

        let paths = plans_by_root.entry(root).or_default();
        paths.insert(rel_path, event.kind);
    }

    plans_by_root
        .into_iter()
        .map(|(root, paths)| {
            let mut plan = RefreshPlan {
                root,
                git_dirty: !paths.is_empty(),
                ..RefreshPlan::default()
            };

            for (path, kind) in paths {
                match kind {
                    WatchEventKind::Create => plan.created_paths.push(path),
                    WatchEventKind::Change => plan.changed_paths.push(path),
                    WatchEventKind::Remove => plan.removed_paths.push(path),
                }
            }

            plan
        })
        .collect()
}

struct NotifyWatcherService {
    backend: NotifyBackend,
    receiver: Receiver<notify::Result<NotifyEvent>>,
    pending_events: Vec<WatchEvent>,
    watched_roots: BTreeSet<PathBuf>,
    debounce: Duration,
    last_event_at: Option<Instant>,
}

enum NotifyBackend {
    Recommended(RecommendedWatcher),
    Poll(NotifyPollWatcher),
}

impl NotifyWatcherService {
    fn new(config: &WatcherConfig) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let build_event_handler = |sender: mpsc::Sender<notify::Result<NotifyEvent>>| {
            move |result| {
                let _ = sender.send(result);
            }
        };

        let backend = select_backend(
            config,
            || {
                Ok(NotifyBackend::Recommended(RecommendedWatcher::new(
                    build_event_handler(tx.clone()),
                    NotifyConfig::default(),
                )?))
            },
            || {
                let notify_config =
                    NotifyConfig::default().with_poll_interval(config.debounce_duration());
                Ok(NotifyBackend::Poll(NotifyPollWatcher::new(
                    build_event_handler(tx.clone()),
                    notify_config,
                )?))
            },
        )?;

        Ok(Self {
            backend,
            receiver: rx,
            pending_events: Vec::new(),
            watched_roots: BTreeSet::new(),
            debounce: config.debounce_duration(),
            last_event_at: None,
        })
    }
    fn collect_notify_events(&mut self) -> Result<bool> {
        let mut changed = false;
        loop {
            match self.receiver.try_recv() {
                Ok(Ok(event)) => {
                    changed = true;
                    self.last_event_at = Some(Instant::now());
                    self.pending_events
                        .extend(event_to_watch_events(&event, &self.watched_roots));
                }
                Ok(Err(err)) => return Err(err.into()),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(std::io::Error::other("watcher event channel disconnected").into());
                }
            }
        }
        Ok(changed)
    }
}

impl WatcherService for NotifyWatcherService {
    fn reconcile_open_roots(&mut self, roots: &[PathBuf]) -> Result<bool> {
        let next_roots = normalize_roots(roots);
        let backend = &mut self.backend;
        let changed = reconcile_root_set(
            &mut self.watched_roots,
            &next_roots,
            |action| match action {
                RootReconcileAction::Unwatch(root) => unwatch_root_with_backend(backend, root),
                RootReconcileAction::Watch(root) => watch_root_with_backend(backend, root),
            },
        )?;
        if !changed {
            return Ok(false);
        }
        debug_log::log_component(
            "watcher",
            &format!("watched_roots={}", self.watched_roots.len()),
        );
        Ok(true)
    }

    fn poll_refresh_plans(&mut self) -> Result<Vec<RefreshPlan>> {
        let _ = self.collect_notify_events()?;
        if self.pending_events.is_empty() {
            return Ok(Vec::new());
        }

        if let Some(last_event_at) = self.last_event_at
            && last_event_at.elapsed() < self.debounce
        {
            return Ok(Vec::new());
        }

        if self.pending_events.len() > 1 {
            debug_log::log_component(
                "watcher",
                &format!(
                    "pending_events={} debounce_ms={}",
                    self.pending_events.len(),
                    self.debounce.as_millis()
                ),
            );
        }

        let plans = coalesce_refresh_plans(std::mem::take(&mut self.pending_events));
        self.last_event_at = None;
        if !plans.is_empty() {
            debug_log::log_component("watcher", &format!("refresh_plans={}", plans.len()));
        }
        Ok(plans)
    }
}

fn normalize_roots(roots: &[PathBuf]) -> BTreeSet<PathBuf> {
    roots
        .iter()
        .map(|root| normalize_watched_root(root))
        .collect()
}

fn reconcile_root_set(
    current_roots: &mut BTreeSet<PathBuf>,
    next_roots: &BTreeSet<PathBuf>,
    mut apply: impl FnMut(RootReconcileAction<'_>) -> Result<()>,
) -> Result<bool> {
    if current_roots == next_roots {
        return Ok(false);
    }

    let removed_roots: Vec<PathBuf> = current_roots.difference(next_roots).cloned().collect();
    let added_roots: Vec<PathBuf> = next_roots.difference(current_roots).cloned().collect();

    for root in removed_roots {
        apply(RootReconcileAction::Unwatch(&root))?;
        current_roots.remove(&root);
    }
    for root in added_roots {
        apply(RootReconcileAction::Watch(&root))?;
        current_roots.insert(root);
    }

    Ok(true)
}

enum RootReconcileAction<'a> {
    Unwatch(&'a Path),
    Watch(&'a Path),
}

fn watch_root_with_backend(backend: &mut NotifyBackend, root: &Path) -> Result<()> {
    match backend {
        NotifyBackend::Recommended(watcher) => watcher.watch(root, RecursiveMode::Recursive)?,
        NotifyBackend::Poll(watcher) => watcher.watch(root, RecursiveMode::Recursive)?,
    }
    Ok(())
}

fn unwatch_root_with_backend(backend: &mut NotifyBackend, root: &Path) -> Result<()> {
    match backend {
        NotifyBackend::Recommended(watcher) => watcher.unwatch(root)?,
        NotifyBackend::Poll(watcher) => watcher.unwatch(root)?,
    }
    Ok(())
}

fn select_backend<T, R, P>(config: &WatcherConfig, recommended: R, poll: P) -> Result<T>
where
    R: FnOnce() -> Result<T>,
    P: FnOnce() -> Result<T>,
{
    match recommended() {
        Ok(backend) => Ok(backend),
        Err(err) if config.poll_fallback => {
            debug_log::log_component(
                "watcher",
                &format!("recommended backend unavailable, falling back to poll: {err}"),
            );
            poll()
        }
        Err(err) => Err(err),
    }
}

fn relative_path_for_event(root: &Path, path: &Path) -> Option<PathBuf> {
    path.strip_prefix(root).ok().map(|rel_path| {
        if rel_path.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            rel_path.to_path_buf()
        }
    })
}

fn normalize_event_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }

    let Some(parent) = path.parent() else {
        return path.to_path_buf();
    };
    let normalized_parent = normalize_event_path(parent);
    match path.file_name() {
        Some(file_name) => normalized_parent.join(file_name),
        None => normalized_parent,
    }
}

fn event_to_watch_events(
    event: &NotifyEvent,
    watched_roots: &BTreeSet<PathBuf>,
) -> Vec<WatchEvent> {
    let kind = match &event.kind {
        NotifyEventKind::Create(_) => WatchEventKind::Create,
        NotifyEventKind::Remove(_) => WatchEventKind::Remove,
        NotifyEventKind::Modify(ModifyKind::Name(_)) => WatchEventKind::Change,
        NotifyEventKind::Modify(ModifyKind::Data(_))
        | NotifyEventKind::Modify(ModifyKind::Metadata(_))
        | NotifyEventKind::Modify(ModifyKind::Any) => WatchEventKind::Change,
        NotifyEventKind::Any | NotifyEventKind::Other => WatchEventKind::Change,
        _ => WatchEventKind::Change,
    };

    watch_event_paths(kind, &event.paths, watched_roots)
}

fn watch_event_paths(
    kind: WatchEventKind,
    paths: &[PathBuf],
    watched_roots: &BTreeSet<PathBuf>,
) -> Vec<WatchEvent> {
    let mut events = Vec::new();
    for path in paths.iter().filter(|path| !path.as_os_str().is_empty()) {
        let normalized_path = normalize_event_path(path);
        for root in watched_roots
            .iter()
            .filter(|root| normalized_path.starts_with(root.as_path()))
        {
            events.push(WatchEvent::new(root, kind, &normalized_path));
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::io;

    #[test]
    fn normalize_roots_deduplicates_canonical_paths() {
        let root = make_temp_dir("grove-watcher-runtime-root");
        let roots = vec![
            root.clone(),
            root.join("."),
            root.join("..").join(root.file_name().unwrap()),
        ];

        let normalized = normalize_roots(&roots);

        assert_eq!(normalized.len(), 1);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn select_backend_prefers_recommended_when_available() {
        let config = WatcherConfig {
            poll_fallback: true,
            ..WatcherConfig::default()
        };
        let mut poll_called = false;

        let backend = select_backend(
            &config,
            || Ok("recommended"),
            || {
                poll_called = true;
                Ok("poll")
            },
        )
        .expect("recommended backend should be selected");

        assert_eq!(backend, "recommended");
        assert!(!poll_called, "poll fallback must not run on success");
    }

    #[test]
    fn select_backend_falls_back_to_poll_only_when_recommended_fails_and_poll_fallback_enabled() {
        let config = WatcherConfig {
            poll_fallback: true,
            ..WatcherConfig::default()
        };
        let mut poll_called = false;

        let backend = select_backend(
            &config,
            || Err(io::Error::other("recommended failed").into()),
            || {
                poll_called = true;
                Ok("poll")
            },
        )
        .expect("poll fallback should recover recommended failure");

        assert_eq!(backend, "poll");
        assert!(
            poll_called,
            "poll fallback should run after recommended failure"
        );
    }

    #[test]
    fn select_backend_propagates_recommended_failure_when_poll_fallback_disabled() {
        let config = WatcherConfig {
            poll_fallback: false,
            ..WatcherConfig::default()
        };
        let mut poll_called = false;

        let err = select_backend(
            &config,
            || Err(io::Error::other("recommended failed").into()),
            || {
                poll_called = true;
                Ok("poll")
            },
        )
        .expect_err("recommended failure should surface without poll fallback");

        assert!(!poll_called, "poll fallback must stay disabled");
        assert!(err.to_string().contains("recommended failed"));
    }

    #[test]
    fn notify_service_uses_configured_debounce_to_hold_and_release_pending_plans() {
        let root = make_temp_dir("grove-watcher-debounce-runtime");
        let file = root.join("notes.md");
        fs::write(&file, "updated").expect("file should exist");

        let config = WatcherConfig {
            debounce_ms: 50,
            poll_fallback: false,
            ..WatcherConfig::default()
        };
        let mut service = NotifyWatcherService::new(&config).expect("notify watcher should build");
        service
            .pending_events
            .push(WatchEvent::new(&root, WatchEventKind::Change, &file));
        service.last_event_at = Some(Instant::now());

        assert!(
            service
                .poll_refresh_plans()
                .expect("debounced poll should succeed")
                .is_empty(),
            "pending events should remain held until the configured debounce window expires"
        );

        service.last_event_at = Some(Instant::now() - config.debounce_duration());
        let plans = service
            .poll_refresh_plans()
            .expect("expired debounce poll should succeed");
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].root, normalize_watched_root(&root));

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn event_paths_emit_all_matching_watched_roots_for_nested_open_roots() {
        let root = make_temp_dir("grove-watcher-nested-root");
        let docs_root = root.join("docs");
        fs::create_dir_all(&docs_root).expect("docs root should exist");
        let file = docs_root.join("notes.md");
        fs::write(&file, "updated").expect("file should exist");

        let mut watched_roots = BTreeSet::new();
        watched_roots.insert(normalize_watched_root(&root));
        watched_roots.insert(normalize_watched_root(&docs_root));

        let events = watch_event_paths(
            WatchEventKind::Change,
            std::slice::from_ref(&file),
            &watched_roots,
        );

        assert_eq!(
            events.len(),
            2,
            "nested watched roots should both receive events"
        );
        assert!(
            events
                .iter()
                .any(|event| event.root == normalize_watched_root(&root))
        );
        assert!(
            events
                .iter()
                .any(|event| event.root == normalize_watched_root(&docs_root))
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn reconcile_root_set_keeps_state_aligned_after_partial_failure() {
        let root_a = PathBuf::from("/tmp/root-a");
        let root_b = PathBuf::from("/tmp/root-b");
        let root_c = PathBuf::from("/tmp/root-c");
        let mut current = BTreeSet::from([root_a.clone(), root_b.clone()]);
        let next = BTreeSet::from([root_b.clone(), root_c.clone()]);
        let mut watched_backend = current.clone();

        let err = reconcile_root_set(&mut current, &next, |action| match action {
            RootReconcileAction::Unwatch(root) => {
                watched_backend.remove(root);
                Ok(())
            }
            RootReconcileAction::Watch(root) => {
                if root == root_c.as_path() {
                    return Err(io::Error::other("watch failed").into());
                }
                watched_backend.insert(root.to_path_buf());
                Ok(())
            }
        })
        .expect_err("partial watch failure should surface");

        assert!(err.to_string().contains("watch failed"));
        assert_eq!(
            current, watched_backend,
            "in-memory root state should match the backend state after partial failure"
        );
        assert_eq!(
            current,
            BTreeSet::from([root_b]),
            "successful unwatch work should remain reflected even when a later watch fails"
        );
    }

    fn make_temp_dir(label: &str) -> PathBuf {
        let unique = Instant::now().elapsed().as_nanos();
        let root = std::env::temp_dir().join(format!("{label}-{unique}"));
        fs::create_dir_all(&root).expect("temp root should be created");
        root
    }
}

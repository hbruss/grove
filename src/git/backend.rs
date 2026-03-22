use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use git2::{DiffFormat, DiffOptions, ObjectType, Repository, Status, StatusOptions, Tree};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::state::GitRepoSummary;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoHandle {
    pub root: PathBuf,
    pub branch_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GitStatus {
    #[default]
    Unmodified,
    Modified,
    Added,
    Deleted,
    Renamed,
    Typechange,
    Conflicted,
    Ignored,
    Unknown,
}

pub fn git_status_for_path(status: &GitPathStatus) -> GitStatus {
    if status.conflicted {
        return GitStatus::Conflicted;
    }
    if status.ignored {
        return GitStatus::Ignored;
    }
    if status.untracked {
        return GitStatus::Unknown;
    }
    if status.index != GitChange::Unmodified && status.worktree != GitChange::Unmodified {
        return GitStatus::Modified;
    }

    let change = if status.index != GitChange::Unmodified {
        status.index
    } else {
        status.worktree
    };
    git_status_for_change(change)
}

pub fn combine_git_status(current: GitStatus, candidate: GitStatus) -> GitStatus {
    if git_status_priority(candidate) > git_status_priority(current) {
        candidate
    } else {
        current
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitChange {
    #[default]
    Unmodified,
    Added,
    Modified,
    Deleted,
    Renamed,
    Typechange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GitPathStatus {
    pub index: GitChange,
    pub worktree: GitChange,
    pub untracked: bool,
    pub conflicted: bool,
    pub ignored: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffMode {
    #[default]
    Unstaged,
    Staged,
    Head,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnifiedDiff {
    pub text: String,
    pub first_changed_line: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlameLine {
    pub line_number: usize,
    pub commit_id: String,
    pub author: String,
    pub author_time: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlameView {
    pub lines: Vec<BlameLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommitSummary {
    pub id: String,
    pub summary: String,
    pub author: String,
    pub when: String,
}

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git repository not found")]
    RepoNotFound,
    #[error("git operation failed: {message}")]
    Operation { message: String },
}

#[derive(Debug, Clone, Default)]
pub struct LibgitBackend;

pub trait GitBackend: Send + Sync {
    fn discover_repo(&self, root: &Path) -> Result<Option<RepoHandle>, GitError>;
    fn status_map(&self, repo: &RepoHandle) -> Result<HashMap<PathBuf, GitPathStatus>, GitError>;
    fn diff_for_path(
        &self,
        repo: &RepoHandle,
        rel_path: &Path,
        mode: DiffMode,
    ) -> Result<UnifiedDiff, GitError>;
    fn blame_for_path(&self, repo: &RepoHandle, rel_path: &Path) -> Result<BlameView, GitError>;
    fn history_for_path(
        &self,
        repo: &RepoHandle,
        rel_path: &Path,
        limit: usize,
    ) -> Result<Vec<CommitSummary>, GitError>;
    fn stage_path(&self, repo: &RepoHandle, rel_path: &Path) -> Result<(), GitError>;
    fn unstage_path(&self, repo: &RepoHandle, rel_path: &Path) -> Result<(), GitError>;
}

pub fn summarize_repo_statuses(
    repo: &RepoHandle,
    status_map: &HashMap<PathBuf, GitPathStatus>,
) -> GitRepoSummary {
    let mut summary = GitRepoSummary {
        repo_root: repo.root.clone(),
        branch_name: repo.branch_name.clone(),
        ..GitRepoSummary::default()
    };
    for status in status_map.values() {
        if status.index != GitChange::Unmodified {
            summary.staged_paths = summary.staged_paths.saturating_add(1);
        }
        if status.worktree != GitChange::Unmodified {
            summary.unstaged_paths = summary.unstaged_paths.saturating_add(1);
        }
        if status.untracked {
            summary.untracked_paths = summary.untracked_paths.saturating_add(1);
        }
        if status.conflicted {
            summary.conflicted_paths = summary.conflicted_paths.saturating_add(1);
        }
    }
    summary
}

impl GitBackend for LibgitBackend {
    fn discover_repo(&self, root: &Path) -> Result<Option<RepoHandle>, GitError> {
        match Repository::discover(root) {
            Ok(repo) => Ok(Some(RepoHandle {
                root: repo_root(&repo)?,
                branch_name: current_branch_name(&repo),
            })),
            Err(err) if err.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(err) => Err(map_git_error(err)),
        }
    }

    fn status_map(&self, repo: &RepoHandle) -> Result<HashMap<PathBuf, GitPathStatus>, GitError> {
        let repo = Repository::open(&repo.root).map_err(map_git_error)?;
        let mut options = StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(true)
            .renames_head_to_index(true)
            .renames_index_to_workdir(true);

        let statuses = repo.statuses(Some(&mut options)).map_err(map_git_error)?;
        let mut map = HashMap::new();
        for entry in statuses.iter() {
            let Some(path) = entry.path() else {
                continue;
            };
            map.insert(PathBuf::from(path), git_path_status(entry.status()));
        }
        Ok(map)
    }

    fn diff_for_path(
        &self,
        repo: &RepoHandle,
        rel_path: &Path,
        mode: DiffMode,
    ) -> Result<UnifiedDiff, GitError> {
        let repo = Repository::open(&repo.root).map_err(map_git_error)?;
        let mut options = DiffOptions::new();
        options.pathspec(rel_path);

        let diff = match mode {
            DiffMode::Unstaged => {
                let index = repo.index().map_err(map_git_error)?;
                repo.diff_index_to_workdir(Some(&index), Some(&mut options))
                    .map_err(map_git_error)?
            }
            DiffMode::Staged => {
                let index = repo.index().map_err(map_git_error)?;
                let head_tree = head_tree(&repo)?;
                repo.diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut options))
                    .map_err(map_git_error)?
            }
            DiffMode::Head => {
                let head_tree = head_tree(&repo)?;
                repo.diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut options))
                    .map_err(map_git_error)?
            }
        };

        let mut text = String::new();
        let mut first_changed_line = None;
        diff.print(DiffFormat::Patch, |_delta, hunk, line| {
            if first_changed_line.is_none() {
                first_changed_line = hunk
                    .map(|hunk| hunk.new_start() as usize)
                    .or_else(|| line.new_lineno().map(|line_no| line_no as usize));
            }
            if matches!(line.origin(), '+' | '-' | ' ') {
                text.push(line.origin());
            }
            text.push_str(&String::from_utf8_lossy(line.content()));
            true
        })
        .map_err(map_git_error)?;

        Ok(UnifiedDiff {
            text,
            first_changed_line,
        })
    }

    fn blame_for_path(&self, _repo: &RepoHandle, _rel_path: &Path) -> Result<BlameView, GitError> {
        Err(not_implemented("blame_for_path"))
    }

    fn history_for_path(
        &self,
        _repo: &RepoHandle,
        _rel_path: &Path,
        _limit: usize,
    ) -> Result<Vec<CommitSummary>, GitError> {
        Err(not_implemented("history_for_path"))
    }

    fn stage_path(&self, repo: &RepoHandle, rel_path: &Path) -> Result<(), GitError> {
        let repo = Repository::open(&repo.root).map_err(map_git_error)?;
        let workdir = workdir(&repo)?;
        if rel_path.as_os_str().is_empty() {
            return Err(operation_not_supported("root path cannot be staged"));
        }

        let abs_path = workdir.join(rel_path);
        let mut index = repo.index().map_err(map_git_error)?;
        match fs::symlink_metadata(&abs_path) {
            Ok(metadata) => {
                if metadata.is_dir() {
                    return Err(operation_not_supported(
                        "directories cannot be staged with this action",
                    ));
                }
                index.add_path(rel_path).map_err(map_git_error)?;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                index.remove_path(rel_path).map_err(map_git_error)?;
            }
            Err(err) => {
                return Err(GitError::Operation {
                    message: err.to_string(),
                });
            }
        }
        index.write().map_err(map_git_error)?;
        Ok(())
    }

    fn unstage_path(&self, repo: &RepoHandle, rel_path: &Path) -> Result<(), GitError> {
        let repo = Repository::open(&repo.root).map_err(map_git_error)?;
        let workdir = workdir(&repo)?;
        if rel_path.as_os_str().is_empty() {
            return Err(operation_not_supported("root path cannot be unstaged"));
        }

        let abs_path = workdir.join(rel_path);
        if let Ok(metadata) = fs::metadata(&abs_path)
            && metadata.is_dir()
        {
            return Err(operation_not_supported(
                "directories cannot be unstaged with this action",
            ));
        }

        let target = match repo.head() {
            Ok(head) => Some(head.peel(ObjectType::Commit).map_err(map_git_error)?),
            Err(err) if err.code() == git2::ErrorCode::UnbornBranch => None,
            Err(err) => return Err(map_git_error(err)),
        };
        repo.reset_default(target.as_ref(), [rel_path])
            .map_err(map_git_error)?;
        Ok(())
    }
}

fn repo_root(repo: &Repository) -> Result<PathBuf, GitError> {
    if let Some(workdir) = repo.workdir() {
        return Ok(workdir.to_path_buf());
    }

    repo.path()
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| GitError::Operation {
            message: "unable to determine repository root".to_string(),
        })
}

fn current_branch_name(repo: &Repository) -> String {
    match repo.head() {
        Ok(head) => {
            if let Some(name) = head.shorthand() {
                return name.to_string();
            }
            if let Some(target) = head.target() {
                return target.to_string().chars().take(7).collect();
            }
            "HEAD".to_string()
        }
        Err(err) if err.code() == git2::ErrorCode::UnbornBranch => repo
            .find_reference("HEAD")
            .ok()
            .and_then(|head| {
                head.symbolic_target()
                    .and_then(branch_name_from_symbolic_target)
            })
            .unwrap_or_else(|| "HEAD".to_string()),
        Err(_) => "HEAD".to_string(),
    }
}

fn branch_name_from_symbolic_target(symbolic_target: &str) -> Option<String> {
    symbolic_target
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
}

fn head_tree(repo: &Repository) -> Result<Option<Tree<'_>>, GitError> {
    let head = match repo.head() {
        Ok(head) => head,
        Err(err) if err.code() == git2::ErrorCode::UnbornBranch => return Ok(None),
        Err(err) => return Err(map_git_error(err)),
    };
    let commit = head.peel_to_commit().map_err(map_git_error)?;
    let tree = commit.tree().map_err(map_git_error)?;
    Ok(Some(tree))
}

fn workdir(repo: &Repository) -> Result<&Path, GitError> {
    repo.workdir().ok_or_else(|| GitError::Operation {
        message: "bare repositories are not supported".to_string(),
    })
}

fn git_path_status(status: Status) -> GitPathStatus {
    GitPathStatus {
        index: git_change_from_index(status),
        worktree: git_change_from_worktree(status),
        untracked: status.is_wt_new(),
        conflicted: status.is_conflicted(),
        ignored: status.is_ignored(),
    }
}

fn git_change_from_index(status: Status) -> GitChange {
    if status.is_index_new() {
        GitChange::Added
    } else if status.is_index_modified() {
        GitChange::Modified
    } else if status.is_index_deleted() {
        GitChange::Deleted
    } else if status.is_index_renamed() {
        GitChange::Renamed
    } else if status.is_index_typechange() {
        GitChange::Typechange
    } else {
        GitChange::Unmodified
    }
}

fn git_change_from_worktree(status: Status) -> GitChange {
    if status.is_wt_modified() {
        GitChange::Modified
    } else if status.is_wt_deleted() {
        GitChange::Deleted
    } else if status.is_wt_renamed() {
        GitChange::Renamed
    } else if status.is_wt_typechange() {
        GitChange::Typechange
    } else {
        GitChange::Unmodified
    }
}

fn git_status_for_change(change: GitChange) -> GitStatus {
    match change {
        GitChange::Unmodified => GitStatus::Unmodified,
        GitChange::Added => GitStatus::Added,
        GitChange::Modified => GitStatus::Modified,
        GitChange::Deleted => GitStatus::Deleted,
        GitChange::Renamed => GitStatus::Renamed,
        GitChange::Typechange => GitStatus::Typechange,
    }
}

fn git_status_priority(status: GitStatus) -> u8 {
    match status {
        GitStatus::Conflicted => 4,
        GitStatus::Modified
        | GitStatus::Added
        | GitStatus::Deleted
        | GitStatus::Renamed
        | GitStatus::Typechange => 3,
        GitStatus::Unknown => 2,
        GitStatus::Ignored => 1,
        GitStatus::Unmodified => 0,
    }
}

fn map_git_error(err: git2::Error) -> GitError {
    GitError::Operation {
        message: err.message().to_string(),
    }
}

fn operation_not_supported(message: &str) -> GitError {
    GitError::Operation {
        message: message.to_string(),
    }
}

fn not_implemented(operation: &str) -> GitError {
    GitError::Operation {
        message: format!("{operation} is not implemented"),
    }
}

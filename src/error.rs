use std::fmt::{Display, Formatter};

pub type Result<T> = std::result::Result<T, GroveError>;

#[derive(Debug)]
pub enum GroveError {
    Io(std::io::Error),
    Git(crate::git::backend::GitError),
}

impl Display for GroveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Git(err) => write!(f, "git error: {err}"),
        }
    }
}

impl std::error::Error for GroveError {}

impl From<std::io::Error> for GroveError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<notify::Error> for GroveError {
    fn from(value: notify::Error) -> Self {
        Self::Io(std::io::Error::other(value.to_string()))
    }
}

impl From<crate::git::backend::GitError> for GroveError {
    fn from(value: crate::git::backend::GitError) -> Self {
        Self::Git(value)
    }
}

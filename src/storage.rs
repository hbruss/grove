use std::io::{Error, ErrorKind};
use std::path::PathBuf;

use crate::error::Result;

const APP_DIR: &str = "grove";

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn state_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("state.json"))
}

pub fn cache_dir() -> Result<PathBuf> {
    resolve_root("XDG_CACHE_HOME", ".cache")
}

fn config_dir() -> Result<PathBuf> {
    resolve_root("XDG_CONFIG_HOME", ".config")
}

fn resolve_root(env_key: &str, home_suffix: &str) -> Result<PathBuf> {
    if let Some(value) = std::env::var_os(env_key) {
        return Ok(PathBuf::from(value).join(APP_DIR));
    }

    let Some(home) = std::env::var_os("HOME") else {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!("unable to resolve {env_key} or HOME for Grove runtime paths"),
        )
        .into());
    };

    Ok(PathBuf::from(home).join(home_suffix).join(APP_DIR))
}

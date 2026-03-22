use std::cmp::Ordering;
use std::ffi::OsStr;
use std::path::Path;

use ignore::WalkBuilder;

use super::model::VisibilitySettings;

pub(crate) fn build_recursive_walker(
    root_abs: &Path,
    visibility: VisibilitySettings,
) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root_abs);
    configure_builder(&mut builder, root_abs, visibility);
    builder
}

pub(crate) fn build_directory_walker(
    dir_abs: &Path,
    root_abs: &Path,
    visibility: VisibilitySettings,
) -> WalkBuilder {
    let mut builder = WalkBuilder::new(dir_abs);
    configure_builder(&mut builder, root_abs, visibility);
    builder.max_depth(Some(1));
    builder
}

fn configure_builder(builder: &mut WalkBuilder, root_abs: &Path, visibility: VisibilitySettings) {
    builder.standard_filters(false);
    builder.hidden(!visibility.show_hidden);
    builder.parents(visibility.respect_gitignore);
    builder.git_ignore(visibility.respect_gitignore);
    builder.git_exclude(visibility.respect_gitignore);
    builder.git_global(visibility.respect_gitignore);
    builder.require_git(true);
    builder.follow_links(false);
    builder.current_dir(root_abs.to_path_buf());
    builder.sort_by_file_name(compare_file_names);
}

fn compare_file_names(left: &OsStr, right: &OsStr) -> Ordering {
    left.to_string_lossy()
        .to_lowercase()
        .cmp(&right.to_string_lossy().to_lowercase())
}

pub mod action_bar;
pub mod command_palette;
pub mod content_search;
pub mod dialog;
pub mod layout;
pub mod metadata;
pub mod path_filter;
pub mod preview;
pub mod root;
pub mod roots;
pub mod status_bar;
pub mod target_picker;
pub mod theme;
pub mod tree;

use ratatui::Frame;

use crate::app::App;

pub fn render(frame: &mut Frame, app: &App) {
    root::render(frame, app);
}

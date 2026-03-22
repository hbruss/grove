use ratatui::Frame;

use crate::app::App;

use super::{
    action_bar, command_palette, content_search, dialog, layout, path_filter, preview, roots,
    status_bar, tree,
};

pub fn render(frame: &mut Frame, app: &App) {
    let split_ratio = app
        .tabs
        .get(app.active_tab)
        .map(|tab| tab.split_ratio)
        .unwrap_or(0.40);
    let areas = layout::compute(frame.area(), split_ratio, app.root_navigator_panel_height());

    path_filter::render(frame, areas.path_filter, app);
    roots::render(frame, areas.roots, app);
    tree::render(frame, areas.tree, app);
    preview::render(frame, areas.preview, app);
    action_bar::render(frame, areas.action_bar, app);
    status_bar::render(frame, areas.status_bar, app);

    if app.dialog_state().is_some() {
        dialog::render(frame, app);
        return;
    }

    if app.overlays.command_palette.active {
        command_palette::render(frame, app);
        return;
    }

    if app
        .tabs
        .get(app.active_tab)
        .is_some_and(|tab| tab.content_search.active)
    {
        content_search::render(frame, app);
    }
}

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::state::{DialogState, Focus};

use super::theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let text = if app.focus == Focus::Dialog {
        match app.dialog_state() {
            Some(DialogState::TargetPicker(_)) => {
                "Picker: Up/Down move  Enter choose  Esc cancel".to_string()
            }
            Some(DialogState::DirectoryPicker(_)) => {
                "Add Root: Up/Down select  Left parent  Right open folder  Enter pin + open  Esc cancel".to_string()
            }
            _ => "Dialog: Enter confirm  Esc cancel".to_string(),
        }
    } else if app.active_multi_select_mode() {
        if app.focus == Focus::Tree {
            "Multi-select: Space toggle  x clear  m done  Ctrl+Y send".to_string()
        } else {
            "Multi-select on: Tab to tree  Ctrl+Y send".to_string()
        }
    } else if app.focus == Focus::Preview {
        "Preview: Up/Down scroll  Shift+Up/Down select  c copy  Esc clear selection".to_string()
    } else if app.focus == Focus::CommandPalette {
        "Commands: type to filter  Up/Down move  Enter choose  Esc cancel".to_string()
    } else {
        crate::actions::catalog::action_bar_entries(app)
            .into_iter()
            .filter(|entry| entry.enabled)
            .filter_map(|entry| {
                entry
                    .subtitle
                    .map(|subtitle| format!("{subtitle}: {}", entry.label))
            })
            .collect::<Vec<_>>()
            .join("  ")
    };
    let widget = Paragraph::new(text)
        .style(theme::panel_style())
        .block(theme::panel_block("Action Bar"));
    frame.render_widget(widget, area);
}

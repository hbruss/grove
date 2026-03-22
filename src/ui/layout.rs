use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Debug, Clone, Copy)]
pub struct ShellAreas {
    pub path_filter: Rect,
    pub roots: Rect,
    pub tree: Rect,
    pub preview: Rect,
    pub action_bar: Rect,
    pub status_bar: Rect,
}

pub fn compute(area: Rect, split_ratio: f32, roots_height: u16) -> ShellAreas {
    let split_ratio = split_ratio.clamp(0.20, 0.80);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(area);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((split_ratio * 100.0) as u16),
            Constraint::Min(20),
        ])
        .split(outer[0]);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(roots_height),
            Constraint::Min(1),
        ])
        .split(body[0]);

    ShellAreas {
        path_filter: left[0],
        roots: left[1],
        tree: left[2],
        preview: body[1],
        action_bar: outer[1],
        status_bar: outer[2],
    }
}

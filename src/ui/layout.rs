use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Debug, Clone, Copy)]
pub struct ShellAreas {
    pub path_filter: Rect,
    pub roots: Rect,
    pub tree: Rect,
    pub metadata: Rect,
    pub preview: Rect,
    pub action_bar: Rect,
    pub status_bar: Rect,
}

const METADATA_PANEL_HEIGHT: u16 = 6;

pub fn compute(area: Rect, split_ratio: f32, roots_height: u16, preview_visible: bool) -> ShellAreas {
    let split_ratio = split_ratio.clamp(0.20, 0.80);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(area);

    if preview_visible {
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
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(METADATA_PANEL_HEIGHT), Constraint::Min(1)])
            .split(body[1]);

        return ShellAreas {
            path_filter: left[0],
            roots: left[1],
            tree: left[2],
            metadata: right[0],
            preview: right[1],
            action_bar: outer[1],
            status_bar: outer[2],
        };
    }

    let header_height = (3 + roots_height).max(METADATA_PANEL_HEIGHT);
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_height), Constraint::Min(1)])
        .split(outer[0]);
    let header = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((split_ratio * 100.0) as u16),
            Constraint::Min(20),
        ])
        .split(body[0]);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(roots_height),
            Constraint::Min(0),
        ])
        .split(header[0]);

    ShellAreas {
        path_filter: left[0],
        roots: left[1],
        tree: body[1],
        metadata: header[1],
        preview: Rect::new(0, 0, 0, 0),
        action_bar: outer[1],
        status_bar: outer[2],
    }
}

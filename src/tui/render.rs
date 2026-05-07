use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};

use crate::interactive::models::WorkspaceProjection;
use crate::tui::state::{TuiState, TuiTab};

pub fn render_workspace_frame(
    frame: &mut Frame<'_>,
    state: &TuiState,
    projection: Option<&WorkspaceProjection>,
) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(5),
        ])
        .split(frame.area());

    let tabs = Tabs::new(vec!["Overview", "Timeline", "IO", "Artifacts", "Changes"])
        .select(tab_index(state.active_tab))
        .block(Block::default().title("Aria TUI").borders(Borders::ALL));
    frame.render_widget(tabs, root[0]);

    let body_text = match projection {
        Some(projection) => format!(
            "workspace: {}\ntask: {}\nstatus: {}",
            projection.workspace_root,
            projection.active_task_id.as_deref().unwrap_or("none"),
            projection
                .overview
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
        ),
        None => "No workspace projection loaded".to_string(),
    };
    frame.render_widget(
        Paragraph::new(body_text).block(Block::default().title("Workbench").borders(Borders::ALL)),
        root[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(state.action_input.as_str()))
            .block(Block::default().title("Action").borders(Borders::ALL)),
        root[2],
    );
}

fn tab_index(tab: TuiTab) -> usize {
    match tab {
        TuiTab::Overview => 0,
        TuiTab::Timeline => 1,
        TuiTab::Io => 2,
        TuiTab::Artifacts => 3,
        TuiTab::Changes => 4,
    }
}

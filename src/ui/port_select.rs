use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::App;

pub fn draw(app: &mut App, frame: &mut Frame) {
    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).split(frame.area());

    // Port list
    let items: Vec<ListItem> = app
        .ports
        .iter()
        .map(|p| {
            let line = if p.description.is_empty() {
                Line::from(p.name.clone())
            } else {
                Line::from(vec![
                    Span::styled(&p.name, Style::default().fg(Color::Cyan)),
                    Span::raw("  "),
                    Span::styled(&p.description, Style::default().fg(Color::DarkGray)),
                ])
            };
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Serial Ports ")
                .borders(Borders::ALL),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, chunks[0], &mut app.port_list_state);

    // Help bar
    let help = Paragraph::new(Line::from(vec![
        Span::styled(" ↑↓ ", Style::default().fg(Color::Cyan)),
        Span::raw("Navigate  "),
        Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
        Span::raw("Select  "),
        Span::styled(" r ", Style::default().fg(Color::Cyan)),
        Span::raw("Refresh  "),
        Span::styled(" q ", Style::default().fg(Color::Cyan)),
        Span::raw("Quit"),
    ]))
    .block(Block::default().borders(Borders::ALL));

    frame.render_widget(help, chunks[1]);
}

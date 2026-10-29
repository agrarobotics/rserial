use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{App, ConfigField};

pub fn draw(app: &App, frame: &mut Frame) {
    let area = centered_rect(50, 14, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Configure ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1), // spacing
        Constraint::Length(1), // baud
        Constraint::Length(1), // spacing
        Constraint::Length(1), // data bits
        Constraint::Length(1), // spacing
        Constraint::Length(1), // parity
        Constraint::Length(1), // spacing
        Constraint::Length(1), // stop bits
        Constraint::Length(1), // spacing
        Constraint::Length(1), // spacing
        Constraint::Length(1), // help
        Constraint::Min(0),
    ])
    .split(inner);

    let selected_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(Color::DarkGray);
    let value_style = Style::default().fg(Color::White);

    let fields = [
        (
            ConfigField::BaudRate,
            "Baud Rate",
            app.config.baud_display(),
        ),
        (
            ConfigField::DataBits,
            "Data Bits",
            app.config.data_bits_display().to_string(),
        ),
        (
            ConfigField::Parity,
            "Parity   ",
            app.config.parity_display().to_string(),
        ),
        (
            ConfigField::StopBits,
            "Stop Bits",
            app.config.stop_bits_display().to_string(),
        ),
    ];

    for (i, (field, label, value)) in fields.iter().enumerate() {
        let row_idx = 1 + i * 2;
        let is_selected = app.config_field == *field;
        let line = Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{label}: "),
                if is_selected {
                    selected_style
                } else {
                    label_style
                },
            ),
            Span::styled(
                value.clone(),
                if is_selected {
                    selected_style
                } else {
                    value_style
                },
            ),
            if is_selected {
                Span::styled(" ◂▸", Style::default().fg(Color::Yellow))
            } else {
                Span::raw("")
            },
        ]);
        frame.render_widget(Paragraph::new(line), rows[row_idx]);
    }

    // Help text
    let help = Line::from(vec![
        Span::styled(" Tab ", Style::default().fg(Color::Cyan)),
        Span::raw("Field  "),
        Span::styled(" ↑↓ ", Style::default().fg(Color::Cyan)),
        Span::raw("Value  "),
        Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
        Span::raw("Connect  "),
        Span::styled(" Esc ", Style::default().fg(Color::Cyan)),
        Span::raw("Back"),
    ]);
    frame.render_widget(Paragraph::new(help), rows[10]);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .split(area);
    Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .split(vertical[0])[0]
}

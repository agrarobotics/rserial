use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use crate::app::{App, DisplayMode};

pub fn draw(app: &mut App, frame: &mut Frame) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(5),    // data area
        Constraint::Length(3), // TX input
        Constraint::Length(1), // status bar
    ])
    .split(frame.area());

    draw_header(app, frame, chunks[0]);
    draw_data_area(app, frame, chunks[1]);
    draw_input_bar(app, frame, chunks[2]);
    draw_status_bar(app, frame, chunks[3]);
}

fn draw_header(app: &App, frame: &mut Frame, area: ratatui::layout::Rect) {
    let mode_str = match app.display_mode {
        DisplayMode::Text => "TEXT",
        DisplayMode::Hex => "HEX",
    };

    let scroll_str = if app.scroll_offset > 0 {
        format!("SCROLL:{}", app.scroll_offset)
    } else {
        "LIVE".to_string()
    };

    let mut spans = vec![
        Span::styled(" PORT: ", Style::default().fg(Color::DarkGray)),
        Span::styled(&app.connected_port, Style::default().fg(Color::Cyan)),
        Span::styled(
            format!(" @ {} ", app.config.baud_rate()),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("│ "),
        Span::styled(
            format!(" {mode_str} "),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" │ "),
        Span::styled(scroll_str, Style::default().fg(Color::Yellow)),
    ];

    if app.selecting {
        let (lo, hi) = app.selection_range();
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            format!(" SELECT ({} lines) ", hi - lo + 1),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let header = Line::from(spans);

    frame.render_widget(
        Paragraph::new(header).style(Style::default().bg(Color::DarkGray).fg(Color::White)),
        area,
    );
}

fn draw_data_area(app: &mut App, frame: &mut Frame, area: ratatui::layout::Rect) {
    match app.display_mode {
        DisplayMode::Text => draw_text_view(app, frame, area),
        DisplayMode::Hex => draw_hex_view(app, frame, area),
    }
}

fn draw_text_view(app: &mut App, frame: &mut Frame, area: ratatui::layout::Rect) {
    let block = Block::default().title(" RX Data ").borders(Borders::ALL);
    let inner_height = block.inner(area).height as usize;
    app.last_inner_height = inner_height;

    // Collect all lines including partial
    let total = app.total_display_lines();

    // Calculate visible range based on scroll offset
    let end = total.saturating_sub(app.scroll_offset);
    let start = end.saturating_sub(inner_height);

    let (sel_lo, sel_hi) = app.selection_range();

    let mut lines: Vec<Line> = Vec::new();
    for i in start..end {
        let in_selection = app.selecting && i >= sel_lo && i <= sel_hi;
        if i < app.rx_lines.len() {
            let style = if in_selection {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(app.rx_lines[i].as_str(), style)));
        } else if i == app.rx_lines.len() && !app.line_buf.is_empty() {
            let partial = String::from_utf8_lossy(&app.line_buf).into_owned();
            let style = if in_selection {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            lines.push(Line::from(Span::styled(partial, style)));
        }
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);

    // Scrollbar
    if total > inner_height {
        let mut scrollbar_state =
            ScrollbarState::new(total.saturating_sub(inner_height)).position(start);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area,
            &mut scrollbar_state,
        );
    }
}

fn draw_hex_view(app: &mut App, frame: &mut Frame, area: ratatui::layout::Rect) {
    let block = Block::default()
        .title(" RX Data (Hex) ")
        .borders(Borders::ALL);
    let inner_height = block.inner(area).height as usize;
    app.last_inner_height = inner_height;

    // Flatten all raw bytes
    let all_bytes: Vec<u8> = app
        .rx_raw
        .iter()
        .flat_map(|chunk| chunk.iter().copied())
        .collect();
    let total_rows = all_bytes.len().div_ceil(16);

    let end_row = total_rows.saturating_sub(app.scroll_offset);
    let start_row = end_row.saturating_sub(inner_height);

    let (sel_lo, sel_hi) = app.selection_range();

    let mut lines: Vec<Line> = Vec::new();
    for row in start_row..end_row {
        let in_selection = app.selecting && row >= sel_lo && row <= sel_hi;
        let offset = row * 16;
        let chunk = &all_bytes[offset..all_bytes.len().min(offset + 16)];

        let mut spans = Vec::new();
        // Offset
        spans.push(Span::styled(
            format!("{offset:08X}  "),
            Style::default().fg(Color::DarkGray),
        ));

        // Hex bytes
        for (i, &byte) in chunk.iter().enumerate() {
            if i == 8 {
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(
                format!("{byte:02X} "),
                Style::default().fg(Color::Green),
            ));
        }
        // Pad remaining
        for i in chunk.len()..16 {
            if i == 8 {
                spans.push(Span::raw(" "));
            }
            spans.push(Span::raw("   "));
        }

        spans.push(Span::raw(" │ "));

        // ASCII
        let ascii: String = chunk
            .iter()
            .map(|&b| {
                if (0x20..=0x7E).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        spans.push(Span::styled(ascii, Style::default().fg(Color::Cyan)));

        if in_selection {
            let sel_style = Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD);
            spans = spans
                .into_iter()
                .map(|s| Span::styled(s.content, sel_style))
                .collect();
        }

        lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);

    // Scrollbar
    if total_rows > inner_height {
        let mut scrollbar_state =
            ScrollbarState::new(total_rows.saturating_sub(inner_height)).position(start_row);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area,
            &mut scrollbar_state,
        );
    }
}

fn draw_input_bar(app: &App, frame: &mut Frame, area: ratatui::layout::Rect) {
    let style = if app.input_focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if app.input_focused {
        " TX (Enter to send, Esc to unfocus) "
    } else {
        " TX (press 'i' to type) "
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(style);

    let cursor_suffix = if app.input_focused { "█" } else { "" };
    let input = Paragraph::new(format!("{}{cursor_suffix}", app.input_text)).block(block);
    frame.render_widget(input, area);
}

fn draw_status_bar(app: &App, frame: &mut Frame, area: ratatui::layout::Rect) {
    let status = if app.connected {
        Span::styled(
            " CONNECTED ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " DISCONNECTED ",
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        )
    };

    let line = Line::from(vec![
        status,
        Span::raw("  "),
        Span::styled("RX: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format_bytes(app.bytes_rx),
            Style::default().fg(Color::Green),
        ),
        Span::raw("  "),
        Span::styled("TX: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format_bytes(app.bytes_tx),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("  "),
        Span::styled(app.status_text(), Style::default().fg(Color::Magenta)),
    ]);

    frame.render_widget(
        Paragraph::new(line)
            .alignment(Alignment::Left)
            .style(Style::default().bg(Color::Black)),
        area,
    );
}

fn format_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

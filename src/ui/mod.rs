mod configure;
mod monitor;
mod port_select;

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{App, AppMode};

pub fn draw(app: &mut App, frame: &mut Frame) {
    match app.mode {
        AppMode::SelectPort => port_select::draw(app, frame),
        AppMode::Configure => {
            port_select::draw(app, frame);
            configure::draw(app, frame);
        }
        AppMode::Monitor => {
            monitor::draw(app, frame);
            if app.show_help {
                draw_help_popup(frame);
            }
        }
        AppMode::Quitting => {
            monitor::draw(app, frame);
            draw_quit_dialog(frame);
        }
    }
}

fn draw_quit_dialog(frame: &mut Frame) {
    let area = centered_rect(40, 7, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Quit? ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let text = Paragraph::new("Disconnect and quit?\n\n[y] Yes  [n] No")
        .block(block)
        .alignment(ratatui::layout::Alignment::Center);

    frame.render_widget(text, area);
}

fn draw_help_popup(frame: &mut Frame) {
    let area = centered_rect(50, 18, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Help — Monitor Keys ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let help_text = "\
  Shift+H     Toggle this help
  c           Enter selection mode
  i           Focus TX input
  Tab         Toggle Text / Hex view
  Ctrl+L      Clear buffer
  ↑ / ↓       Scroll up / down
  PgUp / PgDn Scroll by 20 lines
  Home / End  Jump to top / bottom
  Esc         Disconnect
  q           Quit (with confirmation)

 ── Selection Mode ──
  Shift+↑/↓   Expand selection
  Enter        Copy selection to clipboard
  Esc          Cancel selection";

    let text = Paragraph::new(help_text).block(block);
    frame.render_widget(text, area);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .split(area);
    Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .split(vertical[0])[0]
}

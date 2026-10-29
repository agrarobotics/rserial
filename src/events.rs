use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, AppMode, ConfigField, DisplayMode};

pub fn handle_event(app: &mut App, event: Event) {
    if let Event::Key(key) = event
        && key.kind == KeyEventKind::Press
    {
        // Ctrl+Q quits immediately from any mode
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            app.disconnect();
            app.should_quit = true;
            return;
        }

        // Ctrl+H opens the HTML help page in the browser from any mode.
        // Some terminals send Ctrl+H as Backspace (0x08); with keyboard
        // enhancement flags enabled it arrives as Char('h')+CONTROL, but we
        // accept the Backspace form too for terminals that don't support them.
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('h') | KeyCode::Backspace)
        {
            app.open_help();
            return;
        }

        match app.mode {
            AppMode::SelectPort => handle_select_port(app, key),
            AppMode::Configure => handle_configure(app, key),
            AppMode::Monitor => handle_monitor(app, key),
            AppMode::Quitting => handle_quitting(app, key),
        }
    }
}

fn handle_select_port(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('r') => app.refresh_ports(),
        KeyCode::Up => {
            if let Some(selected) = app.port_list_state.selected()
                && selected > 0
            {
                app.port_list_state.select(Some(selected - 1));
            }
        }
        KeyCode::Down => {
            if let Some(selected) = app.port_list_state.selected()
                && selected + 1 < app.ports.len()
            {
                app.port_list_state.select(Some(selected + 1));
            }
        }
        KeyCode::Enter => {
            if app.port_list_state.selected().is_some() && !app.ports.is_empty() {
                app.mode = AppMode::Configure;
            }
        }
        _ => {}
    }
}

fn handle_configure(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::SelectPort;
        }
        KeyCode::Tab => {
            app.config_field = app.config_field.next();
        }
        KeyCode::BackTab => {
            app.config_field = app.config_field.prev();
        }
        KeyCode::Up => match app.config_field {
            ConfigField::BaudRate => app.config.cycle_baud_up(),
            ConfigField::DataBits => app.config.cycle_data_bits_up(),
            ConfigField::Parity => app.config.cycle_parity_up(),
            ConfigField::StopBits => app.config.cycle_stop_bits(),
        },
        KeyCode::Down => match app.config_field {
            ConfigField::BaudRate => app.config.cycle_baud_down(),
            ConfigField::DataBits => app.config.cycle_data_bits_down(),
            ConfigField::Parity => app.config.cycle_parity_down(),
            ConfigField::StopBits => app.config.cycle_stop_bits(),
        },
        KeyCode::Char(c) => {
            if app.config_field == ConfigField::BaudRate
                && app.config.custom_baud_active
                && c.is_ascii_digit()
            {
                app.config.custom_baud.push(c);
            }
        }
        KeyCode::Backspace => {
            if app.config_field == ConfigField::BaudRate && app.config.custom_baud_active {
                app.config.custom_baud.pop();
            }
        }
        KeyCode::Enter => {
            if let Err(e) = app.connect() {
                app.set_status(&format!("Connection failed: {e}"));
                app.mode = AppMode::SelectPort;
            }
        }
        _ => {}
    }
}

fn handle_monitor(app: &mut App, key: KeyEvent) {
    // Help overlay intercepts all keys
    if app.show_help {
        match key.code {
            KeyCode::Esc => app.show_help = false,
            KeyCode::Char('H') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                app.show_help = false;
            }
            _ => {}
        }
        return;
    }

    // Ctrl+L to clear
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('l') {
        app.clear_buffer();
        return;
    }

    if app.input_focused {
        match key.code {
            KeyCode::Esc => {
                app.input_focused = false;
            }
            KeyCode::Enter => {
                app.send_input();
            }
            KeyCode::Backspace => {
                app.input_text.pop();
            }
            KeyCode::Char(c) => {
                app.input_text.push(c);
            }
            _ => {}
        }
        return;
    }

    if app.selecting {
        handle_selection(app, key);
        return;
    }

    match key.code {
        KeyCode::Char('q') => {
            if app.connected {
                app.mode = AppMode::Quitting;
            } else {
                app.mode = AppMode::SelectPort;
            }
        }
        KeyCode::Esc => {
            if app.connected {
                app.disconnect();
            }
            app.mode = AppMode::SelectPort;
        }
        KeyCode::Tab => {
            app.display_mode = match app.display_mode {
                DisplayMode::Text => DisplayMode::Hex,
                DisplayMode::Hex => DisplayMode::Text,
            };
        }
        KeyCode::Char('c') => {
            app.enter_selection_mode();
        }
        KeyCode::Char('H') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.show_help = true;
        }
        KeyCode::Char('i') => {
            app.input_focused = true;
        }
        KeyCode::Up => {
            let total = app.total_display_lines();
            if app.scroll_offset < total.saturating_sub(1) {
                app.scroll_offset += 1;
            }
        }
        KeyCode::Down => {
            if app.scroll_offset > 0 {
                app.scroll_offset -= 1;
            }
        }
        KeyCode::PageUp => {
            let total = app.total_display_lines();
            app.scroll_offset = (app.scroll_offset + 20).min(total.saturating_sub(1));
        }
        KeyCode::PageDown => {
            app.scroll_offset = app.scroll_offset.saturating_sub(20);
        }
        KeyCode::Home => {
            app.scroll_offset = app.total_display_lines().saturating_sub(1);
        }
        KeyCode::End => {
            app.scroll_offset = 0;
        }
        _ => {}
    }
}

fn handle_selection(app: &mut App, key: KeyEvent) {
    let is_shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        KeyCode::Up if is_shift => {
            app.move_selection_cursor(-1);
        }
        KeyCode::Down if is_shift => {
            app.move_selection_cursor(1);
        }
        KeyCode::PageUp if is_shift => {
            app.move_selection_cursor(-20);
        }
        KeyCode::PageDown if is_shift => {
            app.move_selection_cursor(20);
        }
        KeyCode::Up => {
            let total = app.total_display_lines();
            if app.scroll_offset < total.saturating_sub(1) {
                app.scroll_offset += 1;
            }
        }
        KeyCode::Down => {
            if app.scroll_offset > 0 {
                app.scroll_offset -= 1;
            }
        }
        KeyCode::PageUp => {
            let total = app.total_display_lines();
            app.scroll_offset = (app.scroll_offset + 20).min(total.saturating_sub(1));
        }
        KeyCode::PageDown => {
            app.scroll_offset = app.scroll_offset.saturating_sub(20);
        }
        KeyCode::Home => {
            app.scroll_offset = app.total_display_lines().saturating_sub(1);
        }
        KeyCode::End => {
            app.scroll_offset = 0;
        }
        KeyCode::Enter => {
            let text = match app.display_mode {
                DisplayMode::Text => app.selected_text(),
                DisplayMode::Hex => app.selected_hex_text(),
            };
            match arboard::Clipboard::new() {
                Ok(mut clipboard) => match clipboard.set_text(&text) {
                    Ok(()) => {
                        let (lo, hi) = app.selection_range();
                        app.set_status(&format!("Copied {} lines", hi - lo + 1));
                    }
                    Err(e) => app.set_status(&format!("Clipboard error: {e}")),
                },
                Err(e) => app.set_status(&format!("Clipboard error: {e}")),
            }
            app.exit_selection_mode();
        }
        KeyCode::Esc => {
            app.exit_selection_mode();
        }
        _ => {}
    }
}

fn handle_quitting(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            app.disconnect();
            app.should_quit = true;
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.mode = AppMode::Monitor;
        }
        _ => {}
    }
}

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ratatui::widgets::ListState;
use serialport::{DataBits, Parity, StopBits};

use crate::api::{ApiCommand, ApiState};
use crate::serial::{self, PortInfo, SerialEvent, SerialHandle};

const MAX_LINES: usize = 10_000;
const MAX_RAW_CHUNKS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    SelectPort,
    Configure,
    Monitor,
    Quitting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Text,
    Hex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    BaudRate,
    DataBits,
    Parity,
    StopBits,
}

impl ConfigField {
    pub fn next(self) -> Self {
        match self {
            Self::BaudRate => Self::DataBits,
            Self::DataBits => Self::Parity,
            Self::Parity => Self::StopBits,
            Self::StopBits => Self::BaudRate,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::BaudRate => Self::StopBits,
            Self::DataBits => Self::BaudRate,
            Self::Parity => Self::DataBits,
            Self::StopBits => Self::Parity,
        }
    }
}

pub const BAUD_RATES: &[u32] = &[
    300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600,
];

#[derive(Debug, Clone)]
pub struct SerialConfig {
    pub baud_index: usize,
    pub custom_baud: String,
    pub custom_baud_active: bool,
    pub data_bits: DataBits,
    pub parity: Parity,
    pub stop_bits: StopBits,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            baud_index: 8, // 115200
            custom_baud: String::new(),
            custom_baud_active: false,
            data_bits: DataBits::Eight,
            parity: Parity::None,
            stop_bits: StopBits::One,
        }
    }
}

impl SerialConfig {
    pub fn baud_rate(&self) -> u32 {
        if self.custom_baud_active {
            self.custom_baud.parse().unwrap_or(9600)
        } else {
            BAUD_RATES[self.baud_index]
        }
    }

    pub fn baud_display(&self) -> String {
        if self.custom_baud_active {
            if self.custom_baud.is_empty() {
                "Custom: _".to_string()
            } else {
                format!("Custom: {}", self.custom_baud)
            }
        } else {
            BAUD_RATES[self.baud_index].to_string()
        }
    }

    pub fn cycle_baud_up(&mut self) {
        if self.custom_baud_active {
            self.custom_baud_active = false;
            self.baud_index = 0;
        } else if self.baud_index + 1 < BAUD_RATES.len() {
            self.baud_index += 1;
        } else {
            self.custom_baud_active = true;
        }
    }

    pub fn cycle_baud_down(&mut self) {
        if self.custom_baud_active {
            self.custom_baud_active = false;
            self.baud_index = BAUD_RATES.len() - 1;
        } else if self.baud_index > 0 {
            self.baud_index -= 1;
        } else {
            self.custom_baud_active = true;
        }
    }

    pub fn data_bits_display(&self) -> &str {
        match self.data_bits {
            DataBits::Five => "5",
            DataBits::Six => "6",
            DataBits::Seven => "7",
            DataBits::Eight => "8",
        }
    }

    pub fn cycle_data_bits_up(&mut self) {
        self.data_bits = match self.data_bits {
            DataBits::Five => DataBits::Six,
            DataBits::Six => DataBits::Seven,
            DataBits::Seven => DataBits::Eight,
            DataBits::Eight => DataBits::Five,
        };
    }

    pub fn cycle_data_bits_down(&mut self) {
        self.data_bits = match self.data_bits {
            DataBits::Five => DataBits::Eight,
            DataBits::Six => DataBits::Five,
            DataBits::Seven => DataBits::Six,
            DataBits::Eight => DataBits::Seven,
        };
    }

    pub fn parity_display(&self) -> &str {
        match self.parity {
            Parity::None => "None",
            Parity::Odd => "Odd",
            Parity::Even => "Even",
        }
    }

    pub fn cycle_parity_up(&mut self) {
        self.parity = match self.parity {
            Parity::None => Parity::Odd,
            Parity::Odd => Parity::Even,
            Parity::Even => Parity::None,
        };
    }

    pub fn cycle_parity_down(&mut self) {
        self.parity = match self.parity {
            Parity::None => Parity::Even,
            Parity::Odd => Parity::None,
            Parity::Even => Parity::Odd,
        };
    }

    pub fn stop_bits_display(&self) -> &str {
        match self.stop_bits {
            StopBits::One => "1",
            StopBits::Two => "2",
        }
    }

    pub fn cycle_stop_bits(&mut self) {
        self.stop_bits = match self.stop_bits {
            StopBits::One => StopBits::Two,
            StopBits::Two => StopBits::One,
        };
    }
}

pub struct App {
    pub mode: AppMode,
    pub should_quit: bool,

    // Port selection
    pub ports: Vec<PortInfo>,
    pub port_list_state: ListState,

    // Configuration
    pub config: SerialConfig,
    pub config_field: ConfigField,

    // Connection
    pub serial_handle: Option<SerialHandle>,
    pub serial_rx: Option<Receiver<SerialEvent>>,
    pub connected: bool,
    pub connected_port: String,

    // Monitor display
    pub display_mode: DisplayMode,
    pub rx_lines: VecDeque<String>,
    pub rx_raw: VecDeque<Vec<u8>>,
    pub line_buf: Vec<u8>,
    pub scroll_offset: usize,
    pub input_text: String,
    pub input_focused: bool,

    // Selection mode
    pub selecting: bool,
    pub selection_anchor: usize,
    pub selection_cursor: usize,
    pub last_inner_height: usize,

    // Help overlay
    pub show_help: bool,

    // Stats
    pub bytes_rx: u64,
    pub bytes_tx: u64,
    pub status_message: Option<(String, Instant)>,

    // REST API (only wired up when launched with --api)
    pub api_rx: Option<Receiver<ApiCommand>>,
    pub api_state: Option<Arc<Mutex<ApiState>>>,
    /// When set, an auto-reconnect is scheduled for this instant.
    pub reconnect_at: Option<Instant>,
}

impl App {
    pub fn new() -> Self {
        let ports = serial::list_ports();
        let mut port_list_state = ListState::default();
        if !ports.is_empty() {
            port_list_state.select(Some(0));
        }
        Self {
            mode: AppMode::SelectPort,
            should_quit: false,
            ports,
            port_list_state,
            config: SerialConfig::default(),
            config_field: ConfigField::BaudRate,
            serial_handle: None,
            serial_rx: None,
            connected: false,
            connected_port: String::new(),
            display_mode: DisplayMode::Text,
            rx_lines: VecDeque::new(),
            rx_raw: VecDeque::new(),
            line_buf: Vec::new(),
            scroll_offset: 0,
            input_text: String::new(),
            input_focused: false,
            selecting: false,
            selection_anchor: 0,
            selection_cursor: 0,
            last_inner_height: 0,
            show_help: false,
            bytes_rx: 0,
            bytes_tx: 0,
            status_message: None,
            api_rx: None,
            api_state: None,
            reconnect_at: None,
        }
    }

    pub fn refresh_ports(&mut self) {
        self.ports = serial::list_ports();
        if self.ports.is_empty() {
            self.port_list_state.select(None);
        } else {
            let idx = self.port_list_state.selected().unwrap_or(0);
            self.port_list_state
                .select(Some(idx.min(self.ports.len() - 1)));
        }
    }

    pub fn connect(&mut self) -> color_eyre::Result<()> {
        let port_name = match self.port_list_state.selected() {
            Some(idx) => self.ports[idx].name.clone(),
            None => return Ok(()),
        };

        let (rx, handle) = serial::spawn_serial_reader(
            &port_name,
            self.config.baud_rate(),
            self.config.data_bits,
            self.config.parity,
            self.config.stop_bits,
        )?;

        self.serial_rx = Some(rx);
        self.serial_handle = Some(handle);
        self.connected = true;
        self.connected_port = port_name;
        self.bytes_rx = 0;
        self.bytes_tx = 0;
        self.rx_lines.clear();
        self.rx_raw.clear();
        self.line_buf.clear();
        self.scroll_offset = 0;
        self.mode = AppMode::Monitor;
        self.set_status("Connected");
        Ok(())
    }

    /// Reconnect to the last-used port with the current config (used by the
    /// REST API auto-reconnect cycle). Unlike [`connect`], this preserves the
    /// existing receive buffer so the log continues across the reconnect.
    pub fn reconnect(&mut self) -> color_eyre::Result<()> {
        if self.connected {
            return Ok(());
        }
        if self.connected_port.is_empty() {
            self.set_status("Reconnect: no previous port");
            return Ok(());
        }
        let port_name = self.connected_port.clone();
        let (rx, handle) = serial::spawn_serial_reader(
            &port_name,
            self.config.baud_rate(),
            self.config.data_bits,
            self.config.parity,
            self.config.stop_bits,
        )?;
        self.serial_rx = Some(rx);
        self.serial_handle = Some(handle);
        self.connected = true;
        self.connected_port = port_name;
        self.mode = AppMode::Monitor;
        self.set_status("Reconnected (API)");
        Ok(())
    }

    pub fn disconnect(&mut self) {
        if let Some(mut handle) = self.serial_handle.take() {
            handle.stop();
        }
        self.serial_rx = None;
        self.connected = false;
        self.set_status("Disconnected");
    }

    pub fn poll_serial(&mut self) {
        // Drain events into a local vec to avoid borrow conflicts
        let events: Vec<SerialEvent> = {
            let Some(rx) = &self.serial_rx else {
                return;
            };
            let mut events = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(event) => events.push(event),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        events.push(SerialEvent::Disconnected);
                        break;
                    }
                }
            }
            events
        };

        for event in events {
            match event {
                SerialEvent::Data(data) => {
                    self.bytes_rx += data.len() as u64;
                    self.rx_raw.push_back(data.clone());
                    while self.rx_raw.len() > MAX_RAW_CHUNKS {
                        self.rx_raw.pop_front();
                    }
                    for &byte in &data {
                        if byte == b'\n' {
                            let line = String::from_utf8_lossy(&self.line_buf).into_owned();
                            self.rx_lines.push_back(line);
                            self.line_buf.clear();
                            while self.rx_lines.len() > MAX_LINES {
                                self.rx_lines.pop_front();
                                if self.selecting {
                                    if self.selection_anchor == 0 && self.selection_cursor == 0 {
                                        self.selecting = false;
                                        self.status_message = Some((
                                            "Selection lost (buffer overflow)".to_string(),
                                            Instant::now(),
                                        ));
                                    } else {
                                        self.selection_anchor =
                                            self.selection_anchor.saturating_sub(1);
                                        self.selection_cursor =
                                            self.selection_cursor.saturating_sub(1);
                                    }
                                }
                            }
                        } else if byte != b'\r' {
                            self.line_buf.push(byte);
                        }
                    }
                }
                SerialEvent::Error(e) => {
                    self.set_status(&format!("Error: {e}"));
                }
                SerialEvent::Disconnected => {
                    self.connected = false;
                    self.serial_handle = None;
                    self.serial_rx = None;
                    self.set_status("Port disconnected");
                    return;
                }
            }
        }
    }

    pub fn send_input(&mut self) {
        if self.input_text.is_empty() {
            return;
        }
        if let Some(handle) = &self.serial_handle {
            let mut data = self.input_text.clone().into_bytes();
            data.push(b'\n');
            self.bytes_tx += data.len() as u64;
            if handle.tx_sender.send(data).is_err() {
                self.set_status("Send failed: port closed");
            }
        }
        self.input_text.clear();
    }

    pub fn clear_buffer(&mut self) {
        self.rx_lines.clear();
        self.rx_raw.clear();
        self.line_buf.clear();
        self.scroll_offset = 0;
        self.selecting = false;
    }

    pub fn total_display_lines(&self) -> usize {
        let count = self.rx_lines.len();
        // Add 1 for partial line if it exists
        if !self.line_buf.is_empty() {
            count + 1
        } else {
            count
        }
    }

    pub fn enter_selection_mode(&mut self) {
        self.selecting = true;
        let total = self.total_display_lines();
        let cursor_line = total.saturating_sub(self.scroll_offset + 1);
        self.selection_anchor = cursor_line;
        self.selection_cursor = cursor_line;
    }

    pub fn exit_selection_mode(&mut self) {
        self.selecting = false;
    }

    pub fn selection_range(&self) -> (usize, usize) {
        let lo = self.selection_anchor.min(self.selection_cursor);
        let hi = self.selection_anchor.max(self.selection_cursor);
        (lo, hi)
    }

    pub fn move_selection_cursor(&mut self, delta: isize) {
        let total = match self.display_mode {
            DisplayMode::Text => self.total_display_lines(),
            DisplayMode::Hex => self.total_hex_rows(),
        };
        let new_cursor = if delta < 0 {
            self.selection_cursor.saturating_sub(delta.unsigned_abs())
        } else {
            (self.selection_cursor + delta as usize).min(total.saturating_sub(1))
        };
        self.selection_cursor = new_cursor;

        // Auto-scroll to keep cursor visible
        if self.last_inner_height > 0 {
            let end = total.saturating_sub(self.scroll_offset);
            let start = end.saturating_sub(self.last_inner_height);
            if self.selection_cursor < start {
                self.scroll_offset =
                    total.saturating_sub(self.selection_cursor + self.last_inner_height);
            } else if self.selection_cursor >= end {
                self.scroll_offset = total.saturating_sub(self.selection_cursor + 1);
            }
        }
    }

    pub fn total_hex_rows(&self) -> usize {
        let total_bytes: usize = self.rx_raw.iter().map(|c| c.len()).sum();
        total_bytes.div_ceil(16)
    }

    pub fn selected_text(&self) -> String {
        let (lo, hi) = self.selection_range();
        let mut lines: Vec<String> = Vec::new();
        for i in lo..=hi {
            if i < self.rx_lines.len() {
                lines.push(self.rx_lines[i].clone());
            } else if i == self.rx_lines.len() && !self.line_buf.is_empty() {
                lines.push(String::from_utf8_lossy(&self.line_buf).into_owned());
            }
        }
        lines.join("\n")
    }

    pub fn selected_hex_text(&self) -> String {
        let all_bytes: Vec<u8> = self.rx_raw.iter().flat_map(|c| c.iter().copied()).collect();
        let (lo, hi) = self.selection_range();
        let mut lines = Vec::new();
        for row in lo..=hi {
            let offset = row * 16;
            if offset >= all_bytes.len() {
                break;
            }
            let chunk = &all_bytes[offset..all_bytes.len().min(offset + 16)];
            let mut hex_part = format!("{offset:08X}  ");
            for (i, &b) in chunk.iter().enumerate() {
                if i == 8 {
                    hex_part.push(' ');
                }
                hex_part.push_str(&format!("{b:02X} "));
            }
            for i in chunk.len()..16 {
                if i == 8 {
                    hex_part.push(' ');
                }
                hex_part.push_str("   ");
            }
            hex_part.push_str(" | ");
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
            hex_part.push_str(&ascii);
            lines.push(hex_part);
        }
        lines.join("\n")
    }

    pub fn set_status(&mut self, msg: &str) {
        self.status_message = Some((msg.to_string(), Instant::now()));
    }

    /// Open the HTML help page in the default browser (Ctrl+H).
    ///
    /// Mirrors ida_connectivity_chkr's `open_wiki`, but resolves the path
    /// robustly since rserial is usually launched from a GNOME `.desktop`
    /// entry with an arbitrary working directory: we look next to the binary
    /// (release install) and in the project dir (dev run).
    pub fn open_help(&mut self) {
        const HELP_REL: &str = "docs_about__rserial/rserial__help.html";
        const HELP_NAME: &str = "rserial__help.html";

        let mut candidates: Vec<PathBuf> = Vec::new();
        // Next to the executable (install.sh copies the HTML alongside the binary)
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            candidates.push(dir.join(HELP_NAME));
            candidates.push(dir.join(HELP_REL));
        }
        // Project dir at compile time (dev / `cargo run`)
        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(HELP_REL));
        // Current working directory (matches the ida_connectivity_chkr pattern)
        candidates.push(PathBuf::from(HELP_REL));

        match candidates.into_iter().find(|p| p.exists()) {
            Some(path) => match Command::new("xdg-open").arg(&path).spawn() {
                Ok(_) => self.set_status("Opened help in browser"),
                Err(e) => self.set_status(&format!("Could not open help: {e}")),
            },
            None => self.set_status("Help page not found"),
        }
    }

    pub fn status_text(&self) -> &str {
        if let Some((ref msg, instant)) = self.status_message
            && instant.elapsed().as_secs() < 5
        {
            return msg;
        }
        ""
    }

    /// Drain any REST API commands, service a pending timed reconnect, and
    /// publish the current status snapshot. No-op unless launched with `--api`.
    pub fn poll_api(&mut self) {
        // Drain commands into a local vec to avoid borrowing self across the loop.
        let cmds: Vec<ApiCommand> = match &self.api_rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for cmd in cmds {
            match cmd {
                ApiCommand::Disconnect => {
                    self.reconnect_at = None;
                    self.disconnect();
                    self.set_status("API: disconnected");
                }
                ApiCommand::Reconnect => {
                    self.reconnect_at = None;
                    if let Err(e) = self.reconnect() {
                        self.set_status(&format!("API reconnect failed: {e}"));
                    }
                }
                ApiCommand::CycleReconnect { secs } => {
                    self.disconnect();
                    self.reconnect_at = Some(Instant::now() + Duration::from_secs(secs));
                    self.set_status(&format!("API: disconnected, reconnect in {secs}s"));
                }
            }
        }

        // Fire a scheduled auto-reconnect once its deadline passes.
        if let Some(at) = self.reconnect_at
            && Instant::now() >= at
        {
            self.reconnect_at = None;
            if let Err(e) = self.reconnect() {
                self.set_status(&format!("API reconnect failed: {e}"));
            }
        }

        // Publish the snapshot for GET /api/status.
        if let Some(state) = &self.api_state
            && let Ok(mut s) = state.lock()
        {
            s.connected = self.connected;
            s.port = self.connected_port.clone();
            s.reconnect_in_secs = self
                .reconnect_at
                .map(|at| at.saturating_duration_since(Instant::now()).as_secs());
        }
    }
}

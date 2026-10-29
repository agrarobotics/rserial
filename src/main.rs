mod api;
mod app;
mod events;
mod serial;
mod ui;

use std::io;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::api::{ApiState, DEFAULT_API_PORT};

use color_eyre::Result;
use crossterm::event::{
    self, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// Parsed command-line options.
struct Cli {
    api: bool,
    api_port: u16,
}

fn parse_cli() -> Result<Cli> {
    let mut cli = Cli {
        api: false,
        api_port: DEFAULT_API_PORT,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--api" => cli.api = true,
            "--api-port" => {
                let val = args
                    .next()
                    .ok_or_else(|| color_eyre::eyre::eyre!("--api-port requires a value"))?;
                cli.api_port = val
                    .parse()
                    .map_err(|_| color_eyre::eyre::eyre!("invalid --api-port: {val}"))?;
                cli.api = true;
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                return Err(color_eyre::eyre::eyre!("unknown argument: {other}"));
            }
        }
    }
    Ok(cli)
}

fn print_usage() {
    println!(
        "rserial — TUI serial port monitor\n\n\
         USAGE:\n    rserial [OPTIONS]\n\n\
         OPTIONS:\n\
         \x20   --api               Enable the localhost REST API (127.0.0.1:{DEFAULT_API_PORT})\n\
         \x20   --api-port <PORT>   Enable the REST API on a custom port (implies --api)\n\
         \x20   -h, --help          Show this help\n\n\
         REST API (localhost only):\n\
         \x20   GET  /api/status        connection status as JSON\n\
         \x20   POST /api/disconnect    disconnect the port now\n\
         \x20   POST /api/reconnect     reconnect to the last port/config\n\
         \x20   POST /api/cycle?secs=30 disconnect now, auto-reconnect after N seconds\n"
    );
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let cli = parse_cli()?;

    // Setup terminal
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;

    // Enable keyboard enhancement (kitty protocol) so Ctrl+H is reported as
    // Char('h')+CONTROL instead of a bare Backspace. Terminals that don't
    // support it simply ignore the request — we fall back gracefully.
    let enhanced = matches!(terminal::supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        let _ = crossterm::execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &cli);

    // Restore terminal
    if enhanced {
        let _ = crossterm::execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, cli: &Cli) -> Result<()> {
    let mut app = app::App::new();

    // Optionally start the localhost REST API.
    if cli.api {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(ApiState::default()));
        let bind = format!("127.0.0.1:{}", cli.api_port);
        match api::spawn_api_server(&bind, tx, Arc::clone(&state)) {
            Ok(()) => {
                app.api_rx = Some(rx);
                app.api_state = Some(state);
                app.set_status(&format!("REST API on http://{bind}"));
            }
            Err(e) => {
                app.set_status(&format!("REST API disabled: {e}"));
            }
        }
    }

    loop {
        app.poll_serial();
        app.poll_api();

        terminal.draw(|frame| ui::draw(&mut app, frame))?;

        if app.should_quit {
            break;
        }

        if event::poll(Duration::from_millis(16))? {
            let ev = event::read()?;
            events::handle_event(&mut app, ev);
        }
    }

    Ok(())
}

# rserial

TUI serial port monitor written in Rust with ratatui.

## System Dependencies

The `serialport` crate requires `libudev` on Linux:

```bash
sudo apt install libudev-dev pkg-config
```

If `libudev-dev` is unavailable, you can work around it with a local pkgconfig and symlink:

```bash
# Create local pkgconfig file
mkdir -p ~/.local/lib/pkgconfig
cat > ~/.local/lib/pkgconfig/libudev.pc << 'EOF'
prefix=/usr
exec_prefix=${prefix}
libdir=${exec_prefix}/lib/x86_64-linux-gnu
includedir=${prefix}/include
Name: libudev
Description: libudev
Version: 255
Libs: -L${libdir} -ludev
Cflags: -I${includedir}
EOF

# Create linker symlink
ln -sf /usr/lib/x86_64-linux-gnu/libudev.so.1 ~/.local/lib/libudev.so

# Build with workaround paths
PKG_CONFIG_PATH=~/.local/lib/pkgconfig RUSTFLAGS="-L ~/.local/lib" cargo build --release
```

## REST API (`--api`)

Off by default. Enable a localhost-only HTTP API so other programs can drive the
serial connection (e.g. free the port for a flasher, then auto-reconnect):

```bash
rserial --api                 # default port 8791 (127.0.0.1 only)
rserial --api-port 9000       # custom port (implies --api)
```

| Method | Path                  | Action                                          |
|--------|-----------------------|-------------------------------------------------|
| GET    | `/api/status`         | JSON `{connected, port, reconnect_in_secs}`     |
| POST   | `/api/disconnect`     | Disconnect the port now                         |
| POST   | `/api/reconnect`      | Reconnect to the last port/config now           |
| POST   | `/api/cycle?secs=30`  | Disconnect now, auto-reconnect after N s (def 30)|

Implementation: `src/api.rs` runs a blocking `tiny_http` server on a background
thread. It only sends `ApiCommand`s over an mpsc channel and reads a shared
`Arc<Mutex<ApiState>>` snapshot — the UI thread owns all serial state.
`App::poll_api()` (called each main-loop tick) drains commands, fires the
`reconnect_at` timer, and republishes the status snapshot.

## Build & Run

- Build: `cargo build`
- Run: `cargo run`
- Release build: `cargo build --release`
- Check (no codegen): `cargo check`
- Run tests: `cargo test`
- Lint: `cargo clippy`
- Format: `cargo fmt`

If using the libudev workaround, prefix build/run commands with:
```bash
PKG_CONFIG_PATH=~/.local/lib/pkgconfig RUSTFLAGS="-L ~/.local/lib" cargo run
```

## Install

`./install.sh` builds release and copies the binary to:

```
~/scottsoft_wv/sdn_release_rserial/rserial
```

A GNOME launcher entry lives at `~/.local/share/applications/sdn_serialmon.desktop`:

- **Name:** `SDN Serialmon RS` (searchable under "sd" and "rs" — mirrors the flameshot launcher)
- **Exec:** wraps the TUI in `gnome-terminal --title="SDN Serialmon RS" -- <binary>`
- **Icon:** stock `utilities-terminal` (no custom icon yet)

After modifying the `.desktop` file, refresh with `update-desktop-database ~/.local/share/applications/`.

## Project Structure

- `Cargo.toml` — package manifest (edition 2024)
- `install.sh` — release-build + copy-to-release-dir installer
- `src/main.rs` — entry point, CLI parsing (`--api`/`--api-port`), terminal init/restore, main loop
- `src/app.rs` — App state machine, SerialConfig, display modes, `open_help()`, `reconnect()`, `poll_api()`
- `src/api.rs` — localhost REST API (`tiny_http`), `ApiCommand`/`ApiState`
- `src/serial.rs` — port listing (USB first), reader/writer threads (`spawn_serial` → generic `spawn_io`, testable with fake I/O)
- `src/engine.rs` — `Engine`: shared frame log (epoch, limit), stats, framing config; every frame goes through `emit()`
- `src/bus.rs` — `FrameBus`: session/seq, per-subscriber bounded queues (drop-oldest + `dropped_before`), batching (from RSerialDebugAssistant, MIT)
- `src/framing.rs` — `FrameSegmenter`: Timeout / Combined (delimiter+timeout) framing, 64 KiB cap (from RSerialDebugAssistant, MIT)
- `src/events.rs` — key event dispatcher per app mode (global Ctrl+Q / Ctrl+H)
- `src/ui/mod.rs` — draw dispatcher, quit confirmation dialog
- `src/ui/port_select.rs` — port list screen
- `src/ui/configure.rs` — serial config popup (baud, data bits, parity, stop bits)
- `src/ui/monitor.rs` — data display (text/hex), TX input bar, status bar

## Architecture

- No async runtime — serial I/O uses `std::thread` + `std::sync::mpsc` channels
- One `Arc<Engine>` lives for the whole process; connections come and go. A fresh connect starts a new bus session (seq from 1) and resets stats; an API auto-reconnect starts a session but keeps stats and the TUI buffer
- Reader thread: 50 ms read timeout → `FrameSegmenter` (default Combined / AnyNewline / 50 ms) → `engine.emit(Rx, frame)`; the idle check also runs *before* feeding fresh bytes, so a gap spent inside a blocking `read()` still cuts the frame. Partial frame is flushed on stop and on read error
- Writer thread: receives `Vec<u8>` from UI, writes to the cloned handle (1 s write timeout), then `engine.emit(Tx, data)` — TX and RX share one seq
- `emit()` holds the log lock across seq allocation, log append and bus publish, so every reader sees seq order
- The TUI is a bus subscriber (`try_recv_batch` once per tick) and shows RX only; the web UI / HTTP stream will be further subscribers (porting step 2)
- `SerialEvent` now carries only `Error` / `Disconnected`; data never goes through it
- Tests: `cargo test` (fake reader/writer, no hardware). Real-port round trip over a socat pty pair: see the ignored `serial::tests::real_port_roundtrip`
- Stop signal: `Arc<AtomicBool>` checked each iteration
- Main loop runs at ~60fps (16ms poll interval)
- REST API (opt-in): separate `tiny_http` thread → `ApiCommand` mpsc → UI thread; status published back via `Arc<Mutex<ApiState>>`

## Key Bindings

### Global (any screen)
- `Ctrl+Q` quit immediately (disconnects first)
- `Ctrl+H` open the HTML help page in the browser (`docs_about__rserial/rserial__help.html`)

Ctrl+H needs the byte to arrive as `Char('h')+CONTROL` rather than a bare
Backspace — `main.rs` pushes crossterm's `DISAMBIGUATE_ESCAPE_CODES` keyboard
enhancement flag when the terminal supports it (VTE ≥ 0.76 / kitty protocol),
and the handler also accepts the `Backspace+CONTROL` form as a fallback.

### Port Selection
- `↑↓` navigate, `Enter` select, `r` refresh, `q` quit

### Configuration
- `Tab/Shift+Tab` cycle fields, `↑↓` cycle values, `Enter` connect, `Esc` back

### Monitor
- `Tab` toggle text/hex, `↑↓/PgUp/PgDn/Home/End` scroll
- `i` focus TX input, `Enter` send, `Esc` unfocus
- `Ctrl+L` clear buffer, `Esc` disconnect, `q` quit (with confirmation)

## Conventions

- Rust edition 2024
- Follow standard Rust naming: `snake_case` for functions/variables, `CamelCase` for types
- Run `cargo clippy` and `cargo fmt` before committing
- Keep dependencies minimal — only add crates when clearly needed
- Prefer safe Rust; avoid `unsafe` unless absolutely necessary and well-documented

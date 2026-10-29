# RSerialDebugAssistant → rserial: analysis & porting plan

Source: `others/RSerialDebugAssistant` (github.com/Gyanano/RSerialDebugAssistant, v1.4.2,
commit `4eb6171`) — Tauri 2 + React 18. **MIT, © 2025 Gyanano**: copied code keeps that
notice (add `THIRD_PARTY_NOTICES.md` when the first file lands).

Goal: rserial gets RSDA's best features, but with **no React / no Tauri**:

* UI = **tsgo7 pages** (plain TypeScript compiled by `tsgo`, like `rsterm/settings/` and
  `rumima/help/`), embedded in the binary with `include_str!`, served by rserial itself.
* **rserial-rest-api** = 100% remote control, rumima style: `GET /` returns the spec as
  text, replies `{"ok":true}` / `{"ok":false,"error":…}`, `GET`/`POST /settings`, a POST
  answers only after it is *applied*. Rule: **every UI action is an endpoint; the tsgo7 UI
  uses only the public API.** Endpoint list → `docs_about__rserial/rserial_rest_api.md`.
* **Decision (Scottie, 2026-10-29): keep both UIs.** One rserial process owns the port;
  the ratatui TUI and the tsgo7 web UI are both readers of the same frame bus, and both
  drive it through the same command path.
* Server like rsterm: loopback, fixed default port with fallback to the next free one,
  `$XDG_RUNTIME_DIR/rserial/api.json` (`{port, token, pid}`, mode 0600).

## What RSDA has (backend, Rust)

| Feature | RSDA file | Take? |
|---|---|---|
| FrameSegmenter: Timeout / Combined (delimiter+timeout), 64 KiB cap, idle check *before* feeding new bytes | `src-tauri/src/framing.rs` (~170 LOC + tests, pure) | **Yes, near-verbatim** |
| FrameBus: session + seq, per-subscriber bounded queue, drop-oldest + `dropped_before`, batching (16 ms / 512 frames) | `bus.rs` (~260 LOC, std-only) | **Yes** — TUI and HTTP/SSE become two subscribers |
| Log ring (100..10000), TX+RX share one seq, stats (bytes, connect time), reader-death detection, bounded join | `serial_manager.rs` | Yes, rewritten into our `serial.rs` |
| Port list with USB VID/PID/serial, USB first | `serial_manager.rs:147-189` | Yes |
| Display formatting: hex, UTF-8/GBK, control-char glyphs (␊ ␍ ␉ ␀ ␛) | `serial_manager.rs:950-1045` | Yes (GBK via `encoding_rs`, optional) |
| Text + raw recording to auto-named files | `serial_manager.rs:739-930` | Yes — fix TX/RX mixed into raw `.bin` |
| Export TXT / CSV / JSON | `serial_manager.rs:588-652` | Yes |
| `PortOpener` seam + `ScriptedPort` fake for tests | `serial_manager.rs` | **Yes** — first real tests for rserial |
| Updater (GitHub/NSIS/dmg), session save/load (dead code) | `updater.rs`, … | No |

## What RSDA has (frontend → plain TS)

| Feature | RSDA file | Take? |
|---|---|---|
| Checksums XOR / ADD8 / CRC8 / CRC16-Modbus / CCITT-CRC16, start/end range (negative end) | `frontend/src/utils/checksum.ts` (pure) | **Yes, verbatim** — and also in Rust so the API can append it |
| Hex input auto-format, line endings (none / \r / \n / \r\n) | `SendPanel.tsx:31`, `App.tsx:623` | Yes |
| Scheduled send (10 ms..60 s), Ctrl+Enter | `SendPanel.tsx:185-240` | Yes — **server-side timer**, so it is API-controllable |
| Quick-command lists (named, hex/text, line ending, send selected, loop send) | `QuickCommandPanel.tsx` | **Yes** — stored server-side (`settings`), not localStorage |
| Log search (Ctrl+F, next/prev, highlight), smart auto-scroll, line numbers | `LogViewer.tsx` | Yes |
| Snapshot + epoch + seq dedupe sync | `hooks/useSerialLogs.ts` | Yes, as `GET /log/snapshot` + SSE `GET /log/stream?after=<seq>` |

Known RSDA bugs not to copy: Mark/Space parity and 1.5 stop bits silently ignored; frame
timestamp taken at pump time, not receive time; failed-UTF-8 frame shown all-hex; text
export ignores GBK; loop send overlaps when a send is slow; one big mutex held across
blocking writes; frontend log cap hard-coded at 1000.

## rserial-rest-api — first draft

| Method & path | Body | Effect |
|---|---|---|
| `GET /` | — | this spec (plain text) |
| `GET /health` | — | `{ok, version, connected}` |
| `GET /ports` | — | ports with USB info |
| `GET /status` | — | `{connected, port, config, rx_bytes, tx_bytes, since, reconnect_in_secs, recording}` |
| `POST /connect` | `{port, baud?, data_bits?, parity?, stop_bits?, flow?}` | open port |
| `POST /disconnect` · `/reconnect` | — | as today |
| `POST /cycle` | `{secs?:30}` | disconnect, auto-reconnect after N s (flasher use) |
| `POST /send` | `{data, format:"text"\|"hex", line_ending?, checksum?:{algo,start,end}}` | transmit |
| `POST /send/repeat` | `{…send body, interval_ms}` / `{stop:true}` | scheduled send |
| `GET /log/snapshot` | — | `{epoch, session, entries}` |
| `GET /log/stream?after=seq` | — | SSE frame batches (UI + scripts) |
| `POST /log/clear` | — | clear, bumps epoch |
| `POST /log/export` | `{path, format:"txt"\|"csv"\|"json"}` | export |
| `POST /record/start` · `/record/stop` | `{kind:"text"\|"raw"}` | recording |
| `GET/POST /quick` | lists / `{list, …}` | quick-command CRUD |
| `POST /quick/run` | `{list, ids?, loop_ms?}` / `{stop:true}` | run quick commands |
| `GET /settings` · `POST /settings` | `{key:value,…}` | framing, display, log limit, dirs — live + persisted; unknown key → error, never silently dropped |
| `POST /ui/open` | — | open the tsgo7 UI in the browser |

Existing `/api/status|disconnect|reconnect|cycle` stay as aliases so current flasher scripts keep working.

## Order of work

1. ✅ (2026-10-29) Core: `framing.rs` + `bus.rs` from RSDA, `engine.rs` (log ring, stats, framing config), scripted-reader tests + a socat real-port test. The TUI is a bus subscriber. Deviation from RSDA: AnyNewline waits for the LF of a CRLF split across reads.
2. rserial-rest-api: the full table above, the token, `api.json`, `GET /` spec, `rserial_rest_api.md`.
3. tsgo7 UI in `ui_web/` (`index.html` + `src/*.ts`, `build.sh`, built `dist/` committed): monitor, send, quick commands, checksum, search, settings.
4. Recording/export, GBK, help page update, `install.sh` and the `.desktop` launcher.

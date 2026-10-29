//! Shared serial state that outlives a single connection: the frame bus, the
//! frame log, byte statistics and the framing config.
//!
//! One `Arc<Engine>` is created at startup. The reader and writer threads
//! `emit` every frame through it; the TUI (and, later, the HTTP API) are bus
//! subscribers and read the log for snapshots. Nothing here does I/O.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use crate::bus::{BatchPolicy, Direction, Frame, FrameBus, SessionId, Subscription};
use crate::framing::FrameSegmentationConfig;

pub const DEFAULT_LOG_LIMIT: usize = 10_000;
pub const MIN_LOG_LIMIT: usize = 100;
pub const MAX_LOG_LIMIT: usize = 100_000;
pub const MIN_FRAME_TIMEOUT_MS: u64 = 10;
pub const MAX_FRAME_TIMEOUT_MS: u64 = 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_frames: u64,
    pub tx_frames: u64,
    /// When the current session was started by a fresh connect.
    pub connected_since: Option<SystemTime>,
}

/// Bounded frame log. `epoch` increases on every clear, so a reader holding
/// an older snapshot can tell its view is stale.
struct FrameLog {
    entries: VecDeque<Frame>,
    limit: usize,
    epoch: u64,
}

pub struct LogSnapshot {
    pub epoch: u64,
    pub session: SessionId,
    pub entries: Vec<Frame>,
}

pub struct Engine {
    bus: FrameBus,
    log: Mutex<FrameLog>,
    stats: Mutex<Stats>,
    framing: Mutex<FrameSegmentationConfig>,
    /// Bumped on every framing change so the reader only re-reads the config
    /// when it actually changed.
    framing_gen: AtomicU64,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            bus: FrameBus::new(),
            log: Mutex::new(FrameLog {
                entries: VecDeque::new(),
                limit: DEFAULT_LOG_LIMIT,
                epoch: 0,
            }),
            stats: Mutex::new(Stats::default()),
            framing: Mutex::new(FrameSegmentationConfig::default()),
            framing_gen: AtomicU64::new(0),
        }
    }

    pub fn subscribe(&self, policy: BatchPolicy) -> Subscription {
        self.bus.subscribe(policy)
    }

    /// Start a new bus session (seq restarts at 1). A fresh connect resets
    /// the statistics; an auto-reconnect keeps counting.
    pub fn start_session(&self, reset_stats: bool) -> SessionId {
        let id = self.bus.start_session();
        let mut stats = self.stats.lock().unwrap();
        if reset_stats {
            *stats = Stats {
                connected_since: Some(SystemTime::now()),
                ..Stats::default()
            };
        }
        id
    }

    /// Record one frame: allocate its seq, append it to the log, count it
    /// and fan it out. The log lock is held across allocation and publish so
    /// the log and every subscriber see frames in seq order even though the
    /// reader (RX) and writer (TX) threads emit concurrently.
    pub fn emit(&self, dir: Direction, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        let mut log = self.log.lock().unwrap();
        let frame = self.bus.alloc_frame(dir, data);
        {
            let mut stats = self.stats.lock().unwrap();
            match dir {
                Direction::Rx => {
                    stats.rx_bytes += frame.data.len() as u64;
                    stats.rx_frames += 1;
                }
                Direction::Tx => {
                    stats.tx_bytes += frame.data.len() as u64;
                    stats.tx_frames += 1;
                }
            }
        }
        log.entries.push_back(frame.clone());
        while log.entries.len() > log.limit {
            log.entries.pop_front();
        }
        self.bus.publish(&frame);
    }

    pub fn stats(&self) -> Stats {
        *self.stats.lock().unwrap()
    }

    /// Empty the log; returns the new epoch.
    pub fn clear_log(&self) -> u64 {
        let mut log = self.log.lock().unwrap();
        log.entries.clear();
        log.epoch += 1;
        log.epoch
    }

    pub fn snapshot(&self) -> LogSnapshot {
        let log = self.log.lock().unwrap();
        LogSnapshot {
            epoch: log.epoch,
            session: self.bus.current_session(),
            entries: log.entries.iter().cloned().collect(),
        }
    }

    pub fn log_limit(&self) -> usize {
        self.log.lock().unwrap().limit
    }

    /// Set the log size (clamped to MIN..=MAX_LOG_LIMIT); returns the value used.
    pub fn set_log_limit(&self, limit: usize) -> usize {
        let limit = limit.clamp(MIN_LOG_LIMIT, MAX_LOG_LIMIT);
        let mut log = self.log.lock().unwrap();
        log.limit = limit;
        while log.entries.len() > limit {
            log.entries.pop_front();
        }
        limit
    }

    pub fn framing(&self) -> FrameSegmentationConfig {
        self.framing.lock().unwrap().clone()
    }

    pub fn framing_gen(&self) -> u64 {
        self.framing_gen.load(Ordering::SeqCst)
    }

    /// Change the framing config; takes effect on the reader's next loop
    /// without a reconnect. The timeout is clamped to 10..=1000 ms.
    pub fn set_framing(&self, mut config: FrameSegmentationConfig) {
        config.timeout_ms = config
            .timeout_ms
            .clamp(MIN_FRAME_TIMEOUT_MS, MAX_FRAME_TIMEOUT_MS);
        *self.framing.lock().unwrap() = config;
        self.framing_gen.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::BatchPolicy;
    use std::time::Duration;

    fn policy() -> BatchPolicy {
        BatchPolicy {
            max_delay: Duration::from_millis(5),
            min_interval: Duration::from_millis(0),
            max_bytes: 1024,
            max_frames: 64,
            queue_frames: 64,
        }
    }

    #[test]
    fn emit_logs_counts_and_publishes() {
        let engine = Engine::new();
        let sub = engine.subscribe(policy());
        engine.start_session(true);
        engine.emit(Direction::Rx, b"hello\n".to_vec());
        engine.emit(Direction::Tx, b"AT\r\n".to_vec());
        engine.emit(Direction::Rx, Vec::new()); // ignored

        let stats = engine.stats();
        assert_eq!((stats.rx_bytes, stats.tx_bytes), (6, 4));
        assert_eq!((stats.rx_frames, stats.tx_frames), (1, 1));
        assert!(stats.connected_since.is_some());

        let snap = engine.snapshot();
        let seqs: Vec<u64> = snap.entries.iter().map(|f| f.seq).collect();
        assert_eq!(seqs, vec![1, 2]);

        let batch = sub.try_recv_batch().unwrap();
        assert_eq!(batch.frames.len(), 2);
        assert_eq!(batch.frames[1].dir, Direction::Tx);
    }

    #[test]
    fn log_limit_drops_oldest_and_clear_bumps_epoch() {
        let engine = Engine::new();
        engine.start_session(true);
        assert_eq!(engine.set_log_limit(1), MIN_LOG_LIMIT);
        for i in 0..150u32 {
            engine.emit(Direction::Rx, i.to_le_bytes().to_vec());
        }
        let snap = engine.snapshot();
        assert_eq!(snap.entries.len(), MIN_LOG_LIMIT);
        assert_eq!(snap.entries[0].seq, 51);

        assert_eq!(engine.clear_log(), snap.epoch + 1);
        assert!(engine.snapshot().entries.is_empty());
    }

    #[test]
    fn reconnect_keeps_stats_fresh_connect_resets() {
        let engine = Engine::new();
        engine.start_session(true);
        engine.emit(Direction::Rx, b"abc".to_vec());
        engine.start_session(false);
        assert_eq!(engine.stats().rx_bytes, 3);
        engine.start_session(true);
        assert_eq!(engine.stats().rx_bytes, 0);
    }

    #[test]
    fn set_framing_clamps_and_bumps_generation() {
        let engine = Engine::new();
        let g = engine.framing_gen();
        engine.set_framing(FrameSegmentationConfig {
            timeout_ms: 5000,
            ..FrameSegmentationConfig::default()
        });
        assert_eq!(engine.framing().timeout_ms, MAX_FRAME_TIMEOUT_MS);
        assert!(engine.framing_gen() > g);
    }
}

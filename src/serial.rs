use std::io::{ErrorKind, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use serialport::{DataBits, FlowControl, Parity, StopBits};

use crate::bus::Direction;
use crate::engine::Engine;
use crate::framing::FrameSegmenter;

/// Read timeout: bounds how long `stop()` waits for the reader and how late
/// an idle-timeout frame can be emitted.
const READ_TIMEOUT: Duration = Duration::from_millis(50);
/// Write timeout for the cloned write handle (a slow baud rate needs time).
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct PortInfo {
    pub name: String,
    pub description: String,
}

/// Connection-level events; the data itself goes through the engine.
#[derive(Debug)]
pub enum SerialEvent {
    Error(String),
    Disconnected,
}

pub struct SerialHandle {
    pub tx_sender: Sender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    reader_thread: Option<thread::JoinHandle<()>>,
    writer_thread: Option<thread::JoinHandle<()>>,
}

impl SerialHandle {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.reader_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.writer_thread.take() {
            let _ = h.join();
        }
    }
}

impl Drop for SerialHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Available ports, USB devices first, then by name.
pub fn list_ports() -> Vec<PortInfo> {
    let Ok(mut ports) = serialport::available_ports() else {
        return Vec::new();
    };
    ports.sort_by_key(|p| {
        let usb = matches!(p.port_type, serialport::SerialPortType::UsbPort(_));
        (!usb, p.port_name.clone())
    });
    ports
        .into_iter()
        .map(|p| {
            let description = match &p.port_type {
                serialport::SerialPortType::UsbPort(info) => {
                    let mut parts = Vec::new();
                    if let Some(m) = &info.manufacturer {
                        parts.push(m.clone());
                    }
                    if let Some(prod) = &info.product {
                        parts.push(prod.clone());
                    }
                    parts.push(format!("{:04X}:{:04X}", info.vid, info.pid));
                    if let Some(sn) = &info.serial_number {
                        parts.push(format!("SN {sn}"));
                    }
                    parts.join(" - ")
                }
                serialport::SerialPortType::PciPort => "PCI".to_string(),
                serialport::SerialPortType::BluetoothPort => "Bluetooth".to_string(),
                serialport::SerialPortType::Unknown => String::new(),
            };
            PortInfo {
                name: p.port_name,
                description,
            }
        })
        .collect()
}

/// Open `port_name` and start the reader/writer threads. Every received
/// frame and every written chunk is emitted through `engine`.
pub fn spawn_serial(
    port_name: &str,
    baud_rate: u32,
    data_bits: DataBits,
    parity: Parity,
    stop_bits: StopBits,
    engine: Arc<Engine>,
) -> color_eyre::Result<(Receiver<SerialEvent>, SerialHandle)> {
    let port = serialport::new(port_name, baud_rate)
        .data_bits(data_bits)
        .parity(parity)
        .stop_bits(stop_bits)
        .flow_control(FlowControl::None)
        .timeout(READ_TIMEOUT)
        .open()?;

    let mut port_write = port.try_clone()?;
    port_write.set_timeout(WRITE_TIMEOUT)?;

    Ok(spawn_io(port, port_write, engine))
}

/// Thread plumbing, generic over the I/O halves so tests can drive it with
/// scripted fakes instead of a real port. The reader must return
/// `TimedOut`/`WouldBlock`/`Ok(0)` when idle so it can see the stop flag.
pub fn spawn_io<R, W>(
    reader: R,
    writer: W,
    engine: Arc<Engine>,
) -> (Receiver<SerialEvent>, SerialHandle)
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let (event_tx, event_rx) = mpsc::channel();
    let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>();

    let stop = Arc::new(AtomicBool::new(false));

    let reader_thread = {
        let stop = Arc::clone(&stop);
        let engine = Arc::clone(&engine);
        let event_tx = event_tx.clone();
        thread::spawn(move || read_loop(reader, &engine, &stop, &event_tx))
    };

    let writer_thread = {
        let stop = Arc::clone(&stop);
        thread::spawn(move || write_loop(writer, data_rx, &engine, &stop, &event_tx))
    };

    let handle = SerialHandle {
        tx_sender: data_tx,
        stop,
        reader_thread: Some(reader_thread),
        writer_thread: Some(writer_thread),
    };
    (event_rx, handle)
}

fn read_loop<R: Read>(
    mut reader: R,
    engine: &Engine,
    stop: &AtomicBool,
    event_tx: &Sender<SerialEvent>,
) {
    let mut seg = FrameSegmenter::new(engine.framing(), Instant::now());
    let mut framing_gen = engine.framing_gen();
    let mut buf = [0u8; 1024];
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let current_gen = engine.framing_gen();
        if current_gen != framing_gen {
            framing_gen = current_gen;
            seg.set_config(engine.framing());
        }
        match reader.read(&mut buf) {
            Ok(n) if n > 0 => {
                let now = Instant::now();
                // A gap that elapsed while read() was blocked still closes
                // the previous frame before the fresh bytes join the buffer.
                if let Some(frame) = seg.flush_if_idle(now) {
                    engine.emit(Direction::Rx, frame);
                }
                for frame in seg.feed(&buf[..n], now) {
                    engine.emit(Direction::Rx, frame);
                }
            }
            Ok(_) => idle_flush(&mut seg, engine),
            Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                idle_flush(&mut seg, engine)
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => {
                if let Some(frame) = seg.flush() {
                    engine.emit(Direction::Rx, frame);
                }
                let _ = event_tx.send(SerialEvent::Error(e.to_string()));
                let _ = event_tx.send(SerialEvent::Disconnected);
                return;
            }
        }
    }
    // Stopping: don't lose the tail of the stream.
    if let Some(frame) = seg.flush() {
        engine.emit(Direction::Rx, frame);
    }
}

fn idle_flush(seg: &mut FrameSegmenter, engine: &Engine) {
    if let Some(frame) = seg.flush_if_idle(Instant::now()) {
        engine.emit(Direction::Rx, frame);
    }
}

fn write_loop<W: Write>(
    mut writer: W,
    data_rx: Receiver<Vec<u8>>,
    engine: &Engine,
    stop: &AtomicBool,
    event_tx: &Sender<SerialEvent>,
) {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match data_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(data) => match writer.write_all(&data).and_then(|()| writer.flush()) {
                Ok(()) => engine.emit(Direction::Tx, data),
                Err(e) => {
                    let _ = event_tx.send(SerialEvent::Error(format!("write failed: {e}")));
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{BatchPolicy, Frame};
    use crate::framing::{FrameDelimiter, FrameSegmentationConfig, FrameSegmentationMode};
    use std::collections::VecDeque;
    use std::io;
    use std::sync::Mutex;

    /// One scripted reader step.
    enum Step {
        Data(&'static [u8]),
        /// Report an idle read (TimedOut) after sleeping this long.
        Idle(u64),
        Fail,
    }

    /// Plays its script, then reports idle reads forever.
    struct ScriptedReader(VecDeque<Step>);

    impl Read for ScriptedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.0.pop_front() {
                Some(Step::Data(d)) => {
                    buf[..d.len()].copy_from_slice(d);
                    Ok(d.len())
                }
                Some(Step::Idle(ms)) => {
                    thread::sleep(Duration::from_millis(ms));
                    Err(io::Error::from(ErrorKind::TimedOut))
                }
                Some(Step::Fail) => Err(io::Error::new(ErrorKind::BrokenPipe, "unplugged")),
                None => {
                    thread::sleep(Duration::from_millis(5));
                    Err(io::Error::from(ErrorKind::TimedOut))
                }
            }
        }
    }

    #[derive(Clone, Default)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn engine(mode: FrameSegmentationMode) -> Arc<Engine> {
        let engine = Arc::new(Engine::new());
        engine.set_framing(FrameSegmentationConfig {
            mode,
            timeout_ms: 10,
            delimiter: FrameDelimiter::AnyNewline,
        });
        engine.start_session(true);
        engine
    }

    fn policy() -> BatchPolicy {
        BatchPolicy {
            max_delay: Duration::from_millis(5),
            min_interval: Duration::from_millis(0),
            max_bytes: 1 << 20,
            max_frames: 1024,
            queue_frames: 1024,
        }
    }

    /// Collect frames until `n` arrived or a second passed.
    fn collect(sub: &crate::bus::Subscription, n: usize) -> Vec<Frame> {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut frames = Vec::new();
        while frames.len() < n && Instant::now() < deadline {
            if let Some(batch) = sub.recv_batch() {
                frames.extend(batch.frames);
            }
        }
        frames
    }

    fn payloads(frames: &[Frame]) -> Vec<&[u8]> {
        frames.iter().map(|f| &f.data[..]).collect()
    }

    #[test]
    fn combined_mode_frames_lines_and_flushes_tail_on_idle() {
        let engine = engine(FrameSegmentationMode::Combined);
        let sub = engine.subscribe(policy());
        let reader = ScriptedReader(VecDeque::from([
            Step::Data(b"AT\r\nO"),
            Step::Data(b"K\r\n> "),
            Step::Idle(20),
        ]));
        let (_events, mut handle) = spawn_io(reader, SharedWriter::default(), engine.clone());
        let frames = collect(&sub, 3);
        handle.stop();
        assert_eq!(
            payloads(&frames),
            vec![&b"AT\r\n"[..], &b"OK\r\n"[..], &b"> "[..]]
        );
        assert!(frames.iter().all(|f| f.dir == Direction::Rx));
        assert_eq!(engine.stats().rx_bytes, 10);
    }

    #[test]
    fn timeout_mode_gap_during_blocking_read_cuts_frame() {
        let engine = engine(FrameSegmentationMode::Timeout);
        let sub = engine.subscribe(policy());
        // No idle read in between: the gap is only visible as elapsed time
        // when the next data arrives, which must still close frame one.
        struct Slow(VecDeque<&'static [u8]>);
        impl Read for Slow {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                thread::sleep(Duration::from_millis(20));
                match self.0.pop_front() {
                    Some(d) => {
                        buf[..d.len()].copy_from_slice(d);
                        Ok(d.len())
                    }
                    None => Err(io::Error::from(ErrorKind::TimedOut)),
                }
            }
        }
        let reader = Slow(VecDeque::from([&b"one"[..], &b"two"[..]]));
        let (_events, mut handle) = spawn_io(reader, SharedWriter::default(), engine.clone());
        let frames = collect(&sub, 2);
        handle.stop();
        assert_eq!(payloads(&frames), vec![&b"one"[..], &b"two"[..]]);
    }

    #[test]
    fn writes_are_emitted_as_tx_in_shared_sequence() {
        let engine = engine(FrameSegmentationMode::Combined);
        let sub = engine.subscribe(policy());
        let written = SharedWriter::default();
        let reader = ScriptedReader(VecDeque::new());
        let (_events, mut handle) = spawn_io(reader, written.clone(), engine.clone());
        handle.tx_sender.send(b"AT\r\n".to_vec()).unwrap();
        let frames = collect(&sub, 1);
        handle.stop();
        assert_eq!(&*written.0.lock().unwrap(), b"AT\r\n");
        assert_eq!(frames.len(), 1);
        assert_eq!((frames[0].dir, frames[0].seq), (Direction::Tx, 1));
        assert_eq!(engine.stats().tx_bytes, 4);
    }

    #[test]
    fn read_error_flushes_partial_frame_and_reports_disconnect() {
        let engine = engine(FrameSegmentationMode::Combined);
        let sub = engine.subscribe(policy());
        let reader = ScriptedReader(VecDeque::from([Step::Data(b"partial"), Step::Fail]));
        let (events, mut handle) = spawn_io(reader, SharedWriter::default(), engine.clone());
        let frames = collect(&sub, 1);
        assert_eq!(payloads(&frames), vec![&b"partial"[..]]);
        let ev: Vec<SerialEvent> = (0..2)
            .map(|_| events.recv_timeout(Duration::from_secs(1)).unwrap())
            .collect();
        assert!(matches!(ev[0], SerialEvent::Error(ref m) if m.contains("unplugged")));
        assert!(matches!(ev[1], SerialEvent::Disconnected));
        handle.stop();
    }

    #[test]
    fn stop_flushes_pending_bytes() {
        let engine = engine(FrameSegmentationMode::Combined);
        // A long timeout, so only stop() can emit the pending bytes.
        engine.set_framing(FrameSegmentationConfig {
            timeout_ms: 1000,
            ..engine.framing()
        });
        let reader = ScriptedReader(VecDeque::from([Step::Data(b"no newline")]));
        let (_events, mut handle) = spawn_io(reader, SharedWriter::default(), engine.clone());
        thread::sleep(Duration::from_millis(30));
        assert!(engine.snapshot().entries.is_empty());
        handle.stop();
        let snap = engine.snapshot();
        assert_eq!(snap.entries.len(), 1);
        assert_eq!(&snap.entries[0].data[..], b"no newline");
    }

    /// Real-port round trip over a virtual pair. Run with:
    /// `socat -d -d pty,raw,echo=0,link=/tmp/rserial_a pty,raw,echo=0,link=/tmp/rserial_b &`
    /// `RSERIAL_TEST_PORTS=/tmp/rserial_a,/tmp/rserial_b cargo test -- --ignored real_port`
    #[test]
    #[ignore]
    fn real_port_roundtrip() {
        let ports = std::env::var("RSERIAL_TEST_PORTS").expect("RSERIAL_TEST_PORTS=a,b");
        let (a, b) = ports.split_once(',').expect("two ports");
        let engine = engine(FrameSegmentationMode::Combined);
        let sub = engine.subscribe(policy());
        let (_events, mut handle) = spawn_serial(
            a,
            115_200,
            DataBits::Eight,
            Parity::None,
            StopBits::One,
            engine.clone(),
        )
        .unwrap();
        let mut peer = serialport::new(b, 115_200)
            .timeout(Duration::from_secs(1))
            .open()
            .unwrap();

        peer.write_all(b"hello\r\nwor").unwrap();
        thread::sleep(Duration::from_millis(5));
        peer.write_all(b"ld\r\n").unwrap();
        handle.tx_sender.send(b"ping\n".to_vec()).unwrap();

        let frames = collect(&sub, 3);
        let mut echoed = [0u8; 5];
        peer.read_exact(&mut echoed).unwrap();
        handle.stop();

        assert_eq!(&echoed, b"ping\n");
        let rx: Vec<&[u8]> = frames
            .iter()
            .filter(|f| f.dir == Direction::Rx)
            .map(|f| &f.data[..])
            .collect();
        assert_eq!(rx, vec![&b"hello\r\n"[..], &b"world\r\n"[..]]);
        assert!(frames.iter().any(|f| f.dir == Direction::Tx));
    }
}

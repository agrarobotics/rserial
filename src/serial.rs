use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::Duration;

use serialport::{DataBits, FlowControl, Parity, StopBits};

#[derive(Debug, Clone)]
pub struct PortInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug)]
pub enum SerialEvent {
    Data(Vec<u8>),
    Error(String),
    Disconnected,
}

pub struct SerialHandle {
    pub tx_sender: mpsc::Sender<Vec<u8>>,
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

pub fn list_ports() -> Vec<PortInfo> {
    match serialport::available_ports() {
        Ok(ports) => ports
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
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub fn spawn_serial_reader(
    port_name: &str,
    baud_rate: u32,
    data_bits: DataBits,
    parity: Parity,
    stop_bits: StopBits,
) -> color_eyre::Result<(Receiver<SerialEvent>, SerialHandle)> {
    let port = serialport::new(port_name, baud_rate)
        .data_bits(data_bits)
        .parity(parity)
        .stop_bits(stop_bits)
        .flow_control(FlowControl::None)
        .timeout(Duration::from_millis(100))
        .open()?;

    let port_write = port.try_clone()?;

    let (event_tx, event_rx): (SyncSender<SerialEvent>, Receiver<SerialEvent>) =
        mpsc::sync_channel(256);
    let (data_tx, data_rx): (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) = mpsc::channel();

    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = Arc::clone(&stop);
    let stop_writer = Arc::clone(&stop);

    let reader_thread = thread::spawn(move || {
        let mut port = port;
        let mut buf = [0u8; 1024];
        loop {
            if stop_reader.load(Ordering::Relaxed) {
                break;
            }
            match port.read(&mut buf) {
                Ok(n) if n > 0 => {
                    if event_tx.send(SerialEvent::Data(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => {
                    let _ = event_tx.send(SerialEvent::Error(e.to_string()));
                    let _ = event_tx.send(SerialEvent::Disconnected);
                    break;
                }
            }
        }
    });

    let writer_thread = thread::spawn(move || {
        let mut port = port_write;
        loop {
            if stop_writer.load(Ordering::Relaxed) {
                break;
            }
            match data_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(data) => {
                    if let Err(e) = std::io::Write::write_all(&mut port, &data) {
                        eprintln!("Serial write error: {e}");
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    let handle = SerialHandle {
        tx_sender: data_tx,
        stop,
        reader_thread: Some(reader_thread),
        writer_thread: Some(writer_thread),
    };

    Ok((event_rx, handle))
}

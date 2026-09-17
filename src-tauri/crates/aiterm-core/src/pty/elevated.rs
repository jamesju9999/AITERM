use std::io::{self, Read, Write};
use std::sync::Arc;

use parking_lot::Mutex;

/// 提權 channel 目前的連線狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevatedState {
    Connected,
    Disconnected,
}

/// 一個已建立的提權 channel：對外只暴露「寫入鍵盤輸入」跟「目前狀態」，不
/// 關心底層是具名管線還是別的傳輸方式——真正的具名管線/ConPTY 啟動邏輯在
/// `#[cfg(windows)]` 的 `spawn_windows` 裡（後續任務補上），這裡只放狀態機
/// 本身，好讓它能在任何平台上被測試。
pub struct ElevatedChannel {
    writer: Mutex<Box<dyn Write + Send>>,
    state: Arc<Mutex<ElevatedState>>,
}

impl ElevatedChannel {
    /// 給測試與之後的 Windows 啟動邏輯共用的建構子：呼叫端已經備妥一個雙向的
    /// `Read + Write` 傳輸（真正的具名管線，或測試用的記憶體管道），這裡只
    /// 負責包成 channel 並起一條讀取執行緒。
    ///
    /// `on_output`：讀到 `Frame::Data` 時呼叫，把位元組交回去給呼叫端（正式
    /// 環境會接到 session 的 output ring buffer + Tauri 事件）。
    /// `on_disconnect`：讀取端遇到 EOF 或 `Frame::Exit` 時呼叫一次。
    pub fn new<T, F, D>(mut transport_reader: T, transport_writer: Box<dyn Write + Send>, mut on_output: F, mut on_disconnect: D) -> Self
    where
        T: Read + Send + 'static,
        F: FnMut(Vec<u8>) + Send + 'static,
        D: FnMut() + Send + 'static,
    {
        let state = Arc::new(Mutex::new(ElevatedState::Connected));
        let state_for_thread = Arc::clone(&state);

        std::thread::spawn(move || {
            loop {
                match super::elevated_protocol::Frame::read_from(&mut transport_reader) {
                    Ok(Some(super::elevated_protocol::Frame::Data(bytes))) => on_output(bytes),
                    Ok(Some(super::elevated_protocol::Frame::Resize { .. })) => {
                        // Resize frames flow the other direction (host -> sidecar);
                        // seeing one here would mean the sidecar echoed it back,
                        // which is not part of the protocol. Ignore defensively.
                    }
                    Ok(Some(super::elevated_protocol::Frame::Exit)) | Ok(None) => break,
                    Err(_) => break,
                }
            }
            *state_for_thread.lock() = ElevatedState::Disconnected;
            on_disconnect();
        });

        Self { writer: Mutex::new(transport_writer), state }
    }

    pub fn state(&self) -> ElevatedState {
        *self.state.lock()
    }

    /// 寫入鍵盤輸入。channel 已斷線時回錯，呼叫端（`PtySession::write`，之後
    /// 的任務會接上）據此判斷要不要退回一般子行程。
    pub fn write(&self, data: &[u8]) -> io::Result<()> {
        if self.state() == ElevatedState::Disconnected {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "elevated channel disconnected"));
        }
        let frame = super::elevated_protocol::Frame::Data(data.to_vec());
        let mut writer = self.writer.lock();
        frame.write_to(&mut *writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::mpsc;
    use std::time::Duration;

    /// 一個假的雙向傳輸：讀端從固定的 byte 序列讀，寫端丟進一個
    /// `mpsc::Sender` 給測試檢查。
    struct FakeWriter(mpsc::Sender<Vec<u8>>);
    impl Write for FakeWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let _ = self.0.send(buf.to_vec());
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn disconnect_callback_fires_when_transport_hits_eof() {
        let (disc_tx, disc_rx) = mpsc::channel::<()>();
        let reader = Cursor::new(Vec::<u8>::new()); // 立刻 EOF
        let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>();
        let writer = Box::new(FakeWriter(write_tx));

        let channel = ElevatedChannel::new(
            reader,
            writer,
            |_bytes| {},
            move || {
                let _ = disc_tx.send(());
            },
        );

        disc_rx.recv_timeout(Duration::from_secs(2)).expect("disconnect callback must fire on EOF");
        assert_eq!(channel.state(), ElevatedState::Disconnected);
    }

    #[test]
    fn write_after_disconnect_errors_instead_of_silently_dropping() {
        let (disc_tx, disc_rx) = mpsc::channel::<()>();
        let reader = Cursor::new(Vec::<u8>::new());
        let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>();
        let writer = Box::new(FakeWriter(write_tx));

        let channel = ElevatedChannel::new(reader, writer, |_| {}, move || {
            let _ = disc_tx.send(());
        });
        disc_rx.recv_timeout(Duration::from_secs(2)).expect("must disconnect");

        let err = channel.write(b"echo hi").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    }

    #[test]
    fn output_callback_receives_data_frames_in_order() {
        let mut encoded = Vec::new();
        super::super::elevated_protocol::Frame::Data(b"first".to_vec()).write_to(&mut encoded).unwrap();
        super::super::elevated_protocol::Frame::Data(b"second".to_vec()).write_to(&mut encoded).unwrap();
        let reader = Cursor::new(encoded);
        let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>();
        let writer = Box::new(FakeWriter(write_tx));

        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
        let _channel = ElevatedChannel::new(
            reader,
            writer,
            move |bytes| {
                let _ = out_tx.send(bytes);
            },
            || {},
        );

        assert_eq!(out_rx.recv_timeout(Duration::from_secs(2)).unwrap(), b"first".to_vec());
        assert_eq!(out_rx.recv_timeout(Duration::from_secs(2)).unwrap(), b"second".to_vec());
    }
}

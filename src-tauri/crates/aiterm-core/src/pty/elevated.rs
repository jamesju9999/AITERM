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
///
/// 已知缺口（刻意先留著）：讀取執行緒（見 `new` 裡的 `std::thread::spawn`）
/// 目前沒有 join/清理路徑。如果 `ElevatedChannel` 在執行緒還卡在
/// `Frame::read_from` 阻塞讀取時被 drop，執行緒會一直活著直到底層傳輸自己
/// 出錯或 EOF 才會退出——不會馬上跟著 channel 一起結束。這在 mock transport
/// 的測試裡不是問題（`Cursor` 立刻 EOF），但等到實作 `spawn_windows`（真正的
/// 具名管線）那個任務時必須回頭處理：屆時傳輸是長生命週期的具名管線，同樣
/// 的阻塞讀取沒有清理路徑就會變成真的執行緒洩漏。實作 `spawn_windows` 的人
/// 請在那裡補上 `Drop`／join 或等效機制，不要延到之後才發現。
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

#[cfg(windows)]
mod windows_launch {
    use super::ElevatedChannel;
    use std::ffi::CString;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_CANCELLED, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeA, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    /// 主行程呼叫這個函式來啟動一整套提權流程：先建具名管線 server，再用
    /// `ShellExecuteExW(runas)` 拉起 sidecar，等它連上來。回傳一個已連線的
    /// `ElevatedChannel`，或者使用者在 UAC 對話框按了取消（`ERROR_CANCELLED`）
    /// 時回傳 `None`——這不是錯誤，是使用者的正常選擇。
    ///
    /// 對照 `windows-sys` 0.60.2 實際原始碼（而非憑印象猜）修正過草稿版本的
    /// 兩個會導致編譯失敗或直接壞掉的問題：
    /// 1. `PIPE_ACCESS_DUPLEX` 實際定義在 `Win32::Storage::FileSystem`，不是
    ///    `Win32::System::Pipes`（因為 `CreateNamedPipeA` 的 `dwOpenMode`
    ///    參數型別是 `FILE_FLAGS_AND_ATTRIBUTES`，跟著那個型別走）。
    /// 2. 草稿版本寫 `info.lpVerb = widen(&verb).as_ptr()`：`widen(...)` 回傳
    ///    的 `Vec<u16>` 是臨時值，這個陳述式結束就被 drop，`lpVerb` 會是懸空
    ///    指標——`ShellExecuteExW` 讀到的是已釋放記憶體。改成先把三個
    ///    `Vec<u16>` 綁到具名區域變數（`verb_w`/`file_w`/`params_w`），讓它們
    ///    活到呼叫結束。
    /// 另外也補了草稿版本沒處理的三個資源/正確性缺口：`SHELLEXECUTEINFOW`
    /// 這個型別本身在 windows-sys 0.60.2 是被 `Win32_System_Registry`
    /// feature 卡住的（因為裡面有 `hkeyClass: HKEY` 欄位）、`SEE_MASK_NOCLOSEPROCESS`
    /// 給回來的 `info.hProcess` 沒人關會洩漏 handle、`ConnectNamedPipe` 在
    /// `CreateNamedPipeA` 和 `ConnectNamedPipe` 中間如果剛好被搶著連上會回報
    /// `ERROR_PIPE_CONNECTED`（那其實是「已經連上」不是錯誤，MSDN 文件明載的
    /// 已知競態，不處理會把正常路徑誤判成失敗）。
    ///
    /// **人工驗證待辦（無法在 mac 上跑，見檔案結尾的清單）**：
    /// - `sidecar_exe_path` 目前假設 Tauri 打包後 `externalBin` 會把
    ///   `binaries/aiterm-elevated-host-x86_64-pc-windows-msvc.exe` 放在跟主
    ///   執行檔同一個目錄，實際路徑要在真機上用
    ///   `tauri::Env`/`current_exe()` 確認，這裡先用 `current_exe()` 所在目錄
    ///   推算，若跟 Tauri 實際打包結構不符要修正。
    /// - `ShellExecuteExW` 回傳成功不代表 sidecar 真的連得上管線（例如管線
    ///   名稱打錯、或防毒軟體攔截）——`ConnectNamedPipe` 目前是同步阻塞呼叫
    ///   （沒有用 `FILE_FLAG_OVERLAPPED` 開管線），如果 sidecar 啟動失敗，這
    ///   個呼叫會無限期卡住，沒有逾時機制。
    ///
    /// **呼叫端注意：這個函式可能無限期阻塞呼叫它的執行緒**，而且是兩個獨立
    /// 的阻塞點疊加：`ShellExecuteExW(runas)` 會一路擋到使用者回應 UAC 對話
    /// 框為止（可能是好幾分鐘，甚至使用者晾在那邊不理），接著
    /// `ConnectNamedPipe` 上面說的沒有逾時機制。**絕對不要直接從 async 執行
    /// 時的 task 或 UI 執行緒呼叫**——之後接上 `PtyManager`/Tauri command（下
    /// 一個任務）時，要包一層 `spawn_blocking`（或專門開一條 thread）來呼叫
    /// 這個函式，不能讓它卡住 async runtime 的 worker 執行緒或 UI 事件迴圈。
    pub fn spawn_windows(
        session_id: &str,
        shell_variant: super::super::cd_parser::ShellVariant,
        on_output: impl FnMut(Vec<u8>) + Send + 'static,
        on_disconnect: impl FnMut() + Send + 'static,
    ) -> std::io::Result<Option<ElevatedChannel>> {
        let pipe_name = format!(r"\\.\pipe\aiterm-elevate-{session_id}-{}", uuid::Uuid::new_v4());
        // `session_id` 是呼叫端傳進來的字串，理論上可能含有內嵌 NUL（Rust
        // `String` 允許），`CString::new` 遇到會回 `Err`——用 `?` 往上丟成
        // `io::Result`，不要 `.unwrap()` panic 掉整個主行程。
        let c_pipe_name = CString::new(pipe_name.clone()).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        let pipe_handle: HANDLE = unsafe {
            CreateNamedPipeA(
                c_pipe_name.as_ptr() as *const u8,
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                65536,
                65536,
                0,
                std::ptr::null(),
            )
        };
        if pipe_handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }

        let sidecar_path = sidecar_exe_path()?;
        let variant_arg = match shell_variant {
            super::super::cd_parser::ShellVariant::Pwsh => "pwsh",
            _ => "cmd",
        };
        let params = format!("\"{pipe_name}\" {variant_arg}");

        // ShellExecuteExW 的 *W 欄位要的是 null-terminated UTF-16
        // （`PCWSTR` = `*const u16`），跟上面具名管線用的 ANSI `CString`
        // 不是同一種格式。這三個 `Vec<u16>` 必須活到 `ShellExecuteExW` 呼叫
        // 結束——見上面 doc comment 說明的懸空指標問題。
        let verb_w = widen("runas");
        let file_w = widen(&sidecar_path.to_string_lossy());
        let params_w = widen(&params);

        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb_w.as_ptr();
        info.lpFile = file_w.as_ptr();
        info.lpParameters = params_w.as_ptr();
        info.nShow = SW_HIDE as i32;

        let ok = unsafe { ShellExecuteExW(&mut info) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            unsafe { CloseHandle(pipe_handle) };
            if err == ERROR_CANCELLED {
                return Ok(None); // 使用者取消 UAC，不是錯誤。
            }
            return Err(std::io::Error::from_raw_os_error(err as i32));
        }
        // `SEE_MASK_NOCLOSEPROCESS` 會讓 `ShellExecuteExW` 把子行程的
        // process HANDLE 塞進 `info.hProcess` 交給我們保管。這裡不追蹤
        // sidecar 行程本身的生死（斷線偵測走的是下面的具名管線／Frame
        // 協定，見 `ElevatedChannel`），所以立刻關掉這個 handle 避免洩漏
        // ——關 handle 不會終止或影響行程本身，只是釋放我們手上的參照。
        if !info.hProcess.is_null() {
            unsafe { CloseHandle(info.hProcess) };
        }

        let connected = unsafe { ConnectNamedPipe(pipe_handle, std::ptr::null_mut()) };
        if connected == 0 {
            let err = unsafe { GetLastError() };
            // `CreateNamedPipeA` 到 `ConnectNamedPipe` 中間如果 client 剛好
            // 搶著連上，`ConnectNamedPipe` 會回傳失敗但 `GetLastError()` 是
            // `ERROR_PIPE_CONNECTED`——這是 MSDN 文件記載的正常競態，意思是
            // 「已經連上了」，不是真的錯誤，不能當失敗處理。
            if err != ERROR_PIPE_CONNECTED {
                unsafe { CloseHandle(pipe_handle) };
                return Err(std::io::Error::from_raw_os_error(err as i32));
            }
        }

        // 已知缺口（刻意先留著，跟 `ElevatedChannel` 本身文件說明的讀取執行緒
        // 洩漏是同一類問題）：`pipe_handle` 的所有權從這裡轉移進
        // `PipeReadHandle`/`PipeWriteHandle`，但兩者都沒有 `Drop` 實作去關閉
        // 它——`ElevatedChannel` 被 drop 時，這個具名管線 handle 不會自動關閉。
        // 目前只有讀取執行緒自然 EOF／出錯時才會間接讓行程之後收尾；如果之後
        // 要處理提前關閉 session 的路徑，這裡要回頭補上清理機制。
        //
        // 這個「回頭補」不是隨手加個 `Drop` 就能解決的一行修法：
        // `PipeReadHandle` 跟 `PipeWriteHandle` 兩個各自獨立的型別包著同一個
        // 原始 `HANDLE` 值，彼此完全不知道對方的存在，也沒有共享的所有權／
        // 參照計數。如果直接在任一個型別上加 `impl Drop { CloseHandle(self.0) }`，
        // 其中一個被 drop 時就會把 handle 關掉，而另一個當下可能還活著、還在
        // 另一條執行緒上用同一個 handle（寫入路徑，或讀取執行緒可能還卡在
        // `ReadFile` 阻塞讀取）——那會是 use-after-close，不是單純的資源洩漏，
        // 而是新的正確性 bug。之後要修的話，正確作法是讓兩者共享同一份「關
        // 一次」的所有權，例如包成 `Arc<HandleGuard>`（`HandleGuard` 自己的
        // `Drop` 才真正呼叫 `CloseHandle`），把同一個 `Arc` clone 進
        // `PipeReadHandle`/`PipeWriteHandle` 各一份，讓 handle 在兩邊都不再
        // 使用（`Arc` 參照數歸零）時才真正關閉一次——不是各自獨立的 `Drop`。
        let reader = super::PipeReadHandle(pipe_handle);
        let writer: Box<dyn std::io::Write + Send> = Box::new(super::PipeWriteHandle(pipe_handle));
        Ok(Some(ElevatedChannel::new(reader, writer, on_output, on_disconnect)))
    }

    fn sidecar_exe_path() -> std::io::Result<std::path::PathBuf> {
        let exe = std::env::current_exe()?;
        let dir = exe.parent().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no parent dir"))?;
        Ok(dir.join("aiterm-elevated-host.exe"))
    }

    /// null-terminated UTF-16，`ShellExecuteExW` 的 `PCWSTR` 欄位要的格式。
    fn widen(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
pub use windows_launch::spawn_windows;

#[cfg(windows)]
pub(crate) struct PipeReadHandle(pub windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for PipeReadHandle {}
#[cfg(windows)]
impl std::io::Read for PipeReadHandle {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use windows_sys::Win32::Storage::FileSystem::ReadFile;
        let mut n = 0u32;
        let ok = unsafe { ReadFile(self.0, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut()) };
        if ok == 0 { return Err(std::io::Error::last_os_error()); }
        Ok(n as usize)
    }
}

#[cfg(windows)]
pub(crate) struct PipeWriteHandle(pub windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for PipeWriteHandle {}
#[cfg(windows)]
impl std::io::Write for PipeWriteHandle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use windows_sys::Win32::Storage::FileSystem::WriteFile;
        let mut n = 0u32;
        let ok = unsafe { WriteFile(self.0, buf.as_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut()) };
        if ok == 0 { return Err(std::io::Error::last_os_error()); }
        Ok(n as usize)
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

// 這個結構跟 `aiterm-elevated-host/src/windows_host.rs` 的 `PipeHandle`
// 本質上是同一個包裝手法（HANDLE -> Read/Write），故意沒有共用同一個型別：
// 兩邊在不同的 crate（`aiterm-core` vs `aiterm-elevated-host`），
// `aiterm-elevated-host` 已經依賴 `aiterm-core`，反過來讓 `aiterm-core`
// 依賴 sidecar crate 會造成循環依賴；而且這裡刻意拆成 `PipeReadHandle`/
// `PipeWriteHandle` 兩個型別（而不是像對面一樣一個型別身兼二職），是因為
// `ElevatedChannel::new` 要求 reader 是 `T: Read + Send + 'static`（依值移
// 交給內部的讀取執行緒）、writer 是另一個 `Box<dyn Write + Send>`（留在呼叫
// 端手上給 `write()` 用），兩者生命週期與所有權路徑不同，硬塞同一個型別會
// 需要額外包一層 `Arc`/`Clone` 才能讓「同一個 HANDLE 值」分別放進兩個角色
// ——不如直接拆成兩個零開銷的 newtype 包同一個 raw HANDLE 值來得直接。

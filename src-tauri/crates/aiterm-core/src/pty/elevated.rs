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
/// 關閉的連鎖反應（Windows）：channel 被 drop → writer（`PipeWriteHandle`）
/// 的 `Drop` 關掉主行程 -> sidecar 那條管線 → sidecar 的讀取收到
/// `ERROR_BROKEN_PIPE`、整個行程結束（連帶 Job 裡的提權 shell）→ sidecar ->
/// 主行程那條管線跟著斷 → 這裡的讀取執行緒出錯退出。讀取執行緒因此不需要另外
/// join：它的生命週期跟 sidecar 綁在一起，而 sidecar 的生命週期由 writer 決定。
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

    /// 把終端機尺寸變化轉發給提權 ConPTY。
    ///
    /// 沒有這條路的時候，提權 ConPTY 會永遠停在 sidecar 啟動時的尺寸，而
    /// xterm 用的是真實視窗尺寸——兩邊的換行位置與游標定位模型不一致，
    /// ConPTY 送出的重繪序列在 xterm 上就會錯位，實機表現是同一行指令重複
    /// 出現好幾次、一次比一次短、還帶大段前導空白。
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        if self.state() == ElevatedState::Disconnected {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "elevated channel disconnected"));
        }
        let frame = super::elevated_protocol::Frame::Resize { cols, rows };
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
    use windows_sys::Win32::Storage::FileSystem::{PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND};
    use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeA, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
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
    /// 對照 `aiterm-elevated-host` 那邊 `windows_host.rs::log_step` 的同一招：
    /// 主行程這邊也不知道自己卡在哪一步（`spawn_blocking` 背景執行緒沒有主
    /// 控台，`eprintln!` 一樣沒人看得到），寫進另一個檔案（跟 sidecar 的
    /// log 分開，避免兩邊寫入互相干擾／檔名混淆），下次重現時兩份 log 的
    /// 時間戳可以直接對照，看主行程這邊到底有沒有跟著卡住、卡在哪個環節。
    pub(crate) fn log_step(msg: &str) {
        use std::io::Write as _;
        let Some(mut path) = std::env::var_os("TEMP").map(std::path::PathBuf::from) else { return };
        path.push("aiterm-elevate-main.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let _ = writeln!(f, "[{now}] pid={} {msg}", std::process::id());
            let _ = f.flush();
        }
    }

    pub fn spawn_windows(
        session_id: &str,
        shell_variant: super::super::cd_parser::ShellVariant,
        size: (u16, u16),
        on_output: impl FnMut(Vec<u8>) + Send + 'static,
        on_disconnect: impl FnMut() + Send + 'static,
    ) -> std::io::Result<Option<ElevatedChannel>> {
        log_step("spawn_windows: start");
        // **兩條單向管線，不是一條雙向管線**——這是實機抓了四輪才定位到的死鎖
        // 真因。原本的寫法是開一條 `PIPE_ACCESS_DUPLEX` 的同步（沒有
        // `FILE_FLAG_OVERLAPPED`）管線，然後把同一個 raw `HANDLE` 值同時包進
        // `PipeReadHandle` 跟 `PipeWriteHandle` 兩邊用。Windows 對**同步** file
        // object 會把 I/O 序列化（見 MSDN "Synchronous and Overlapped Input and
        // Output"）：讀取執行緒卡在 `ReadFile` 等 sidecar 的輸出時（提權 shell
        // 剛啟動、還沒吐任何東西，這完全正常），另一條執行緒對**同一個 handle**
        // 發的 `WriteFile` 會排在那個讀取後面一起卡死。而送鍵盤輸入那條路
        // （`pty_write` 是同步 `#[tauri::command]`，跑在主執行緒上）正好就是
        // 這個 `WriteFile`——於是整個 UI 執行緒跟著永久卡住，直到 sidecar 行程
        // 被強制關閉、管線斷掉讓 `ReadFile` 回錯為止。實測現象每一項都對得上。
        //
        // 拆成兩條各自單向的管線之後，每個 handle 只會被用在單一方向，序列化
        // 就完全不可能發生。另一個選項是把管線改成 overlapped I/O，但那要連兩
        // 邊每一個 `ReadFile`/`WriteFile` 都改寫成 `OVERLAPPED` + 事件 +
        // `GetOverlappedResult`，改動面大很多，而且本機沒有編譯器能驗證。
        let pipe_base = format!(r"\\.\pipe\aiterm-elevate-{session_id}-{}", uuid::Uuid::new_v4());
        let h2s_name = format!("{pipe_base}-h2s"); // 主行程 -> sidecar（主行程只寫）
        let s2h_name = format!("{pipe_base}-s2h"); // sidecar -> 主行程（主行程只讀）

        // `session_id` 是呼叫端傳進來的字串，理論上可能含有內嵌 NUL（Rust
        // `String` 允許），`CString::new` 遇到會回 `Err`——用 `?` 往上丟成
        // `io::Result`，不要 `.unwrap()` panic 掉整個主行程。
        let h2s_handle = create_pipe(&h2s_name, PIPE_ACCESS_OUTBOUND)?;
        let s2h_handle = match create_pipe(&s2h_name, PIPE_ACCESS_INBOUND) {
            Ok(h) => h,
            Err(e) => {
                unsafe { CloseHandle(h2s_handle) };
                return Err(e);
            }
        };

        let sidecar_path = sidecar_exe_path()?;
        let variant_arg = match shell_variant {
            super::super::cd_parser::ShellVariant::Pwsh => "pwsh",
            _ => "cmd",
        };
        // 尺寸一起傳過去：sidecar 以前寫死 80x24 開 ConPTY，跟 xterm 的真實
        // 尺寸對不上，畫面重繪會錯位（見 `ElevatedChannel::resize`）。這裡傳
        // 的是「啟動當下」的尺寸；之後視窗再變大變小，走 `resize()` 送
        // `Frame::Resize`。
        let (cols, rows) = size;
        let params = format!("\"{h2s_name}\" \"{s2h_name}\" {variant_arg} {cols} {rows}");

        // **已撤回的假設，留紀錄避免重踩**：曾在這裡加過
        // `CoInitializeEx(COINIT_APARTMENTTHREADED)`，理論是 `ShellExecuteExW`
        // 需要呼叫端先初始化 COM，否則會隱含建立一個需要訊息幫浦的 STA、卡死
        // 呼叫的執行緒。實機用工作管理員的「分析等待鏈」驗證後發現：加了這段
        // 之後，卡住的不再是「無限期等待」，而是 Windows 明確判定的**鎖死狀
        // 態**（同一個執行緒 `app.exe` 出現在自己的等待鏈裡）——最可能的解釋
        // 是這條 `spawn_blocking` 背景執行緒建立 STA 後從未幫浦訊息，而
        // `spawn_windows` 回傳之後、同一條執行緒後續呼叫的 `app.emit(...)`
        // （`elevate_with_app`，Tauri/WebView2 在 Windows 上內部會用到 COM）
        // 剛好需要用到那個從未被服務過的 STA 佇列，反而把原本的問題換成更嚴
        // 重的自我死鎖。已經整段移除；`ShellExecuteExW` 不顯式初始化 COM 時
        // 的預設行為（隱含建立、用完即釋放的 STA）看起來才是安全的路徑。
        //
        // ShellExecuteExW 的 *W 欄位要的是 null-terminated UTF-16
        // （`PCWSTR` = `*const u16`），跟上面具名管線用的 ANSI `CString`
        // 不是同一種格式。這三個 `Vec<u16>` 必須活到 `ShellExecuteExW` 呼叫
        // 結束——見上面 doc comment 說明的懸空指標問題。
        let verb_w = widen("runas");
        let file_w = widen(&sidecar_path.to_string_lossy());
        let params_w = widen(&params);

        // `SEE_MASK_NOASYNC`：MSDN 對 `SHELLEXECUTEINFO.fMask` 的文件明載
        // ——「呼叫 `ShellExecuteEx` 的執行緒沒有 message loop 時，必須指定
        // 這個旗標」。我們的呼叫端是 `tokio::task::spawn_blocking` 開出來的
        // 純運算執行緒，從來沒有 Win32 message loop（`GetMessage`/
        // `DispatchMessage`）。沒有這個旗標時，`ShellExecuteEx` 對某些委派執
        // 行路徑（例如 DDE 完成通知）會採用需要訊息幫浦才能收到「完成」信號
        // 的非同步機制——呼叫執行緒沒有在幫浦訊息，這個內部完成信號永遠送不
        // 到，而这個機制在 shell32 內部很可能不是每個呼叫獨立、而是 process
        // 共用的（隱藏視窗／STA），卡住的不只是這條呼叫執行緒本身，還可能波
        // 及其他共用同一個 shell32 內部機制的操作。這正好吻合實測現象：
        // `spawn_windows` 本身（含 `ConnectNamedPipe`）很快就回傳成功，但主
        // 視窗仍持續無回應，直到 sidecar 行程被強制關閉才恢復——加上這個旗
        // 標讓 `ShellExecuteEx` 改用同步等待完成的路徑，不依賴訊息幫浦。
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        info.lpVerb = verb_w.as_ptr();
        info.lpFile = file_w.as_ptr();
        info.lpParameters = params_w.as_ptr();
        info.nShow = SW_HIDE as i32;

        log_step("calling ShellExecuteExW(runas)");
        let ok = unsafe { ShellExecuteExW(&mut info) };
        log_step(&format!("ShellExecuteExW returned, ok={ok}"));
        if ok == 0 {
            let err = unsafe { GetLastError() };
            unsafe { CloseHandle(h2s_handle) };
            unsafe { CloseHandle(s2h_handle) };
            log_step(&format!("ShellExecuteExW failed, err={err}"));
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

        // 兩條都要等 sidecar 連上。連線順序必須跟 sidecar 那邊 `CreateFileA`
        // 的順序一致（先 h2s 再 s2h），否則兩邊會各自等對方先連另一條而互卡。
        log_step("calling ConnectNamedPipe on h2s (with timeout)");
        if let Err(e) = connect_named_pipe_with_timeout(h2s_handle, std::time::Duration::from_secs(30)) {
            log_step(&format!("ConnectNamedPipe(h2s) failed/timed out: {e}"));
            unsafe { CloseHandle(h2s_handle) };
            unsafe { CloseHandle(s2h_handle) };
            return Err(e);
        }
        log_step("calling ConnectNamedPipe on s2h (with timeout)");
        if let Err(e) = connect_named_pipe_with_timeout(s2h_handle, std::time::Duration::from_secs(30)) {
            log_step(&format!("ConnectNamedPipe(s2h) failed/timed out: {e}"));
            unsafe { CloseHandle(h2s_handle) };
            unsafe { CloseHandle(s2h_handle) };
            return Err(e);
        }
        log_step("both pipes connected, constructing ElevatedChannel");

        // 兩個 handle 的所有權從這裡轉移進 `PipeReadHandle`/`PipeWriteHandle`，
        // 由它們的 `Drop` 負責關閉（見兩者的說明）。
        let reader = super::PipeReadHandle(s2h_handle);
        let writer: Box<dyn std::io::Write + Send> = Box::new(super::PipeWriteHandle(h2s_handle));
        let channel = ElevatedChannel::new(reader, writer, on_output, on_disconnect);
        log_step("spawn_windows: returning Ok(Some(channel))");
        Ok(Some(channel))
    }

    /// 建一條單向的具名管線 server。`access` 是 `PIPE_ACCESS_OUTBOUND`（主行
    /// 程只寫）或 `PIPE_ACCESS_INBOUND`（主行程只讀）——刻意不用
    /// `PIPE_ACCESS_DUPLEX`，讓「這個 handle 只能用於單一方向」這件事由核心
    /// 強制，而不是靠呼叫端自律；那正是先前死鎖的來源。
    fn create_pipe(name: &str, access: windows_sys::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES) -> std::io::Result<HANDLE> {
        let c_name = CString::new(name).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let handle: HANDLE = unsafe {
            CreateNamedPipeA(
                c_name.as_ptr() as *const u8,
                access,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                65536,
                65536,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        Ok(handle)
    }

    /// `ConnectNamedPipe` 本身沒有逾時參數（非 overlapped 模式下就是同步阻塞
    /// 到有 client 連上為止），這是文件裡已經記錄過的已知缺口。實機測試證實
    /// 這不是紙上談兵：sidecar 卡住、AITerm 整個視窗跟著沒回應，使用者得去
    /// 工作管理員強制關閉 sidecar 行程才能恢復。
    ///
    /// 這裡改成「另開一條執行緒呼叫真正的 `ConnectNamedPipe`，呼叫端用
    /// `recv_timeout` 等結果」，而不是把整個管線切換成 overlapped I/O——後者
    /// 需要連 `PipeReadHandle`/`PipeWriteHandle` 的每一次 `ReadFile`/`WriteFile`
    /// 都跟著改成 overlapped 語意（`OVERLAPPED` 結構、事件、`GetOverlappedResult`），
    /// 牽動面大很多且沒有本機編譯器能驗證，風險不成比例。
    ///
    /// **逾時之後那條背景執行緒會被放著**：它可能還卡在 `ConnectNamedPipe`
    /// 裡，直到呼叫端關掉 `pipe_handle`（呼叫端在這個函式回傳 `Err` 之後會
    /// 做這件事）讓那個阻塞呼叫連帶失敗、執行緒才會結束——這是刻意的取捨：
    /// 放著一條會在 handle 關閉時自然結束的執行緒，好過讓整個 UI 卡死。
    fn connect_named_pipe_with_timeout(pipe_handle: HANDLE, timeout: std::time::Duration) -> std::io::Result<()> {
        struct SendableHandle(HANDLE);
        unsafe impl Send for SendableHandle {}
        let handle = SendableHandle(pipe_handle);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let handle = handle;
            let connected = unsafe { ConnectNamedPipe(handle.0, std::ptr::null_mut()) };
            let result = if connected == 0 {
                let err = unsafe { GetLastError() };
                // `CreateNamedPipeA` 到 `ConnectNamedPipe` 中間如果 client 剛
                // 好搶著連上，`ConnectNamedPipe` 會回傳失敗但 `GetLastError()`
                // 是 `ERROR_PIPE_CONNECTED`——MSDN 文件記載的正常競態，意思是
                // 「已經連上了」，不是真的錯誤。
                if err == ERROR_PIPE_CONNECTED { Ok(()) } else { Err(err) }
            } else {
                Ok(())
            };
            // 呼叫端逾時放棄後，這個 channel 的接收端已經沒人在聽，`send`
            // 失敗是預期行為，忽略即可。
            let _ = tx.send(result);
        });

        match rx.recv_timeout(timeout) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(win32_err)) => Err(std::io::Error::from_raw_os_error(win32_err as i32)),
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("ConnectNamedPipe did not complete within {timeout:?} — sidecar likely failed to launch or connect"),
            )),
        }
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

/// 讀取執行緒結束（sidecar 斷線）時隨執行緒一起被 drop。
#[cfg(windows)]
impl Drop for PipeReadHandle {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(windows)]
pub(crate) struct PipeWriteHandle(pub windows_sys::Win32::Foundation::HANDLE);

/// 關分頁（`PtySession::kill` 把 `elevated` 設成 `None`）時就是靠這裡通知
/// sidecar 收工：少了它，這條管線要到整個 AITerm 結束才會被作業系統關掉，
/// 期間 sidecar 和它底下的**管理員** shell 一直活著——分頁已經關了，使用者
/// 卻還留著一個看不見的提權 shell。
///
/// 不在關閉前先送 `Frame::Exit`：關 handle 本身就讓 sidecar 的 `ReadFile`
/// 立刻回 `ERROR_BROKEN_PIPE`，效果相同；多送一個 frame 反而可能在 sidecar
/// 卡住、管線緩衝區滿的時候讓這個 drop（跑在 `kill` 裡、持有 session 的鎖）
/// 跟著阻塞。
#[cfg(windows)]
impl Drop for PipeWriteHandle {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}
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

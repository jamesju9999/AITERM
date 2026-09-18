// src-tauri/crates/aiterm-elevated-host/src/windows_host.rs
#![cfg(windows)]

use std::ffi::CString;
use std::io::{Read, Write};

use aiterm_core::pty::elevated_protocol::Frame;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileA, OPEN_EXISTING};

/// 這個行程用 `SW_HIDE` 啟動、沒有可見的主控台——`eprintln!` 寫到一個使用者
/// 看不到、也點不開的隱藏主控台，等於沒地方輸出。實機診斷 hang 時完全看不
/// 到卡在哪一步，只能用工作管理員的行程樹（連子行程都沒有）反推。改成把每
/// 一個關鍵步驟寫進 `%TEMP%\aiterm-elevated-host.log`（附加寫入、每行立刻
/// flush），這樣使用者下次重現時直接看這個檔案最後一行，就知道卡在哪一步
/// ——不用再用「有沒有子行程」這種間接證據猜。
fn log_step(msg: &str) {
    use std::io::Write as _;
    let Some(mut path) = std::env::var_os("TEMP").map(std::path::PathBuf::from) else { return };
    path.push("aiterm-elevated-host.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(f, "[{now}] pid={} {msg}", std::process::id());
        let _ = f.flush();
    }
}

/// 入口：`argv[1]` 是主行程 -> 這裡（我們只讀）的管線名稱，`argv[2]` 是這裡
/// -> 主行程（我們只寫）的管線名稱，`argv[3]` 是 shell variant（"cmd" 或
/// "pwsh"）。連不上管線、或參數不對，直接印錯誤結束——這個行程沒有 UI，唯一
/// 能溝通失敗原因的管道就是 stderr（現在也一併寫進上面說的 log 檔，因為
/// stderr 實務上沒人看得到）。
///
/// **為什麼是兩條單向管線而不是一條雙向的**：見
/// `aiterm_core::pty::elevated` 的 `spawn_windows`。簡述：同步（非 overlapped）
/// 管線 handle 上的 I/O 會被 Windows 序列化，同一個 handle 一邊卡在
/// `ReadFile` 時另一邊的 `WriteFile` 會跟著卡死，實機上會讓主視窗整個沒回應。
pub fn run() {
    log_step("run() start");
    let args: Vec<String> = std::env::args().collect();
    log_step(&format!("argv={args:?}"));
    let (Some(read_pipe_name), Some(write_pipe_name)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: aiterm-elevated-host <host-to-sidecar-pipe> <sidecar-to-host-pipe> <shell-variant>");
        log_step("missing pipe-name arguments, exiting");
        std::process::exit(1);
    };
    let shell_variant = args.get(3).map(String::as_str).unwrap_or("cmd");
    // 尺寸解析不到就退回 80x24——那是舊的寫死值，至少不會比以前差。
    let cols: u16 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(80);
    let rows: u16 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(24);

    // 連線順序必須跟主行程 `ConnectNamedPipe` 的順序一致（先 h2s 再 s2h），
    // 否則兩邊會各自等對方先連另一條而互卡。
    log_step(&format!("connecting to read pipe {read_pipe_name}"));
    let read_pipe = match connect_pipe(read_pipe_name, GENERIC_READ) {
        Ok(h) => {
            log_step("connected to read pipe");
            h
        }
        Err(e) => {
            eprintln!("failed to connect to {read_pipe_name}: {e}");
            log_step(&format!("failed to connect to read pipe: {e}"));
            std::process::exit(1);
        }
    };
    log_step(&format!("connecting to write pipe {write_pipe_name}"));
    let write_pipe = match connect_pipe(write_pipe_name, GENERIC_WRITE) {
        Ok(h) => {
            log_step("connected to write pipe");
            h
        }
        Err(e) => {
            eprintln!("failed to connect to {write_pipe_name}: {e}");
            log_step(&format!("failed to connect to write pipe: {e}"));
            std::process::exit(1);
        }
    };

    if let Err(e) = run_conpty_bridge(read_pipe, write_pipe, shell_variant, cols, rows) {
        eprintln!("conpty bridge failed: {e}");
        log_step(&format!("conpty bridge failed: {e}"));
        std::process::exit(1);
    }
    log_step("run() returned normally");
}

/// 以 client 身分連進主行程已經開好的具名管線 server。不依賴繼承 handle
/// ——UAC broker 本來就不轉送 stdio 的 handle 繼承，這是唯一可靠的做法。
///
/// `access` 只給 `GENERIC_READ` 或 `GENERIC_WRITE`（不是兩個都給）：對面是用
/// `PIPE_ACCESS_INBOUND`/`OUTBOUND` 開的單向 server，要求的存取權限跟管線方
/// 向不符時 `CreateFileA` 會直接失敗。
fn connect_pipe(name: &str, access: u32) -> std::io::Result<HANDLE> {
    let c_name = CString::new(name).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let handle = unsafe {
        CreateFileA(
            c_name.as_ptr() as *const u8,
            access,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    Ok(handle)
}

/// 開一個新的 ConPTY，掛上對應的 shell，雙向轉送到 `pipe`。
///
/// **人工驗證待辦（無法在 mac 上跑）**：
/// - `portable-pty::native_pty_system().openpty()` 在提權行程裡建立 ConPTY
///   是否需要額外權限（理論上不需要，ConPTY 是一般 user-mode API，跟呼叫端
///   是否提權無關）——第一次在真機上跑時要確認。
/// - shell 是否正確以 `cmd.exe` / `powershell.exe` 啟動，尤其 PowerShell 的
///   路徑解析（沿用 `aiterm_core::pty::shell::default_shell` 邏輯還是自己找）。
fn run_conpty_bridge(read_pipe: HANDLE, write_pipe: HANDLE, shell_variant: &str, cols: u16, rows: u16) -> std::io::Result<()> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    // 在建立 pseudoconsole **之前**先脫離本行程自己的主控台。
    //
    // 這是目前找到的、提權路徑與一般路徑之間唯一的結構性差異：一般分頁的
    // ConPTY 是 `app.exe`（GUI subsystem、完全沒有主控台）建的，運作正常；而
    // 這支 sidecar 是 console subsystem 執行檔，被 `ShellExecuteExW` 以
    // `SW_HIDE` 啟動時 Windows 會配一個（隱藏的）主控台給它。實機 log 已證實
    // conhost 本身是活的（它送出了 `ESC[?9001h ESC[?1004h`）、PowerShell 也
    // 活著（watchdog 沒回報結束），但連 `-NoProfile` 配一行 `Write-Host` 的
    // 輸出都到不了——這組症狀指向「子行程沒有真的接上我們建的 pseudoconsole」，
    // 而行程自帶主控台正是最可能干擾主控台歸屬的因素。
    //
    // 這支行程本來就不需要主控台（`SW_HIDE` 啟動、診斷一律走 log 檔，`eprintln!`
    // 早就沒有人看得到），所以脫離它沒有任何損失。
    unsafe {
        use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleWindow};
        let had_console = !GetConsoleWindow().is_null();
        let freed = FreeConsole() != 0;
        log_step(&format!("had own console={had_console}, FreeConsole ok={freed}"));
    }

    log_step(&format!("run_conpty_bridge: calling openpty() at {cols}x{rows}"));
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    log_step("openpty() returned");

    // 用跟一般分頁同一套 shell integration（OSC 133）啟動提權 shell。
    //
    // 這不是美化，是提權畫面能不能顯示的關鍵：AITerm 不是把 PTY 位元組直接畫
    // 上去，而是靠 OSC 133 把輸出切成卡片，沒有 running 中的區塊時輸出會被靜默
    // 丟棄。pty11 曾誤判把這段拿掉（以為 `-EncodedCommand` 害 shell 啞掉），實
    // 際上輸出一路都正常，只是當時兩端的儀表都只記第一筆才看不出來——拿掉的正
    // 好就是讓畫面顯示的那個機制。詳見 `elevated_shell_spec` 的說明。
    let variant = if shell_variant == "pwsh" {
        aiterm_core::pty::cd_parser::ShellVariant::Pwsh
    } else {
        aiterm_core::pty::cd_parser::ShellVariant::Cmd
    };
    let spec = aiterm_core::pty::shell::elevated_shell_spec(variant);
    log_step(&format!("spawning {:?} args={:?}", spec.program, spec.args.len()));
    let mut cmd = CommandBuilder::new(&spec.program);
    for arg in &spec.args {
        cmd.arg(arg);
    }
    for (k, v) in &spec.envs {
        cmd.env(k, v);
    }
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    log_step(&format!("spawn_command returned, child pid={:?}", child.process_id()));
    drop(pair.slave);

    // 提權 shell 自己結束時，這裡以前完全不會發現：主迴圈的 `child.try_wait()`
    // 只在 `Frame::read_from` 回傳之後才會執行，而那個呼叫正卡在等主行程送輸
    // 入過來；輸出執行緒也不會收到 EOF，因為 PseudoConsole 還開著（master 那
    // 份 `Arc<Mutex<Inner>>` 還活著）。結果就是「shell 早就死了，但 sidecar
    // 繼續掛著、主行程也不知道」——實機診斷「提權後畫面沒有任何輸出」時，
    // 正是因為分不出「shell 活著但不吐東西」跟「shell 一起來就死了」而卡關。
    // 用一條專門的執行緒 blocking `wait()`，結束時把 exit status 記下來。
    let mut killer = child.clone_killer();
    let mut child_for_wait = child;
    let child_watchdog = std::thread::spawn(move || {
        match child_for_wait.wait() {
            Ok(status) => log_step(&format!("child exited with status {status:?}")),
            Err(e) => log_step(&format!("child wait failed: {e}")),
        }
    });

    let mut pty_writer = pair
        .master
        .take_writer()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let mut pty_reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    log_step("pty reader/writer ready, entering relay loop");

    let mut pipe_writer = PipeHandle(write_pipe);
    let mut pipe_reader = PipeHandle(read_pipe);

    // ConPTY 輸出 -> 管線，在自己的執行緒跑，避免跟下面「管線輸入 -> ConPTY」的
    // 迴圈互相卡住（雙向轉送的兩個方向不能共用同一個阻塞式迴圈）。
    let output_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        // 這條路徑先前完全沒有儀表——實機出現「提權後畫面零輸出」時，分不出
        // 是 ConPTY 根本沒吐東西（shell 沒起來／卡住）還是吐了但送不回主行程。
        // 只記第一筆與結束時的累計，不是每筆都記，避免把 log 灌爆。
        let mut total: u64 = 0;
        let mut chunks: u32 = 0;
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => {
                    log_step(&format!("output thread: ConPTY EOF after {total} bytes in {chunks} chunks"));
                    break;
                }
                Ok(n) => {
                    chunks += 1;
                    if chunks <= 10 {
                        // 連內容一起記：光看位元組數分不出這是 ConPTY 自己的
                        // 初始序列，還是 shell 真的輸出了什麼。特別要看有沒有
                        // 夾帶需要終端機回覆的查詢序列（例如 DSR `ESC[6n`）
                        // ——那種序列如果沒人回應，shell 會就地卡死等回覆。
                        let shown = n.min(200);
                        log_step(&format!(
                            "output thread: chunk #{chunks}, {n} bytes: {:?}",
                            String::from_utf8_lossy(&buf[..shown])
                        ));
                    }
                    total += n as u64;
                    if let Err(e) = Frame::Data(buf[..n].to_vec()).write_to(&mut pipe_writer) {
                        eprintln!("conpty bridge: failed to write output frame to pipe: {e}");
                        log_step(&format!("output thread: pipe write failed after {total} bytes: {e}"));
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("conpty bridge: ConPTY read failed: {e}");
                    log_step(&format!("output thread: ConPTY read failed after {total} bytes in {chunks} chunks: {e}"));
                    break;
                }
            }
        }
        let _ = Frame::Exit.write_to(&mut pipe_writer);
    });

    // 管線輸入 -> ConPTY，在主執行緒跑。
    let mut in_frames: u32 = 0;
    loop {
        match Frame::read_from(&mut pipe_reader) {
            Ok(Some(Frame::Data(bytes))) => {
                in_frames += 1;
                if in_frames <= 10 {
                    let shown = bytes.len().min(120);
                    log_step(&format!(
                        "input frame #{in_frames}, {} bytes: {:?}",
                        bytes.len(),
                        String::from_utf8_lossy(&bytes[..shown])
                    ));
                }
                if let Err(e) = pty_writer.write_all(&bytes) {
                    eprintln!("conpty bridge: failed to write input to ConPTY: {e}");
                    log_step(&format!("input frame #{in_frames}: write to ConPTY failed: {e}"));
                    break;
                }
                if in_frames <= 10 {
                    log_step(&format!("input frame #{in_frames}: written to ConPTY"));
                }
            }
            Ok(Some(Frame::Resize { cols, rows })) => {
                if let Err(e) = pair.master.resize(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }) {
                    eprintln!("conpty bridge: resize to {cols}x{rows} failed: {e}");
                }
            }
            Ok(Some(Frame::Exit)) | Ok(None) => break,
            Err(e) => {
                eprintln!("conpty bridge: failed to read frame from pipe: {e}");
                break;
            }
        }
    }

    // 已知缺口：`kill()` 在 Windows 上只 TerminateProcess 直屬的 shell 行程，
    // 不會連帶終止 shell 底下開出來的子孫行程（例如使用者在提權 shell 裡跑的
    // 常駐程式）。`aiterm-core/src/pty/session.rs` 的 `kill_tree_first` +
    // Windows Job Object 是同一問題的正確解法，這裡還沒補上——刻意先留著，
    // 之後要回頭處理，不要讓它一直是個未追蹤的缺口。
    let _ = killer.kill();
    let _ = child_watchdog.join();
    let _ = output_thread.join();
    unsafe { CloseHandle(read_pipe) };
    unsafe { CloseHandle(write_pipe) };
    Ok(())
}

/// 把具名管線 `HANDLE` 包成 `Read + Write`，讓它可以直接餵給 `Frame::write_to`/
/// `read_from`（兩者只要求這兩個 trait，不管底層實際上是什麼）。
struct PipeHandle(HANDLE);
unsafe impl Send for PipeHandle {}

impl Read for PipeHandle {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use windows_sys::Win32::Storage::FileSystem::ReadFile;
        let mut n = 0u32;
        let ok = unsafe { ReadFile(self.0, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut()) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(n as usize)
    }
}

impl Write for PipeHandle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use windows_sys::Win32::Storage::FileSystem::WriteFile;
        let mut n = 0u32;
        let ok = unsafe { WriteFile(self.0, buf.as_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut()) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(n as usize)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

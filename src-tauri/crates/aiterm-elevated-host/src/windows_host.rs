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

    if let Err(e) = run_conpty_bridge(read_pipe, write_pipe, shell_variant) {
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
fn run_conpty_bridge(read_pipe: HANDLE, write_pipe: HANDLE, shell_variant: &str) -> std::io::Result<()> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    log_step("run_conpty_bridge: calling openpty()");
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    log_step("openpty() returned");

    let program = if shell_variant == "pwsh" { "powershell.exe" } else { "cmd.exe" };
    log_step(&format!("spawning {program}"));
    let cmd = CommandBuilder::new(program);
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    log_step(&format!("spawn_command returned, child pid={:?}", child.process_id()));
    drop(pair.slave);

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
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(e) = Frame::Data(buf[..n].to_vec()).write_to(&mut pipe_writer) {
                        eprintln!("conpty bridge: failed to write output frame to pipe: {e}");
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("conpty bridge: ConPTY read failed: {e}");
                    break;
                }
            }
        }
        let _ = Frame::Exit.write_to(&mut pipe_writer);
    });

    // 管線輸入 -> ConPTY，在主執行緒跑。
    loop {
        match Frame::read_from(&mut pipe_reader) {
            Ok(Some(Frame::Data(bytes))) => {
                if let Err(e) = pty_writer.write_all(&bytes) {
                    eprintln!("conpty bridge: failed to write input to ConPTY: {e}");
                    break;
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
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
    }

    // 已知缺口：`kill()` 在 Windows 上只 TerminateProcess 直屬的 shell 行程，
    // 不會連帶終止 shell 底下開出來的子孫行程（例如使用者在提權 shell 裡跑的
    // 常駐程式）。`aiterm-core/src/pty/session.rs` 的 `kill_tree_first` +
    // Windows Job Object 是同一問題的正確解法，這裡還沒補上——刻意先留著，
    // 之後要回頭處理，不要讓它一直是個未追蹤的缺口。
    let _ = child.kill();
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

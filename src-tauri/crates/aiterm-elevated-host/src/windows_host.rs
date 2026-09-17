// src-tauri/crates/aiterm-elevated-host/src/windows_host.rs
#![cfg(windows)]

use std::ffi::CString;
use std::io::{Read, Write};

use aiterm_core::pty::elevated_protocol::Frame;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileA, OPEN_EXISTING};

/// 入口：`argv[1]` 是主行程先建好的具名管線名稱，`argv[2]` 是 shell variant
/// （"cmd" 或 "pwsh"）。連不上管線、或參數不對，直接印錯誤結束——這個行程
/// 沒有 UI，唯一能溝通失敗原因的管道就是 stderr。
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let Some(pipe_name) = args.get(1) else {
        eprintln!("usage: aiterm-elevated-host <pipe-name> <shell-variant>");
        std::process::exit(1);
    };
    let shell_variant = args.get(2).map(String::as_str).unwrap_or("cmd");

    let pipe = match connect_pipe(pipe_name) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("failed to connect to {pipe_name}: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = run_conpty_bridge(pipe, shell_variant) {
        eprintln!("conpty bridge failed: {e}");
        std::process::exit(1);
    }
}

/// 以 client 身分連進主行程已經開好的具名管線 server。不依賴繼承 handle
/// ——UAC broker 本來就不轉送 stdio 的 handle 繼承，這是唯一可靠的做法。
fn connect_pipe(name: &str) -> std::io::Result<HANDLE> {
    let c_name = CString::new(name).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let handle = unsafe {
        CreateFileA(
            c_name.as_ptr() as *const u8,
            GENERIC_READ | GENERIC_WRITE,
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
fn run_conpty_bridge(pipe: HANDLE, shell_variant: &str) -> std::io::Result<()> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    let program = if shell_variant == "pwsh" { "powershell.exe" } else { "cmd.exe" };
    let cmd = CommandBuilder::new(program);
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    drop(pair.slave);

    let mut pty_writer = pair
        .master
        .take_writer()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let mut pty_reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    let mut pipe_writer = PipeHandle(pipe);
    let mut pipe_reader = PipeHandle(pipe);

    // ConPTY 輸出 -> 管線，在自己的執行緒跑，避免跟下面「管線輸入 -> ConPTY」的
    // 迴圈互相卡住（雙向轉送的兩個方向不能共用同一個阻塞式迴圈）。
    let output_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if Frame::Data(buf[..n].to_vec()).write_to(&mut pipe_writer).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = Frame::Exit.write_to(&mut pipe_writer);
    });

    // 管線輸入 -> ConPTY，在主執行緒跑。
    loop {
        match Frame::read_from(&mut pipe_reader) {
            Ok(Some(Frame::Data(bytes))) => {
                if pty_writer.write_all(&bytes).is_err() {
                    break;
                }
            }
            Ok(Some(Frame::Resize { cols, rows })) => {
                let _ = pair.master.resize(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
            }
            Ok(Some(Frame::Exit)) | Ok(None) => break,
            Err(_) => break,
        }
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
    }

    let _ = child.kill();
    let _ = output_thread.join();
    unsafe { CloseHandle(pipe) };
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

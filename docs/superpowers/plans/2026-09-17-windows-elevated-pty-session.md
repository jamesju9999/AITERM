# Windows 提權（UAC）PTY Session Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 AITerm 在 Windows 上偵測到指令因權限不足失敗時，能詢問使用者是否要以系統管理員身分重跑；同意後只跳一次 UAC，之後的提權輸出直接併入原本那個分頁的 session（同一個 ring buffer + 同一個 `pty://data/{id}` 事件），讓畫面交錯顯示、AI context 零改動就能看到。

**Architecture:** 新增獨立的 sidecar 執行檔 `aiterm-elevated-host`（Windows-only workspace crate，不依賴 Tauri），用 `ShellExecuteExW(runas)` 提權啟動一次，內部開自己的 ConPTY 掛一個提權 shell，跟主行程之間用具名管線做位元組轉送。主行程收到提權輸出後直接寫進原本 session 的 output ring buffer、發在同一個 Tauri 事件上。`PtySession` 新增 `elevated: Mutex<Option<ElevatedChannel>>` 狀態，決定 `write` 路由到一般子行程還是提權 channel。

**Tech Stack:** Rust（`portable-pty`、`windows-sys` 0.60、`parking_lot`）、React/TypeScript（既有 `src/ipc/pty.ts` IPC 模式）、PowerShell 建置腳本。

**設計依據：** `docs/superpowers/specs/2026-09-17-windows-elevated-pty-session-design.md`（已經使用者核准）。

**已知限制（貫穿整份計畫）：** 這台開發機是 macOS。所有 `#[cfg(windows)]` 底下的 ConPTY／`ShellExecuteExW`／具名管線程式碼**在這台機器上無法編譯進測試二進位、也就無法跑紅燈-綠燈 TDD 迴圈**。這份計畫把邏輯拆成「平台無關、可在 mac 上 TDD」與「真正的 Windows API 呼叫、只能人工在 Windows 機器上驗證」兩類任務，並在每個 Windows-only 任務標明「無法在此驗證」。使用者的 Parallels Windows VM（見既有 memory）是後續驗收的建議環境。

---

## File Structure

新增：

- `src-tauri/crates/aiterm-elevated-host/Cargo.toml` — 新 binary crate。
- `src-tauri/crates/aiterm-elevated-host/src/main.rs` — 入口：解析參數，`#[cfg(windows)]` 分派到 `windows_host::run`，非 Windows 印錯誤並以非零碼結束。
- `src-tauri/crates/aiterm-elevated-host/src/protocol.rs` — 具名管線 frame 編解碼（平台無關）。
- `src-tauri/crates/aiterm-elevated-host/src/windows_host.rs` — `#[cfg(windows)]`：ConPTY 建立 + 管線 client + 轉送迴圈。
- `src-tauri/crates/aiterm-core/src/pty/detection.rs` — 權限不足偵測正則（平台無關）。
- `src-tauri/crates/aiterm-core/src/pty/elevated.rs` — `ElevatedChannel` 狀態機（用 `Read + Write` trait object 抽象傳輸層，平台無關可測），加上 `#[cfg(windows)]` 的具體啟動函式（具名管線 server + `ShellExecuteExW`）。
- `src/components/ElevationBadge/index.tsx` — 分頁提權徽章，仿 `ShellWarningBadge`。
- `src/components/ElevationBadge/index.css`
- `src/hooks/useElevationState.ts` — 訂閱 `pty://elevation-state/{id}` 事件。
- `scripts/setup-elevated-host-win.ps1` — 編譯並複製到 `src-tauri/binaries/`。

修改：

- `src-tauri/Cargo.toml` — `[workspace] members` 加入新 crate。
- `src-tauri/crates/aiterm-core/Cargo.toml` — 新增 `pty::detection`/`pty::elevated` 用到的依賴（沿用既有 `windows-sys`/`portable-pty`，不需要新依賴）。
- `src-tauri/crates/aiterm-core/src/pty/mod.rs` — 新增 `pub mod detection;` `pub mod elevated;` 並 re-export。
- `src-tauri/crates/aiterm-core/src/pty/session.rs:129-199`（`PtySession` struct）— 新增 `elevated: Mutex<Option<ElevatedChannel>>` 欄位；`write`（552-558）改為先檢查 `elevated`；`kill`（831-843）與 `Drop`（971-987）新增提權 channel 的終止。
- `src-tauri/crates/aiterm-core/src/pty/manager.rs:61-63`（`write`）— 不需改，走 session 內部路由；新增 `pub fn elevate(&self, id: &str, shell_variant: ShellVariant) -> PtyResult<()>` 與 `pub fn is_elevated(&self, id: &str) -> Option<bool>`。
- `src-tauri/crates/aiterm-core/src/pty/events.rs` — 新增 `elevation_state_event_name`、`ElevationStatePayload`、`elevation_suggested_event_name`、`ElevationSuggestedPayload`。
- `src-tauri/src/pty/commands.rs` — 新增 `pty_elevate` command；`pty_write`（48-55）維持不變（路由在 session 內部完成）。
- `src-tauri/src/pty/manager.rs` — `create_with_app` 的 `on_data` closure 之後，額外掛一段偵測邏輯（呼叫 `detection::detect`），命中時 emit `elevation_suggested_event_name`。
- `src-tauri/src/lib.rs:141-142,376-382` — `invoke_handler!` 清單加入 `pty_elevate`。
- `src-tauri/tauri.conf.json:49-52` — 不變（Windows-only 功能不進 base）。
- `src-tauri/tauri.windows.conf.json` — `externalBin` 陣列（目前只有 `["binaries/uv"]`，Windows 這份會**整組取代**base 的陣列，不是合併——加新項目必須連 `binaries/uv` 一起寫）加入 `binaries/aiterm-elevated-host`。
- `src/ipc/pty.ts` — 新增 `elevatePty(id, command)`、`onElevationSuggested(id, cb)`、`onElevationState(id, cb)`。
- `src/ipc/events.ts` — 新增對應的事件名稱 helper（仿現有 `ptyDataEvent`）。
- `src/lib/i18n.ts` — `zhTW`（約 line 9 起）與 `enRaw`（line 1596 起）各補 6 個新 key。
- `src/components/TerminalView.tsx:1845` 附近（`<ShellWarningBadge>` 旁邊）— 掛 `<ElevationBadge>`；另新增 inline banner 的掛載點與確認/取消處理。
- `CLAUDE.md` — Commands 表格補一行 Windows 提權 sidecar 的建置注意事項，比照既有 DB2/uv 條目。

---

## Task 1: 具名管線 frame 協定（平台無關，可在 mac TDD）

**Files:**
- Create: `src-tauri/crates/aiterm-elevated-host/Cargo.toml`
- Create: `src-tauri/crates/aiterm-elevated-host/src/main.rs`
- Create: `src-tauri/crates/aiterm-elevated-host/src/protocol.rs`
- Test: 同檔內 `#[cfg(test)] mod tests`（沿用專案慣例，見 `session.rs:989-`）

- [ ] **Step 1: 建立 crate 骨架**

```toml
# src-tauri/crates/aiterm-elevated-host/Cargo.toml
[package]
name = "aiterm-elevated-host"
version = "0.1.0"
edition = "2021"
rust-version = "1.88"

[[bin]]
name = "aiterm-elevated-host"
path = "src/main.rs"

[dependencies]
portable-pty = "0.8"

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.60", features = [
    "Win32_Foundation",
    "Win32_Security",
    "Win32_System_Console",
    "Win32_System_Pipes",
    "Win32_System_Threading",
    "Win32_Storage_FileSystem",
] }
```

- [ ] **Step 2: 加進 workspace**

修改 `src-tauri/Cargo.toml`：

```toml
[workspace]
members = [".", "crates/aiterm-core", "crates/aiterm-host", "crates/aiterm-elevated-host"]
```

- [ ] **Step 3: 寫 frame 協定的失敗測試**

```rust
// src-tauri/crates/aiterm-elevated-host/src/protocol.rs
use std::io::{self, Read, Write};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// 雙向的原始位元組：主行程→sidecar 是鍵盤輸入，sidecar→主行程是 ConPTY 輸出。
    Data(Vec<u8>),
    /// 主行程通知 sidecar 調整 ConPTY 大小。
    Resize { cols: u16, rows: u16 },
    /// 任一端準備關閉連線前的最後一個 frame。
    Exit,
}

const KIND_DATA: u8 = 0;
const KIND_RESIZE: u8 = 1;
const KIND_EXIT: u8 = 2;

impl Frame {
    /// 寫入格式：4 bytes little-endian payload 長度 + 1 byte 種類 + payload。
    /// 長度**只算 payload**，不含種類位元組本身。
    pub fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        match self {
            Frame::Data(bytes) => {
                w.write_all(&(bytes.len() as u32).to_le_bytes())?;
                w.write_all(&[KIND_DATA])?;
                w.write_all(bytes)?;
            }
            Frame::Resize { cols, rows } => {
                w.write_all(&4u32.to_le_bytes())?;
                w.write_all(&[KIND_RESIZE])?;
                w.write_all(&cols.to_le_bytes())?;
                w.write_all(&rows.to_le_bytes())?;
            }
            Frame::Exit => {
                w.write_all(&0u32.to_le_bytes())?;
                w.write_all(&[KIND_EXIT])?;
            }
        }
        Ok(())
    }

    /// 讀一個完整 frame。EOF 在長度前綴之前發生時回傳 `Ok(None)`；讀到一半才
    /// EOF 一律當錯誤，不能把不完整的 frame 當成合法資料處理。
    pub fn read_from<R: Read>(r: &mut R) -> io::Result<Option<Frame>> {
        let mut len_buf = [0u8; 4];
        match r.read(&mut len_buf[..1])? {
            0 => return Ok(None),
            _ => r.read_exact(&mut len_buf[1..])?,
        }
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut kind_buf = [0u8; 1];
        r.read_exact(&mut kind_buf)?;

        match kind_buf[0] {
            KIND_DATA => {
                let mut payload = vec![0u8; len];
                r.read_exact(&mut payload)?;
                Ok(Some(Frame::Data(payload)))
            }
            KIND_RESIZE => {
                if len != 4 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "resize frame must be 4 bytes"));
                }
                let mut buf = [0u8; 4];
                r.read_exact(&mut buf)?;
                let cols = u16::from_le_bytes([buf[0], buf[1]]);
                let rows = u16::from_le_bytes([buf[2], buf[3]]);
                Ok(Some(Frame::Resize { cols, rows }))
            }
            KIND_EXIT => Ok(Some(Frame::Exit)),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown frame kind: {other}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn data_frame_round_trips() {
        let frame = Frame::Data(b"hello".to_vec());
        let mut buf = Vec::new();
        frame.write_to(&mut buf).unwrap();
        let mut cursor = Cursor::new(buf);
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), Some(frame));
    }

    #[test]
    fn resize_frame_round_trips() {
        let frame = Frame::Resize { cols: 120, rows: 40 };
        let mut buf = Vec::new();
        frame.write_to(&mut buf).unwrap();
        let mut cursor = Cursor::new(buf);
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), Some(frame));
    }

    #[test]
    fn exit_frame_round_trips() {
        let frame = Frame::Exit;
        let mut buf = Vec::new();
        frame.write_to(&mut buf).unwrap();
        let mut cursor = Cursor::new(buf);
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), Some(frame));
    }

    #[test]
    fn empty_stream_yields_none() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), None);
    }

    #[test]
    fn two_frames_back_to_back_both_read() {
        let mut buf = Vec::new();
        Frame::Data(b"a".to_vec()).write_to(&mut buf).unwrap();
        Frame::Data(b"bb".to_vec()).write_to(&mut buf).unwrap();
        let mut cursor = Cursor::new(buf);
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), Some(Frame::Data(b"a".to_vec())));
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), Some(Frame::Data(b"bb".to_vec())));
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), None);
    }

    #[test]
    fn truncated_frame_is_an_error_not_none() {
        let mut buf = Vec::new();
        Frame::Data(b"hello".to_vec()).write_to(&mut buf).unwrap();
        buf.truncate(3); // cut mid-length-prefix
        let mut cursor = Cursor::new(buf);
        assert!(Frame::read_from(&mut cursor).is_err());
    }
}
```

- [ ] **Step 4: 最小 main.rs（先讓 crate 能編譯，非 Windows 印錯誤退出）**

```rust
// src-tauri/crates/aiterm-elevated-host/src/main.rs
mod protocol;

#[cfg(windows)]
mod windows_host;

fn main() {
    #[cfg(windows)]
    {
        windows_host::run();
    }
    #[cfg(not(windows))]
    {
        eprintln!("aiterm-elevated-host only runs on Windows");
        std::process::exit(1);
    }
}
```

暫時先放一個空殼 `windows_host.rs`（`#[cfg(windows)] pub fn run() {}`），Task 5 再補完整實作。

- [ ] **Step 5: 跑測試確認全綠**

Run: `cd src-tauri && cargo test -p aiterm-elevated-host`
Expected: 6 個測試全部 `ok`（`data_frame_round_trips`、`resize_frame_round_trips`、`exit_frame_round_trips`、`empty_stream_yields_none`、`two_frames_back_to_back_both_read`、`truncated_frame_is_an_error_not_none`）。這是這份計畫裡**唯一能在這台 mac 上完整跑紅燈再綠燈**的 crate 邏輯——先故意跑一次確認測試檔案本身是對的（例如先把 `KIND_DATA`/`KIND_RESIZE` 的判斷式改錯一次觀察測試真的會紅，再改回來）。

- [ ] **Step 6: 確認 workspace 整體仍然乾淨編譯（這是 mac 上唯一能替代「真的在 Windows 上編譯」的檢查）**

Run: `cd src-tauri && cargo check --workspace`
Expected: 成功，不因為新 crate 而報錯。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/crates/aiterm-elevated-host
git commit -m "feat: scaffold aiterm-elevated-host crate with named-pipe frame protocol"
```

---

## Task 2: 權限不足偵測正則（平台無關，可在 mac TDD）

**Files:**
- Create: `src-tauri/crates/aiterm-core/src/pty/detection.rs`
- Modify: `src-tauri/crates/aiterm-core/src/pty/mod.rs`

- [ ] **Step 1: 確認 `pty/mod.rs` 現有模組清單**

Run: `cat src-tauri/crates/aiterm-core/src/pty/mod.rs`

（讀完再動手改，確保新增的 `pub mod detection;` 插入位置跟既有的 `pub mod ansi;`、`pub mod cd_parser;` 等排列風格一致。）

- [ ] **Step 2: 寫失敗測試——正例與反例都要**

```rust
// src-tauri/crates/aiterm-core/src/pty/detection.rs
use super::cd_parser::ShellVariant;

/// 掃 `recent_output`（已 ANSI-stripped，來自 `PtySession::get_recent_output`）
/// 尾端的一小段，判斷是否像是「剛執行完的指令因權限不足失敗」。
///
/// 只看**最後一段非空白內容**，不是整個 scrollback——避免使用者自己
/// `echo "Access is denied."` 或訊息出現在 scrollback 中段時誤判。
pub fn looks_like_permission_denied(recent_output: &str, shell: ShellVariant) -> bool {
    let tail = last_nonblank_tail(recent_output);
    match shell {
        ShellVariant::Cmd => {
            tail.contains("Access is denied.") || tail.contains("You do not have sufficient privilege")
        }
        ShellVariant::Pwsh => {
            tail.contains("UnauthorizedAccessException")
                || tail.contains("is denied")
                || tail.contains("requires elevation")
        }
        ShellVariant::Bash | ShellVariant::Unknown => false,
    }
}

/// 取輸出尾端最後 8 個非空白行，串成一段字串給正則掃。8 行足夠涵蓋 PowerShell
/// 例外訊息常見的多行堆疊（訊息本文 + `CategoryInfo` + `FullyQualifiedErrorId`），
/// 又不會大到把使用者自己輸入的內容一起吃進來。
fn last_nonblank_tail(output: &str) -> String {
    output
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_access_is_denied_is_detected() {
        let output = "C:\\Windows\\System32>echo test > C:\\Windows\\System32\\x.txt\nAccess is denied.\n\nC:\\Windows\\System32>";
        assert!(looks_like_permission_denied(output, ShellVariant::Cmd));
    }

    #[test]
    fn cmd_insufficient_privilege_is_detected() {
        let output = "You do not have sufficient privilege to perform this operation.";
        assert!(looks_like_permission_denied(output, ShellVariant::Cmd));
    }

    #[test]
    fn pwsh_unauthorized_access_exception_is_detected() {
        let output = "Set-Content : Access to the path 'C:\\Windows\\System32\\x.txt' is denied.\nUnauthorizedAccessException";
        assert!(looks_like_permission_denied(output, ShellVariant::Pwsh));
    }

    #[test]
    fn cmd_variant_does_not_match_pwsh_only_phrase() {
        // 反例：PowerShell 專屬用語出現在 cmd.exe 輸出裡不該觸發——這種輸出
        // 實務上不會發生，但規則本身必須是 shell-variant-scoped 而非全域字串比對。
        let output = "UnauthorizedAccessException";
        assert!(!looks_like_permission_denied(output, ShellVariant::Cmd));
    }

    #[test]
    fn echoing_the_phrase_yourself_mid_scrollback_does_not_trigger() {
        // 反例：這句話出現在很早之前的輸出裡，後面接了一大段其他輸出——
        // 代表它不是「剛執行完那條指令」的結果，只是被捲到 scrollback 中段。
        let mut output = String::from("Access is denied.\n");
        for i in 0..20 {
            output.push_str(&format!("some later unrelated output line {i}\n"));
        }
        assert!(!looks_like_permission_denied(&output, ShellVariant::Cmd));
    }

    #[test]
    fn bash_never_matches() {
        assert!(!looks_like_permission_denied("Access is denied.", ShellVariant::Bash));
    }

    #[test]
    fn empty_output_does_not_match() {
        assert!(!looks_like_permission_denied("", ShellVariant::Cmd));
    }
}
```

- [ ] **Step 3: 跑測試確認全部失敗（模組還沒接進 `mod.rs`，應該是編譯錯誤）**

Run: `cd src-tauri && cargo test -p aiterm-core detection`
Expected: FAIL — `error[E0433]: failed to resolve: unresolved module` 或類似（`detection` 還沒被 `pty/mod.rs` 宣告）。

- [ ] **Step 4: 把模組接進 `pty/mod.rs`**

在既有的 `pub mod cd_parser;` 附近加一行：

```rust
pub mod detection;
```

- [ ] **Step 5: 跑測試確認全綠**

Run: `cd src-tauri && cargo test -p aiterm-core detection`
Expected: 7 個測試全部 `ok`。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/aiterm-core/src/pty/detection.rs src-tauri/crates/aiterm-core/src/pty/mod.rs
git commit -m "feat: detect permission-denied PTY output for cmd.exe and PowerShell"
```

---

## Task 3: `ElevatedChannel` 狀態機（平台無關部分，用 mock transport TDD）

**Files:**
- Create: `src-tauri/crates/aiterm-core/src/pty/elevated.rs`
- Modify: `src-tauri/crates/aiterm-core/src/pty/mod.rs`

這個 channel 的「連線/資料轉送/斷線」狀態轉換邏輯跟實際傳輸方式（具名管線 vs 隨便什麼 `Read + Write`）無關，所以用一個 trait 抽象傳輸層，讓狀態機本身能在 mac 上用 `std::io::Cursor`／記憶體管道測，不需要真的具名管線或 ConPTY。

- [ ] **Step 1: 寫失敗測試——狀態機在斷線時的行為**

```rust
// src-tauri/crates/aiterm-core/src/pty/elevated.rs
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
/// `#[cfg(windows)]` 的 `spawn_windows` 裡，這裡只放狀態機本身，好讓它能在
/// 任何平台上被測試。
pub struct ElevatedChannel {
    writer: Mutex<Box<dyn Write + Send>>,
    state: Arc<Mutex<ElevatedState>>,
}

impl ElevatedChannel {
    /// 給測試與 `spawn_windows` 共用的建構子：呼叫端已經備妥一個雙向的
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
                match crate::elevated_protocol::Frame::read_from(&mut transport_reader) {
                    Ok(Some(crate::elevated_protocol::Frame::Data(bytes))) => on_output(bytes),
                    Ok(Some(crate::elevated_protocol::Frame::Resize { .. })) => {
                        // Resize frames flow the other direction (host -> sidecar);
                        // seeing one here would mean the sidecar echoed it back,
                        // which is not part of the protocol. Ignore defensively.
                    }
                    Ok(Some(crate::elevated_protocol::Frame::Exit)) | Ok(None) => break,
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

    /// 寫入鍵盤輸入。channel 已斷線時回錯，呼叫端（`PtySession::write`）據此
    /// 判斷要不要退回一般子行程。
    pub fn write(&self, data: &[u8]) -> io::Result<()> {
        if self.state() == ElevatedState::Disconnected {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "elevated channel disconnected"));
        }
        let frame = crate::elevated_protocol::Frame::Data(data.to_vec());
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
        let (_write_tx, write_rx) = mpsc::channel::<Vec<u8>>();
        let reader = Cursor::new(Vec::<u8>::new()); // 立刻 EOF
        let writer = Box::new(FakeWriter(mpsc::channel().0));
        drop(write_rx);

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
        crate::elevated_protocol::Frame::Data(b"first".to_vec()).write_to(&mut encoded).unwrap();
        crate::elevated_protocol::Frame::Data(b"second".to_vec()).write_to(&mut encoded).unwrap();
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
```

**注意**：這個檔案引用 `crate::elevated_protocol::Frame`，但那個型別目前定義在 `aiterm-elevated-host` crate（Task 1），跟這裡（`aiterm-core`）不是同一個 crate。下一步先把 `Frame` 抽到兩邊都能用的地方。

- [ ] **Step 2: 把 `Frame` 搬到 `aiterm-core`，讓兩個 crate 共用**

`protocol.rs` 的內容（Task 1 Step 3 寫的那份，含測試）整份搬到
`src-tauri/crates/aiterm-core/src/pty/elevated_protocol.rs`，並：

- `pty/mod.rs` 加 `pub mod elevated_protocol;`
- `aiterm-elevated-host/Cargo.toml` 加上 `aiterm-core = { path = "../aiterm-core" }` 依賴
- `aiterm-elevated-host/src/main.rs` 的 `mod protocol;` 改成 `use aiterm_core::pty::elevated_protocol as protocol;`，刪掉原本的 `protocol.rs`
- 這個檔案（`elevated.rs`）裡的 `crate::elevated_protocol::Frame` 改成 `super::elevated_protocol::Frame`

這樣做的原因：sidecar 進程與主行程各自獨立編譯執行，但**兩邊對同一條管線的 frame 格式認知必須完全一致**——共用同一份程式碼而不是兩邊各寫一份，才不會出現「其中一邊改了格式忘記改另一邊」的協定不同步。

- [ ] **Step 3: 跑測試，確認全部失敗（`elevated.rs` 剛加、還沒接進 `mod.rs`）**

Run: `cd src-tauri && cargo test -p aiterm-core elevated`
Expected: FAIL（未宣告模組）。

- [ ] **Step 4: 接進 `pty/mod.rs`**

```rust
pub mod elevated;
pub mod elevated_protocol;
```

- [ ] **Step 5: 跑測試確認全綠**

Run: `cd src-tauri && cargo test -p aiterm-core elevated`
Expected: `elevated_protocol` 的 5 個測試 + `elevated` 的 3 個測試全部 `ok`。

- [ ] **Step 6: 跑整個 workspace 確認沒有連帶弄壞別的東西**

Run: `cd src-tauri && cargo test --workspace --no-fail-fast 2>&1 | tail -50`
Expected: 沒有新增的失敗（既有的 flaky pty 整合測試不算，見 `CLAUDE.md` 的說明）。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/crates/aiterm-core/src/pty/elevated.rs src-tauri/crates/aiterm-core/src/pty/elevated_protocol.rs src-tauri/crates/aiterm-core/src/pty/mod.rs src-tauri/crates/aiterm-elevated-host
git commit -m "feat: ElevatedChannel state machine, shared frame protocol between host and sidecar"
```

---

## Task 4: `PtySession` 整合 `elevated` 欄位與路由

**Files:**
- Modify: `src-tauri/crates/aiterm-core/src/pty/session.rs`

**這個任務的路由邏輯可測，但「真正產生一個已連線的 `ElevatedChannel`」這件事在 Windows 之外做不到**——測試只驗證「有 `ElevatedChannel` 時 write 走它、沒有時走一般子行程、channel 斷線後自動退回一般子行程」這三條路由規則，用 Task 3 的 mock transport 建構測試用的 channel，不需要真的提權。

- [ ] **Step 1: 讀現有 `write` 方法附近的程式碼，確認插入點**

Run: `sed -n '129,199p;552,558p' src-tauri/crates/aiterm-core/src/pty/session.rs`

（Struct 定義在 129-199 行，`write` 方法在 552-558 行——這份計畫寫下的行號是撰寫當下的快照，實際位置以這次讀到的為準，前後若有其他任務改過檔案就重新抓行號。）

- [ ] **Step 2: 寫失敗測試——路由規則**

在 `session.rs` 的 `#[cfg(test)] mod tests` 區塊裡加：

```rust
    #[test]
    fn write_routes_to_elevated_channel_when_present() {
        use super::super::elevated::ElevatedChannel;
        use std::io::Cursor;
        use std::sync::mpsc;
        use std::time::Duration;

        let session = PtySession::spawn(test_shell(), PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }, None, |_| {})
            .expect("spawn pty");

        let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>();
        struct CapturingWriter(mpsc::Sender<Vec<u8>>);
        impl std::io::Write for CapturingWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                let _ = self.0.send(buf.to_vec());
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let channel = ElevatedChannel::new(
            Cursor::new(Vec::<u8>::new()), // 不會產生輸出，這個測試只關心寫入方向
            Box::new(CapturingWriter(write_tx)),
            |_| {},
            || {},
        );
        *session.elevated.lock() = Some(channel);

        session.write(b"whoami\r\n").expect("write should succeed via elevated channel");

        let captured = write_rx.recv_timeout(Duration::from_secs(2)).expect("elevated channel must receive the write");
        // Frame::Data 編碼後的內容裡應該包含原始位元組（長度前綴+種類位元組之後）。
        assert!(captured.windows(8).any(|w| w == b"whoami\r\n"));

        drop(session);
    }

    #[test]
    fn write_falls_back_to_normal_child_when_elevated_channel_disconnected() {
        use super::super::elevated::ElevatedChannel;
        use std::io::Cursor;
        use std::sync::mpsc;
        use std::time::Duration;

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let session = PtySession::spawn(test_shell(), PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }, None, move |chunk| {
            let _ = tx.send(chunk);
        })
        .expect("spawn pty");

        let (write_tx, _write_rx) = mpsc::channel::<Vec<u8>>();
        struct CapturingWriter(mpsc::Sender<Vec<u8>>);
        impl std::io::Write for CapturingWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                let _ = self.0.send(buf.to_vec());
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        // 空的 reader 讓讀取執行緒立刻看到 EOF、狀態轉成 Disconnected。
        let channel = ElevatedChannel::new(Cursor::new(Vec::<u8>::new()), Box::new(CapturingWriter(write_tx)), |_| {}, || {});
        // 等它真的斷線，避免測試競態。
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while channel.state() != super::super::elevated::ElevatedState::Disconnected && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        *session.elevated.lock() = Some(channel);

        #[cfg(windows)]
        session.write(b"echo HELLO_AITERM\r\nexit\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"echo HELLO_AITERM\nexit\n").unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut buffer = Vec::new();
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => buffer.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if String::from_utf8_lossy(&buffer).contains("HELLO_AITERM") {
                break;
            }
        }
        assert!(
            String::from_utf8_lossy(&buffer).contains("HELLO_AITERM"),
            "disconnected elevated channel must not swallow writes — they should reach the normal child"
        );

        drop(session);
    }
```

- [ ] **Step 3: 跑測試，確認失敗（`elevated` 欄位還不存在）**

Run: `cd src-tauri && cargo test -p aiterm-core --lib write_routes_to_elevated write_falls_back_to_normal`
Expected: FAIL — `error[E0609]: no field 'elevated' on type 'PtySession'`。

- [ ] **Step 4: 加欄位與路由邏輯**

在 struct 定義（129-199 行區間）的 `size: Mutex<PtySize>,` 欄位後面加：

```rust
    /// 這個分頁目前掛著的提權 channel，`None` 代表一般身分。有值但已斷線
    /// （`ElevatedChannel::state()` 回 `Disconnected`）時，`write` 自動退回
    /// 一般子行程——見 `write` 方法。
    pub(crate) elevated: Mutex<Option<super::elevated::ElevatedChannel>>,
```

在 `Ok(Self { ... })`（Task 展示過的那個建構區塊）加一行：

```rust
            elevated: Mutex::new(None),
```

把 `write` 方法（552-558 行）改成：

```rust
    pub fn write(&self, data: &[u8]) -> PtyResult<()> {
        self.record_into_line_buffer(data);
        {
            let elevated = self.elevated.lock();
            if let Some(channel) = elevated.as_ref() {
                if channel.state() == super::elevated::ElevatedState::Connected {
                    return channel
                        .write(data)
                        .map_err(|e| PtyError::Internal(format!("elevated write: {e}")));
                }
                // 斷線：往下穿透到一般子行程，不視為錯誤——這正是自動切回
                // 一般模式的地方。
            }
        }
        let mut writer = self.writer.lock();
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }
```

- [ ] **Step 5: 跑測試確認通過**

Run: `cd src-tauri && cargo test -p aiterm-core --lib write_routes_to_elevated write_falls_back_to_normal`
Expected: 兩個測試都 `ok`。

- [ ] **Step 6: `kill`／`Drop` 一併終止提權 channel**

`kill`（831-843 行附近）與 `Drop::drop`（971-987 行附近）各自在既有的 `#[cfg(windows)] if let Some(job) = &self.job { job.terminate(); }` **之前**加一行：

```rust
        *self.elevated.lock() = None;
```

`ElevatedChannel` 沒有自訂 `Drop`（Task 5 補上具名管線/子行程 handle 後才需要），但先把這行接上，確保之後補上真正的資源釋放時，路徑已經正確——分頁關閉必然會經過這裡，不需要另外在前端加一個「記得清理提權 channel」的呼叫（呼應 memory 裡「分頁關閉時遺漏清理」這個曾經真的發生過的坑）。

- [ ] **Step 7: 跑整個 crate 測試確認沒有連帶壞掉**

Run: `cd src-tauri && cargo test -p aiterm-core --lib`
Expected: 全部通過（含既有的 cd_parser/detection/elevated/session 測試）。

- [ ] **Step 8: Commit**

```bash
git add src-tauri/crates/aiterm-core/src/pty/session.rs
git commit -m "feat: route PtySession writes through an elevated channel when present"
```

---

## Task 5:（Windows-only，無法在此驗證）sidecar 的 ConPTY 宿主

**Files:**
- Modify: `src-tauri/crates/aiterm-elevated-host/src/windows_host.rs`

**這個任務的程式碼無法在這台 mac 上編譯進任何測試二進位**（`#[cfg(windows)]`），因此不走 TDD 紅燈流程。完成後標記為「待 Windows 機器人工驗證」，不能單靠這裡的步驟宣告完成。

- [ ] **Step 1: 參數解析與連線**

```rust
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
            0,
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
```

- [ ] **Step 2: 把 `main.rs` 的空殼 `windows_host` 換成真正的模組**

`main.rs` 已經有 `#[cfg(windows)] mod windows_host;`（Task 1 Step 4），確認它現在指到這個檔案，不用再改。

- [ ] **Step 3: 人工驗證待辦（寫進 PR 說明，不在這裡打勾）**

- [ ] 在 Windows 機器上 `cargo build --release -p aiterm-elevated-host` 確認編譯成功。
- [ ] 手動建一個具名管線 server（可以先用 PowerShell 的 `[System.IO.Pipes.NamedPipeServerStream]` 寫一個十行的測試腳本），跑這支 exe 連上去，確認雙向轉送真的動作。
- [ ] 確認 `child.kill()` 真的終止得了 ConPTY 掛著的 `cmd.exe`/`powershell.exe`（Windows 上 kill 一個父行程不保證子行程一起死，`session.rs` 的 `kill_tree_first`／Job Object 那套邏輯是不是也要搬一份過來——**這點目前這份計畫還沒處理，先列成已知缺口**）。

- [ ] **Step 4: Commit（即使還沒人工驗證，先把程式碼進版控，PR 說明標注「Windows 端未驗證」）**

```bash
git add src-tauri/crates/aiterm-elevated-host/src/windows_host.rs
git commit -m "feat: sidecar ConPTY bridge (Windows-only, unverified on this machine)"
```

---

## Task 6:（Windows-only，無法在此驗證）主行程的提權啟動器

**Files:**
- Modify: `src-tauri/crates/aiterm-core/src/pty/elevated.rs`

- [ ] **Step 1: 加 `#[cfg(windows)]` 的啟動函式**

在 `elevated.rs` 補上（不影響 Task 3 已經寫好、測試過的平台無關部分）：

```rust
#[cfg(windows)]
mod windows_launch {
    use super::{ElevatedChannel, ElevatedState};
    use std::ffi::CString;
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_CANCELLED, HANDLE};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeA, PIPE_ACCESS_DUPLEX, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOCLOSEPROCESS};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    /// 主行程呼叫這個函式來啟動一整套提權流程：先建具名管線 server，再用
    /// `ShellExecuteExW(runas)` 拉起 sidecar，等它連上來。回傳一個已連線的
    /// `ElevatedChannel`，或者使用者在 UAC 對話框按了取消（`ERROR_CANCELLED`）
    /// 時回傳 `None`——這不是錯誤，是使用者的正常選擇。
    ///
    /// **人工驗證待辦（無法在 mac 上跑）**：
    /// - `sidecar_exe_path` 目前假設 Tauri 打包後 `externalBin` 會把
    ///   `binaries/aiterm-elevated-host-x86_64-pc-windows-msvc.exe` 放在跟主
    ///   執行檔同一個目錄，實際路徑要在真機上用
    ///   `tauri::Env`/`current_exe()` 確認，這裡先用 `current_exe()` 所在目錄
    ///   推算，若跟 Tauri 實際打包結構不符要修正。
    /// - `ShellExecuteExW` 回傳成功不代表 sidecar 真的連得上管線（例如管線
    ///   名稱打錯、或防毒軟體攔截）——`ConnectNamedPipe` 需要一個逾時，不能
    ///   無限等，目前先用簡單的 `WaitNamedPipe`／輪詢，實機上要驗證逾時時間
    ///   是否合理。
    pub fn spawn_windows(
        session_id: &str,
        shell_variant: super::super::cd_parser::ShellVariant,
        on_output: impl FnMut(Vec<u8>) + Send + 'static,
        on_disconnect: impl FnMut() + Send + 'static,
    ) -> std::io::Result<Option<ElevatedChannel>> {
        let pipe_name = format!(r"\\.\pipe\aiterm-elevate-{session_id}-{}", uuid::Uuid::new_v4());
        let c_pipe_name = CString::new(pipe_name.clone()).unwrap();

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
        if pipe_handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }

        let sidecar_path = sidecar_exe_path()?;
        let variant_arg = match shell_variant {
            super::super::cd_parser::ShellVariant::Pwsh => "pwsh",
            _ => "cmd",
        };
        let params = format!("\"{pipe_name}\" {variant_arg}");

        let verb = CString::new("runas").unwrap();
        let file = CString::new(sidecar_path.to_string_lossy().into_owned()).unwrap();
        let params_c = CString::new(params).unwrap();

        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = widen(&verb).as_ptr();
        info.lpFile = widen(&file).as_ptr();
        info.lpParameters = widen(&params_c).as_ptr();
        info.nShow = SW_HIDE as i32;

        let ok = unsafe { ShellExecuteExW(&mut info) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err == ERROR_CANCELLED {
                return Ok(None); // 使用者取消 UAC，不是錯誤。
            }
            return Err(std::io::Error::from_raw_os_error(err as i32));
        }

        let connected = unsafe { ConnectNamedPipe(pipe_handle, std::ptr::null_mut()) };
        if connected == 0 {
            return Err(std::io::Error::last_os_error());
        }

        let reader = super::super::elevated::PipeReadHandle(pipe_handle);
        let writer: Box<dyn std::io::Write + Send> = Box::new(super::super::elevated::PipeWriteHandle(pipe_handle));
        Ok(Some(ElevatedChannel::new(reader, writer, on_output, on_disconnect)))
    }

    fn sidecar_exe_path() -> std::io::Result<std::path::PathBuf> {
        let exe = std::env::current_exe()?;
        let dir = exe.parent().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no parent dir"))?;
        Ok(dir.join("aiterm-elevated-host.exe"))
    }

    /// `CString` -> null-terminated UTF-16，`ShellExecuteExW` 要的格式。
    fn widen(s: &CString) -> Vec<u16> {
        s.to_str().unwrap_or("").encode_utf16().chain(std::iter::once(0)).collect()
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
```

- [ ] **Step 2: `windows-sys` 需要的 feature flags**

檢查 `src-tauri/crates/aiterm-core/Cargo.toml` 現有的 `windows-sys` feature 清單（目前已知有 `Win32_Storage_FileSystem`、`Win32_System_WindowsProgramming`——見 `session.rs` 的 Job Object 程式碼用了 `Win32_System_JobObjects`、`Win32_System_Threading`），確認/新增這個任務用到的：`Win32_System_Pipes`、`Win32_UI_Shell`、`Win32_UI_WindowsAndMessaging`、`Win32_Foundation`。

- [ ] **Step 3: mac 上能做的唯一檢查——確認這段程式碼被 `#[cfg(windows)]` 完整包住，不影響非 Windows 編譯**

Run: `cd src-tauri && cargo check --workspace`
Expected: 成功（這段新程式碼在 mac 上完全不會被編譯，`cargo check` 只是確認 `#[cfg(windows)]` 邊界抓對，沒有語法錯誤洩漏到 cfg 外面）。

- [ ] **Step 4: 人工驗證待辦（PR 說明列出，不在此打勾）**

- [ ] Windows 真機：UAC 對話框正常跳出、按「是」後 sidecar 真的連上管線。
- [ ] 按「取消」時 `ERROR_CANCELLED` 路徑正確回傳 `Ok(None)`，不是誤判成錯誤。
- [ ] `sidecar_exe_path()` 在 Tauri 打包後的實際安裝目錄下解析正確（開發模式 `cargo tauri dev` 跟正式安裝版的執行檔相對路徑可能不同，需分別驗證）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/aiterm-core/src/pty/elevated.rs
git commit -m "feat: Windows ShellExecuteExW launcher for the elevated sidecar (unverified on this machine)"
```

---

## Task 7: `PtyManager`/Tauri command/事件整合

**Files:**
- Modify: `src-tauri/crates/aiterm-core/src/pty/manager.rs`
- Modify: `src-tauri/crates/aiterm-core/src/pty/events.rs`
- Modify: `src-tauri/src/pty/commands.rs`
- Modify: `src-tauri/src/pty/manager.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: `events.rs` 加事件名稱與 payload**

```rust
// 加在 events.rs 既有的 closed_event_name 之後
pub fn elevation_state_event_name(session_id: &str) -> String {
    format!("pty://elevation-state/{session_id}")
}

pub fn elevation_suggested_event_name(session_id: &str) -> String {
    format!("pty://elevation-suggested/{session_id}")
}

#[derive(Debug, Clone, Serialize)]
pub struct ElevationStatePayload {
    pub elevated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ElevationSuggestedPayload {
    /// 被判定為權限不足而失敗的那條指令文字，讓前端 banner 顯示，也讓
    /// `pty_elevate` 就緒後可以自動重送。
    pub failed_command: String,
}
```

- [ ] **Step 2: `aiterm-core` 的 `PtyManager` 加 `elevate`/`is_elevated`**

```rust
    /// 對指定 session 啟動提權流程（Windows-only；其他平台回
    /// `PtyError::Internal("elevation not supported on this platform")`）。
    /// `on_output` 收到提權 shell 的原始位元組，呼叫端負責接回 output ring
    /// buffer + Tauri 事件。
    #[cfg(windows)]
    pub fn elevate<F, D>(
        &self,
        id: &str,
        shell_variant: super::cd_parser::ShellVariant,
        on_output: F,
        on_disconnect: D,
    ) -> PtyResult<bool>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
        D: FnMut() + Send + 'static,
    {
        let session = self.get(id)?;
        match super::elevated::spawn_windows(id, shell_variant, on_output, on_disconnect)
            .map_err(|e| PtyError::Internal(format!("elevate: {e}")))?
        {
            Some(channel) => {
                *session.elevated.lock() = Some(channel);
                Ok(true)
            }
            None => Ok(false), // 使用者取消 UAC
        }
    }

    #[cfg(not(windows))]
    pub fn elevate<F, D>(&self, _id: &str, _shell_variant: super::cd_parser::ShellVariant, _on_output: F, _on_disconnect: D) -> PtyResult<bool>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
        D: FnMut() + Send + 'static,
    {
        Err(PtyError::Internal("elevation not supported on this platform".into()))
    }

    pub fn is_elevated(&self, id: &str) -> Option<bool> {
        self.sessions
            .lock()
            .get(id)
            .map(|s| s.elevated.lock().as_ref().map(|c| c.state() == super::elevated::ElevatedState::Connected).unwrap_or(false))
    }
```

- [ ] **Step 3: GUI 端 `src/pty/manager.rs` 加 `elevate_with_app`**

```rust
/// 對指定 session 提權，輸出併入同一個 `pty://data/{id}` 事件；連線狀態變化
/// 額外發 `pty://elevation-state/{id}`，讓前端徽章能顯示/消失。
#[cfg(windows)]
pub fn elevate_with_app(
    manager: &PtyManager,
    app: AppHandle,
    id: String,
    shell_variant: aiterm_core::pty::cd_parser::ShellVariant,
) -> PtyResult<bool> {
    let data_event = data_event_name(&id);
    let app_for_output = app.clone();
    let state_event = aiterm_core::pty::events::elevation_state_event_name(&id);
    let app_for_disconnect = app.clone();
    let state_event_for_disconnect = state_event.clone();

    let started = manager.elevate(
        &id,
        shell_variant,
        move |chunk| {
            let payload = PtyDataPayload { base64: BASE64.encode(&chunk) };
            if let Err(e) = app_for_output.emit(&data_event, payload) {
                eprintln!("emit {data_event} failed: {e}");
            }
        },
        move || {
            let payload = aiterm_core::pty::events::ElevationStatePayload { elevated: false };
            let _ = app_for_disconnect.emit(&state_event_for_disconnect, payload);
        },
    )?;

    if started {
        let payload = aiterm_core::pty::events::ElevationStatePayload { elevated: true };
        let _ = app.emit(&state_event, payload);
    }
    Ok(started)
}

#[cfg(not(windows))]
pub fn elevate_with_app(_manager: &PtyManager, _app: AppHandle, _id: String, _shell_variant: aiterm_core::pty::cd_parser::ShellVariant) -> PtyResult<bool> {
    Err(aiterm_core::pty::error::PtyError::Internal("elevation not supported on this platform".into()))
}
```

- [ ] **Step 4: `create_with_app` 的 `on_data` 裡加偵測**

修改 `create_with_app`（`src/pty/manager.rs:20-44`），在既有的 `move |chunk| { ... app.emit(...) ... }` closure 裡，emit 之後加：

```rust
        // 偵測權限不足，命中時另外發一個建議事件；不影響原本的資料事件。
        if let Some(text) = aiterm_core::pty::ansi::strip_ansi_bytes(&chunk) {
            // 用 manager 現有的 get_recent_output 取「尾端」比只看單一 chunk 準確
            // ——失敗訊息常常跨好幾個 chunk 到。
        }
```

**這段留白是刻意的**——偵測需要跨 chunk 的完整上下文（`detection::looks_like_permission_denied` 吃的是 `get_recent_output` 回傳的字串，不是單一 chunk），而 `on_data` closure 目前拿不到 `&PtyManager` 自身的引用（它是在 session 建立**之前**傳進去的，此時 session/manager 都還不存在）。正確做法是把偵測搬到**讀取執行緒之外**、由一個獨立的輪詢或是在 `pty_write` 之後主動檢查一次——這個檔案交給下一步用另一種方式接。

- [ ] **Step 5: 改用「指令執行後主動檢查」取代「每個 chunk 都掃」**

理由：`PtySession` 的 OSC 133 機制（`last_exit_code`，`session.rs:799-806`）已經知道「一條指令什麼時候執行完」。偵測權限不足的最佳時機是**指令執行完的那一刻**，不是每個 chunk——這樣「尾端」天然就是那條指令的輸出，不需要另外猜視窗大小。

在 `src-tauri/src/pty/commands.rs` 新增：

```rust
/// 主動要求檢查目前 session 最近一次指令是否像是權限不足失敗。前端在收到
/// OSC 133 D（指令結束）時呼叫——沿用既有的 `useTerminalBlocks` 已經在追蹤
/// 的那個訊號，不需要後端另外起一個計時器或輪詢。
#[tauri::command]
pub fn pty_check_permission_denied(
    manager: State<'_, std::sync::Arc<PtyManager>>,
    id: String,
) -> bool {
    let Some(output) = manager.get_recent_output(&id, 4096) else { return false };
    let Some(variant) = manager.get_shell_variant(&id) else { return false };
    aiterm_core::pty::detection::looks_like_permission_denied(&output, variant)
}
```

前端在 Task 8 會接上：既有的 OSC 133 D 監聽器觸發時呼叫這支 command，命中才發 banner——不需要後端自己維護額外的訂閱/輪詢機制，複用前端已經有的「指令結束」訊號。

- [ ] **Step 6: `pty_elevate` Tauri command**

```rust
// src-tauri/src/pty/commands.rs
/// 使用者確認要提權後呼叫。回傳 `true` 代表已經開始提權（或沿用既有的已提
/// 權 channel），`false` 代表使用者在 UAC 對話框按了取消。
#[tauri::command]
pub fn pty_elevate(
    app: tauri::AppHandle,
    manager: State<'_, std::sync::Arc<PtyManager>>,
    id: String,
) -> Result<bool, PtyError> {
    let variant = manager.get_shell_variant(&id).unwrap_or(super::cd_parser::ShellVariant::Cmd);
    crate::pty::elevate_with_app(&manager, app, id, variant)
}
```

- [ ] **Step 7: 註冊進 `invoke_handler!`**

`src-tauri/src/lib.rs:141-142` 的 `use` 清單加 `pty_check_permission_denied, pty_elevate,`；`376-382` 附近的 handler 清單同樣加這兩個 command 名稱。

- [ ] **Step 8: 確認整個 workspace 仍然乾淨編譯**

Run: `cd src-tauri && cargo check --workspace`
Expected: 成功。這是這個任務在 mac 上能做的完整驗證——`pty_elevate`/`elevate_with_app` 的 Windows 分支邏輯本身仍然只能在 Windows 上實測。

- [ ] **Step 9: Commit**

```bash
git add src-tauri/crates/aiterm-core/src/pty/manager.rs src-tauri/crates/aiterm-core/src/pty/events.rs src-tauri/src/pty/commands.rs src-tauri/src/pty/manager.rs src-tauri/src/lib.rs
git commit -m "feat: wire pty_elevate command and permission-denied check into Tauri layer"
```

---

## Task 8: 前端 IPC / banner / 徽章

**Files:**
- Modify: `src/ipc/pty.ts`
- Modify: `src/ipc/events.ts`
- Create: `src/hooks/useElevationState.ts`
- Create: `src/components/ElevationBadge/index.tsx`
- Create: `src/components/ElevationBadge/index.css`
- Modify: `src/lib/i18n.ts`
- Modify: `src/components/TerminalView.tsx`

- [ ] **Step 1: `src/ipc/pty.ts` 加提權相關呼叫**

```typescript
/** 要求對指定 session 提權。回傳 false 代表使用者在 UAC 對話框按了取消。 */
export function elevatePty(id: string): Promise<boolean> {
  return invoke<boolean>("pty_elevate", { id });
}

/** 主動檢查最近一次指令是否像是權限不足失敗（前端在偵測到指令結束時呼叫）。 */
export function checkPermissionDenied(id: string): Promise<boolean> {
  return invoke<boolean>("pty_check_permission_denied", { id });
}
```

- [ ] **Step 2: `src/ipc/events.ts` 加事件名稱 helper**

Run: `grep -n "ptyDataEvent" src/ipc/events.ts` 先看現有寫法，照同樣的函式簽章風格加：

```typescript
export function elevationStateEvent(sessionId: string): string {
  return `pty://elevation-state/${sessionId}`;
}
```

- [ ] **Step 3: `useElevationState` hook**

```typescript
// src/hooks/useElevationState.ts
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { elevationStateEvent } from "../ipc/events";

interface ElevationStatePayload {
  elevated: boolean;
}

/** 訂閱後端的提權狀態事件，供 `ElevationBadge` 顯示/隱藏用。 */
export function useElevationState(sessionId: string | null): boolean {
  const [elevated, setElevated] = useState(false);

  useEffect(() => {
    if (!sessionId) return;
    setElevated(false); // 換分頁時重置，避免沿用上一個 session 的狀態
    let unlisten: (() => void) | undefined;
    void listen<ElevationStatePayload>(elevationStateEvent(sessionId), (event) => {
      setElevated(event.payload.elevated);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [sessionId]);

  return elevated;
}
```

- [ ] **Step 4: `i18n.ts` 加字串**

在 `zhTW`（約 line 9 起，`common_confirm` 附近）加：

```typescript
    elevation_badge_label: "系統管理員",
    elevation_banner_question: "此指令似乎需要系統管理員權限，要用系統管理員身分重新執行嗎？",
    elevation_banner_confirm: "是",
    elevation_banner_cancel: "否",
    elevation_cancelled: "已取消系統管理員權限",
    elevation_disconnected: "系統管理員連線已中斷",
```

在 `enRaw`（line 1596 起）同樣位置加對應英文（`localeSources`/型別推導機制見 memory「i18n 語系漂移」，兩邊 key 必須一致，順序不用一致）：

```typescript
    elevation_badge_label: "Administrator",
    elevation_banner_question: "This command may need administrator privileges. Re-run it as administrator?",
    elevation_banner_confirm: "Yes",
    elevation_banner_cancel: "No",
    elevation_cancelled: "Administrator elevation cancelled",
    elevation_disconnected: "Administrator connection lost",
```

- [ ] **Step 5: `ElevationBadge` 元件**

```tsx
// src/components/ElevationBadge/index.tsx
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

interface Props {
  elevated: boolean;
}

/** 分頁目前是否處於提權模式的徽章，仿 ShellWarningBadge 的樣式與掛載模式。 */
export function ElevationBadge({ elevated }: Props) {
  const { t } = useLocale();
  if (!elevated) return null;
  return (
    <span className="aiterm-elevation-badge" title={t.elevation_badge_label}>
      ⚡ {t.elevation_badge_label}
    </span>
  );
}
```

```css
/* src/components/ElevationBadge/index.css */
.aiterm-elevation-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 12px;
  background: color-mix(in srgb, orange 20%, transparent);
  color: darkorange;
  border: 1px solid color-mix(in srgb, orange 40%, transparent);
}
```

- [ ] **Step 6: 掛進 `TerminalView.tsx`**

在 `<ShellWarningBadge identity={shellIdentity} />`（line 1845 附近）旁邊加：

```tsx
<ElevationBadge elevated={elevated} />
```

在既有的 `const shellIdentity = useShellIdentity(termState);`（line 664 附近）旁邊加：

```tsx
const elevated = useElevationState(sessionId);
```

（`sessionId` 沿用這個檔案裡既有、傳給 `closePty`/`writePty` 等呼叫的同一個變數——不新增額外狀態。）

並在 import 區塊加上 `ElevationBadge`、`useElevationState` 的 import。

- [ ] **Step 7: inline banner——偵測到權限不足時詢問**

在 OSC 133 D（指令結束）既有的處理路徑上（`grep -n "133" src/components/TerminalView.tsx` 先確認掛載點），指令結束時額外呼叫：

```typescript
void checkPermissionDenied(sessionId).then((denied) => {
  if (denied) setShowElevationBanner(true);
});
```

`showElevationBanner`（新增的 `useState(false)`）為 `true` 時渲染一個簡單的 banner：

```tsx
{showElevationBanner && (
  <div className="aiterm-elevation-banner">
    <span>{t.elevation_banner_question}</span>
    <button onClick={() => { setShowElevationBanner(false); void elevatePty(sessionId); }}>
      {t.elevation_banner_confirm}
    </button>
    <button onClick={() => setShowElevationBanner(false)}>{t.elevation_banner_cancel}</button>
  </div>
)}
```

（自動重送剛剛失敗的指令——依照 spec 的設計——需要在 `pty_elevate` 就緒後拿到 `ElevationSuggestedPayload.failed_command`；這份 MVP 先做「按是之後開始提權，但不自動重送」，重送是後續小任務，不在這個 task 的 scope 內卡住整個提權功能。)

- [ ] **Step 8: 前端型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤。

- [ ] **Step 9: 跑前端測試**

Run: `npm run test`
Expected: 既有測試全部通過（這個 task 沒有新增測試檔案——若要補 `ElevationBadge`/`useElevationState` 的測試，是合理的後續小任務，但這份計畫先求前端骨架能編譯、能跑）。

- [ ] **Step 10: Commit**

```bash
git add src/ipc/pty.ts src/ipc/events.ts src/hooks/useElevationState.ts src/components/ElevationBadge src/lib/i18n.ts src/components/TerminalView.tsx
git commit -m "feat: elevation banner and tab badge in the terminal UI"
```

---

## Task 9: 建置腳本 / Tauri 打包設定 / CLAUDE.md

**Files:**
- Create: `scripts/setup-elevated-host-win.ps1`
- Modify: `src-tauri/tauri.windows.conf.json`
- Modify: `CLAUDE.md`

- [ ] **Step 1: 建置腳本**

```powershell
# scripts/setup-elevated-host-win.ps1
# 編譯 aiterm-elevated-host 並複製到 Tauri externalBin 要求的位置（Windows x64）。
# Run once from the workspace root:
#   powershell -ExecutionPolicy Bypass -File scripts\setup-elevated-host-win.ps1

$ErrorActionPreference = "Stop"

$TRIPLE = "x86_64-pc-windows-msvc"
$DEST = "src-tauri\binaries"

Push-Location src-tauri
try {
  Write-Host "==> Building aiterm-elevated-host (release)"
  cargo build --release -p aiterm-elevated-host
} finally {
  Pop-Location
}

New-Item $DEST -ItemType Directory -Force | Out-Null
Copy-Item "src-tauri\target\release\aiterm-elevated-host.exe" "$DEST\aiterm-elevated-host-$TRIPLE.exe" -Force
Write-Host "==> Wrote $DEST\aiterm-elevated-host-$TRIPLE.exe"
```

- [ ] **Step 2: `tauri.windows.conf.json` 的 `externalBin`**

**重要**：這份 conf 的 `externalBin` 會整組取代 base `tauri.conf.json` 的 `externalBin`（不是合併——base 有 `db2-sidecar`+`uv`，目前 windows 這份只有 `uv`，因為 DB2 在 Windows 上走 `resources` 不是 `externalBin`）。所以新增項目時要連 `uv` 一起寫，不能只寫新的一項：

```json
{
  "bundle": {
    "windows": { "...": "..." },
    "externalBin": ["binaries/uv", "binaries/aiterm-elevated-host"],
    "resources": { "...": "..." }
  }
}
```

（實際編輯時只改 `externalBin` 這一個陣列，其餘欄位維持原樣——用 Edit 工具做精準替換，不要整份檔案重寫。）

- [ ] **Step 3: `CLAUDE.md` 補一行建置注意事項**

在 Commands 區塊、`uv` sidecar 那段既有說明後面加一句：

```markdown
- **提權 PTY 的 sidecar 也是一樣的模式**（`binaries/aiterm-elevated-host`，Windows-only，只在 `tauri.windows.conf.json` 的 `externalBin`）：沒先跑 `scripts/setup-elevated-host-win.ps1`，Windows 上連 `cargo check`/`cargo test` 都會在編譯期失敗，原因同 DB2/uv。
```

- [ ] **Step 4: 確認 mac 上的 workspace 不受影響（這個 sidecar 完全沒進 base/mac conf）**

Run: `cd src-tauri && cargo check --workspace && cd .. && npx tsc -b`
Expected: 兩者都成功——這個任務只動 Windows 專屬設定與文件，不該讓 mac 上任何檢查變紅。

- [ ] **Step 5: Commit**

```bash
git add scripts/setup-elevated-host-win.ps1 src-tauri/tauri.windows.conf.json CLAUDE.md
git commit -m "build: package aiterm-elevated-host as a Windows-only externalBin sidecar"
```

---

## Plan Self-Review（撰寫完成後的檢查結果）

**Spec 涵蓋度**：對照 `2026-09-17-windows-elevated-pty-session-design.md` 每一節——架構（Task 1/3/5/6）、觸發流程（Task 2/7）、UI（Task 8）、錯誤處理（Task 3 的斷線回退、Task 6 的 UAC 取消）、平台範圍（各任務的 `#[cfg(windows)]` 邊界）、建置/打包（Task 9）都對得到任務。唯一在 spec 裡提過、這份計畫**刻意未完整實作**的：

1. **自動重送失敗指令**（spec「觸發流程」最後一條）——Task 8 Step 7 只做了「按是之後開始提權」，重送邏輯需要 `ElevationSuggestedPayload.failed_command` 在前端狀態裡保留並在 `pty_elevate` 成功後送出，這段拆成後續小任務，避免這份已經很大的計畫再膨脹，且不影響核心機制先能動起來。
2. **分頁關閉時終止 sidecar 行程**——Task 4 Step 6 把 `elevated` 欄位清空，但 `ElevatedChannel` 目前沒有實作 `Drop` 去真的關閉具名管線 handle、殺掉 sidecar 行程（Task 5/6 的 Windows-only 程式碼還沒補上這段，Task 5 Step 3 已列為已知缺口）。**這是這份計畫最大的已知風險**：提權功能如果被合併但沒補這段，會重演 memory 裡「派工分頁與 claude 行程都不會自動清」的同一類洩漏。建議在 Task 6 之後、正式合併前，追加一個「Task 10：sidecar 行程生命週期」把這段補齊，或至少在 PR 說明明確列成 blocker。

**型別一致性**：`ElevatedChannel`/`ElevatedState`（Task 3）在 Task 4、Task 6 的用法一致；`elevation_state_event_name`/`ElevationStatePayload`（Task 7）在 Task 8 的 `useElevationState.ts` 用法一致；i18n key 名稱（Task 8 Step 4）在 banner/badge 元件裡的引用一致。

**佔位符掃描**：無 TBD/TODO；Task 7 Step 4 的「留白」段落有明確技術理由（closure 拿不到 manager 引用）並在 Step 5 給出替代方案，不是佔位符。

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-09-17-windows-elevated-pty-session.md`. Two execution options:

**1. Subagent-Driven (recommended)** - 每個任務派一個新 subagent，中間有審查點，適合這種「大半任務彼此依賴、但每個任務範圍明確」的計畫。

**2. Inline Execution** - 在這個 session 裡直接照順序執行，批次跑完再一起檢查點。

**在開始之前有一件事想先跟你確認**：這份計畫裡 Task 5、Task 6 的 Windows 原生程式碼（ConPTY 宿主、`ShellExecuteExW` 啟動器）在這台 mac 上完全無法編譯或測試，只能等 Windows 機器人工驗證。你的 Parallels Windows VM 能配合驗收嗎？如果不方便，我會照常把程式碼寫完、commit，但這兩個任務會停在「寫完但未驗證」的狀態，不能算真正完成。

哪一種執行方式？以及 Windows 驗證這件事要怎麼安排？

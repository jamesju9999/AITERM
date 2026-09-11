# Shell 身分徽章 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 分頁狀態列在偵測到 Windows PowerShell 5.1 時顯示橘色徽章，點開後依「PowerShell 7 是否已裝在常見路徑」給出兩種不同的修正建議。

**Architecture:** PowerShell 整合腳本載入時送一次 OSC 7000 回報自己的
edition/version；前端 `useShellIdentity` hook 註冊 OSC 7000 處理器接收；
edition 是 `Desktop`（＝5.1）時才顯示徽章，並且只在那一刻才去呼叫後端探測
`pwsh.exe` 的安裝位置。

**Tech Stack:** Rust（aiterm-core / Tauri command）、React 19 + TypeScript、
xterm.js OSC handler、Vitest + React Testing Library。

**Spec:** `docs/superpowers/specs/2026-09-11-shell-identity-badge-design.md`

**已驗證的前提**（實作時不需重新確認）：

- xterm.js 的 `OscParser.end()` 在沒有註冊對應處理器時走 `_handlerFb`
  （`node_modules/@xterm/xterm/src/common/parser/OscParser.ts:145`），不會
  把 payload 印到畫面。遠端觀看端不會看到亂碼。
- `src-tauri/binaries/uv-aarch64-apple-darwin` 已存在，`cargo check` 在這台
  機器上跑得過。

---

## File Structure

| 檔案 | 責任 |
|------|------|
| `src-tauri/crates/aiterm-core/src/pty/shell.rs` | 腳本常數化 + 送 OSC 7000 + 探測 pwsh.exe 路徑 |
| `src-tauri/src/pty/commands.rs` | `detect_powershell7` Tauri 指令 |
| `src-tauri/src/lib.rs` | 註冊上述指令 |
| `src/ipc/shell.ts` | `detectPowerShell7()` 前端包裝 |
| `src/hooks/useShellIdentity.ts` | 接收並解析 OSC 7000 |
| `src/components/ShellWarningBadge/` | 徽章 + 說明面板 + CSS + 測試 |
| `src/lib/i18n.ts` | 中英文案 |
| `src/components/TerminalView.tsx` | 把 hook 與徽章接上狀態列 |

---

### Task 1: PowerShell 整合腳本常數化（讓它在 macOS 也能被測）

腳本目前整段包在 `#[cfg(windows)]` 裡，連同它的測試。開發機是 macOS，
所以這段腳本改壞了在本機**不會有任何徵兆**——這正是接下來要動它之前必須
先拆掉的地雷。

**Files:**
- Modify: `src-tauri/crates/aiterm-core/src/pty/shell.rs:63-158`（函式本體）
- Modify: `src-tauri/crates/aiterm-core/src/pty/shell.rs:439-485`（既有測試）

- [ ] **Step 1: 把腳本字串抽成不帶 cfg 的模組層常數**

在 `shell.rs` 中 `inject_powershell_integration` 的**前面**（也就是
`#[cfg(windows)]` 屬性之外）新增：

```rust
/// PowerShell 的 OSC 133 整合腳本內容。
///
/// 刻意放在 `#[cfg(windows)]` 外面：這段腳本原本只有 Windows 才編譯得到，
/// 連帶它的測試也只在 Windows 跑，於是在 macOS 開發機上改壞了不會有任何
/// 徵兆。常數化之後，內容本身在任何平台都測得到。
pub(crate) const POWERSHELL_INTEGRATION_SCRIPT: &str = r#"
# ── AITerm Shell Integration (PowerShell) ──
...原本 `let script = r#"..."#;` 裡的完整內容，一字不改地搬過來...
"#;
```

然後把函式裡的 `let script = r#"..."#;` 整段刪掉，並把寫檔改成：

```rust
    let _ = std::fs::write(&script_path, POWERSHELL_INTEGRATION_SCRIPT);
```

- [ ] **Step 2: 把既有的腳本測試改成測常數、拿掉 cfg**

`shell.rs` 測試模組裡的
`powershell_integration_emits_c_via_enter_override_and_b_after_rendered_prompt`
（第 439 行起）目前是 `#[cfg(windows)]`，而且透過呼叫
`inject_powershell_integration` 再讀回檔案來取得內容。改成直接測常數：

- 刪掉 `#[cfg(windows)]` 屬性。
- 把函式開頭取得 `content` 的方式改成 `let content = POWERSHELL_INTEGRATION_SCRIPT;`
  （原本的 `let spec = inject_powershell_integration(...)` 與讀檔全部刪掉）。
- 刪掉函式最後那兩行 `assert_eq!(spec.program, PathBuf::from("pwsh.exe"));`
  以及它上方的中文註解——該註解自己已寫明 `spec.program` 由
  `windows_default_shell_returns_exe_path` 涵蓋，這裡只是為了避免 unused 警告。

其餘所有 `assert!(content.contains(...))` 一條都不要動。

- [ ] **Step 3: 跑測試，確認這條測試現在在 macOS 上真的會執行**

```bash
cd src-tauri && cargo test --workspace powershell_integration_emits_c 2>&1 | tail -5
```

Expected：`test result: ok. 1 passed`。**關鍵是 `1 passed` 而不是
`0 passed; 1 filtered out`**——後者代表 cfg 還沒拿掉，這一步沒有達成目的。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/aiterm-core/src/pty/shell.rs
git commit -m "refactor(pty): PowerShell 整合腳本常數化，讓它在非 Windows 也測得到"
```

---

### Task 2: 腳本送出 OSC 7000 回報 shell 身分

**Files:**
- Modify: `src-tauri/crates/aiterm-core/src/pty/shell.rs`（`POWERSHELL_INTEGRATION_SCRIPT`）
- Test: 同檔案測試模組

- [ ] **Step 1: 寫會紅的測試**

在 `shell.rs` 測試模組新增（不要 `#[cfg(windows)]`）：

```rust
#[test]
fn powershell_integration_reports_shell_identity_via_osc_7000() {
    let content = POWERSHELL_INTEGRATION_SCRIPT;
    assert!(
        content.contains(r#"]7000;shell=PowerShell;edition=$($PSVersionTable.PSEdition);version=$($PSVersionTable.PSVersion)"#),
        "expected the script to report its own edition/version once at load time — \
         前端靠這個分辨 Windows PowerShell 5.1（Desktop）與 PowerShell 7（Core）"
    );
}
```

- [ ] **Step 2: 跑測試確認它是紅的**

```bash
cd src-tauri && cargo test --workspace powershell_integration_reports_shell_identity 2>&1 | tail -5
```

Expected：FAIL，訊息含 `expected the script to report its own edition/version`。

- [ ] **Step 3: 在腳本末尾加上回報那一行**

在 `POWERSHELL_INTEGRATION_SCRIPT` 的最末端（`Set-PSReadLineKeyHandler` 那個
區塊的收尾 `}` 之後、結束的 `"#` 之前）加上：

```powershell

# Shell 身分：載入時送一次。AITerm 用它判斷這個分頁跑的是不是 Windows
# PowerShell 5.1——5.1 算全形字寬度有誤，dir 這類表格輸出會對不齊。
# 沒有註冊處理器的終端機會直接忽略這個序列，不會印出任何東西。
[Console]::Write("$([char]27)]7000;shell=PowerShell;edition=$($PSVersionTable.PSEdition);version=$($PSVersionTable.PSVersion)$([char]7)")
```

- [ ] **Step 4: 跑測試確認它變綠，且沒有弄壞既有的腳本測試**

```bash
cd src-tauri && cargo test --workspace shell:: 2>&1 | tail -5
```

Expected：全部 pass。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/aiterm-core/src/pty/shell.rs
git commit -m "feat(pty): PowerShell 整合腳本以 OSC 7000 回報 edition 與版本"
```

---

### Task 3: 探測已安裝但不在 PATH 上的 pwsh.exe

**Files:**
- Modify: `src-tauri/crates/aiterm-core/src/pty/shell.rs`
- Test: 同檔案測試模組

- [ ] **Step 1: 寫會紅的測試**

純函式 `first_existing` 拆出來的理由就是為了這個測試——真正讀環境變數的那層
在 Windows 以外拿不到任何路徑，測不出東西。

```rust
#[test]
fn first_existing_picks_the_first_candidate_that_is_a_real_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("nope").join("pwsh.exe");
    let present = dir.path().join("pwsh.exe");
    std::fs::write(&present, b"x").expect("write");

    // 順序很重要：不存在的排在前面，確認它真的是「往下找」而不是「回傳最後一個」。
    assert_eq!(first_existing(&[missing.clone(), present.clone()]), Some(present));
    assert_eq!(first_existing(&[missing]), None);
    assert_eq!(first_existing(&[]), None);
}

#[test]
fn first_existing_ignores_a_directory_with_the_right_name() {
    // pwsh.exe 若剛好是個同名資料夾，不該被當成找到了執行檔。
    let dir = tempfile::tempdir().expect("tempdir");
    let as_dir = dir.path().join("pwsh.exe");
    std::fs::create_dir(&as_dir).expect("mkdir");
    assert_eq!(first_existing(&[as_dir]), None);
}
```

`tempfile` 已是這個 crate 的 dev-dependency（`src-tauri/tests/` 有在用）；
若 `cargo test` 抱怨找不到，在 `crates/aiterm-core/Cargo.toml` 的
`[dev-dependencies]` 補上 `tempfile = "3"`。

- [ ] **Step 2: 跑測試確認它是紅的**

```bash
cd src-tauri && cargo test --workspace first_existing 2>&1 | tail -5
```

Expected：編譯失敗，`cannot find function \`first_existing\``。

- [ ] **Step 3: 實作**

在 `shell.rs` 的 `which_on_path`（第 376 行）附近加：

```rust
/// 回傳第一個確實存在且是檔案的候選路徑。
///
/// 從 `find_powershell7` 拆出來是為了可測性：真正組出候選清單的那一層
/// 讀的是 Windows 專屬環境變數，在別的平台一個路徑都產不出來。
fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

/// PowerShell 7 的常見安裝位置——**不看 PATH**。
///
/// 這個函式存在的理由就是「PATH 裡找不到」：AITerm 行程的 PATH 在啟動那
/// 一刻就固定了，使用者裝完 PowerShell 7 若沒重開 AITerm，`which_on_path`
/// 永遠找不到它，於是退回 Windows PowerShell 5.1。實機上真的發生過。
#[cfg(windows)]
fn powershell7_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|v| std::env::var_os(v))
        .map(|d| PathBuf::from(d).join("PowerShell").join("7").join("pwsh.exe"))
        .collect();
    if let Some(d) = std::env::var_os("LOCALAPPDATA") {
        // Microsoft Store 版裝在這裡。
        out.push(PathBuf::from(d).join("Microsoft").join("WindowsApps").join("pwsh.exe"));
    }
    out
}

/// 已安裝、但這個行程的 PATH 找不到的 PowerShell 7。非 Windows 一律 None。
pub fn find_powershell7() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        first_existing(&powershell7_candidates())
    }
    #[cfg(not(windows))]
    {
        None
    }
}
```

- [ ] **Step 4: 跑測試確認變綠**

```bash
cd src-tauri && cargo test --workspace first_existing 2>&1 | tail -5
```

Expected：`2 passed`。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/aiterm-core/src/pty/shell.rs src-tauri/crates/aiterm-core/Cargo.toml
git commit -m "feat(pty): 探測已安裝但不在 PATH 上的 PowerShell 7"
```

---

### Task 4: `detect_powershell7` Tauri 指令與前端包裝

**Files:**
- Modify: `src-tauri/src/pty/commands.rs`（接在 `pty_get_shell_type` 之後，第 86 行）
- Modify: `src-tauri/src/lib.rs:137`（use 清單）與 `:371` 附近（`invoke_handler`）
- Modify: `src/ipc/shell.ts`

- [ ] **Step 1: 加指令**

`src-tauri/src/pty/commands.rs`：

```rust
/// 已安裝但不在 AITerm 行程 PATH 上的 PowerShell 7 路徑。
///
/// 前端只在確認這個分頁跑的是 Windows PowerShell 5.1 之後才呼叫，所以
/// 正常情況（7.x、或非 Windows）完全不會執行到這裡。
#[tauri::command]
pub fn detect_powershell7() -> Option<String> {
    aiterm_core::pty::shell::find_powershell7().map(|p| p.to_string_lossy().into_owned())
}
```

- [ ] **Step 2: 註冊指令**

`src-tauri/src/lib.rs` 第 137 行那串 `use` 裡，在 `pty_close, pty_create,` 之前
加入 `detect_powershell7,`（維持字母序）；第 371 行 `pty_create,` 附近的
`invoke_handler![...]` 清單也加入 `detect_powershell7,`。

- [ ] **Step 3: 確認編得過**

```bash
cd src-tauri && cargo check 2>&1 | tail -4
```

Expected：`Finished`，沒有 error。

- [ ] **Step 4: 前端包裝**

`src/ipc/shell.ts` 加：

```ts
/** 已安裝但不在 AITerm 行程 PATH 上的 PowerShell 7 路徑，沒有就回 null。 */
export function detectPowerShell7(): Promise<string | null> {
  return invoke<string | null>("detect_powershell7");
}
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/pty/commands.rs src-tauri/src/lib.rs src/ipc/shell.ts
git commit -m "feat(pty): detect_powershell7 指令與前端包裝"
```

---

### Task 5: `useShellIdentity` hook

**Files:**
- Create: `src/hooks/useShellIdentity.ts`
- Test: `src/hooks/useShellIdentity.test.ts`

- [ ] **Step 1: 寫會紅的測試**

```ts
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { Terminal } from "@xterm/xterm";
import { useShellIdentity } from "./useShellIdentity";

async function writeToTerm(term: Terminal, data: string) {
  await new Promise<void>((resolve) => term.write(data, resolve));
}

let term: Terminal;
beforeEach(() => {
  term = new Terminal({ cols: 80, rows: 24 });
});
afterEach(() => {
  term.dispose();
});

describe("useShellIdentity", () => {
  it("收到 OSC 7000 之後解析出 shell / edition / version", async () => {
    const { result } = renderHook(() => useShellIdentity(term));
    expect(result.current).toBeNull();

    await act(async () => {
      await writeToTerm(term, "\x1b]7000;shell=PowerShell;edition=Desktop;version=5.1.26100.33158\x07");
    });

    expect(result.current).toEqual({
      shell: "PowerShell",
      edition: "Desktop",
      version: "5.1.26100.33158",
    });
  });

  it("PowerShell 7 回報的是 Core", async () => {
    const { result } = renderHook(() => useShellIdentity(term));
    await act(async () => {
      await writeToTerm(term, "\x1b]7000;shell=PowerShell;edition=Core;version=7.6.6\x07");
    });
    expect(result.current?.edition).toBe("Core");
  });

  it("欄位不齊的 payload 不採用，維持 null", async () => {
    // 寧可不顯示，也不要顯示錯的——徽章的整個價值就是「它說什麼就是什麼」。
    const { result } = renderHook(() => useShellIdentity(term));
    await act(async () => {
      await writeToTerm(term, "\x1b]7000;shell=PowerShell\x07");
    });
    expect(result.current).toBeNull();
  });

  it("OSC 7000 的內容不會被當成文字印進終端機畫面", async () => {
    // 這條同時保護遠端觀看端：那邊沒有註冊這個 handler。
    const bare = new Terminal({ cols: 80, rows: 24 });
    await writeToTerm(bare, "\x1b]7000;shell=PowerShell;edition=Desktop;version=5.1\x07");
    expect(bare.buffer.active.getLine(0)?.translateToString(true)).toBe("");
    bare.dispose();
  });
});
```

- [ ] **Step 2: 跑測試確認它是紅的**

```bash
npx vitest run src/hooks/useShellIdentity.test.ts 2>&1 | tail -12
```

Expected：FAIL，`Failed to resolve import "./useShellIdentity"`。

- [ ] **Step 3: 實作**

`src/hooks/useShellIdentity.ts`：

```ts
import { useEffect, useState } from "react";
import type { Terminal } from "@xterm/xterm";

export interface ShellIdentity {
  /** 目前只會是 "PowerShell"——cmd.exe 的整合靠 PROMPT 環境變數，送不出這個序列。 */
  shell: string;
  /** PowerShell 專屬：`Desktop` 是 Windows PowerShell 5.1，`Core` 是 7.x。 */
  edition: string;
  version: string;
}

function parse(payload: string): ShellIdentity | null {
  const fields = new Map<string, string>();
  for (const part of payload.split(";")) {
    const eq = part.indexOf("=");
    if (eq > 0) fields.set(part.slice(0, eq), part.slice(eq + 1));
  }
  const shell = fields.get("shell");
  const edition = fields.get("edition");
  const version = fields.get("version");
  // 三個欄位缺一就整筆不採用：顯示錯的身分比不顯示更糟。
  if (!shell || !edition || !version) return null;
  return { shell, edition, version };
}

/**
 * 接收 shell 自己回報的身分（OSC 7000，見 aiterm-core 的 shell.rs）。
 *
 * 刻意跟 `useTerminalBlocks` 的 OSC 133 處理器分開：兩者用途無關，
 * 而且這個 hook 的生命週期單純得多——收到就記住，不需要任何 ref 橋接。
 */
export function useShellIdentity(term: Terminal | null): ShellIdentity | null {
  const [identity, setIdentity] = useState<ShellIdentity | null>(null);

  useEffect(() => {
    if (!term) return;
    const disposable = term.parser.registerOscHandler(7000, (data) => {
      const parsed = parse(data);
      if (parsed) setIdentity(parsed);
      // 回 true 代表這個序列已經被處理掉，不要再往下傳。
      return true;
    });
    return () => disposable.dispose();
  }, [term]);

  return identity;
}
```

- [ ] **Step 4: 跑測試確認變綠**

```bash
npx vitest run src/hooks/useShellIdentity.test.ts 2>&1 | tail -8
```

Expected：`4 passed`。

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useShellIdentity.ts src/hooks/useShellIdentity.test.ts
git commit -m "feat(terminal): useShellIdentity 接收 shell 自報的 OSC 7000 身分"
```

---

### Task 6: i18n 文案

**Files:**
- Modify: `src/lib/i18n.ts`（`zhTW` 區塊結尾、`enRaw` 區塊結尾）

- [ ] **Step 1: 加中文**

在 `zhTW` 物件的最後一個鍵之後加：

```ts
    // Shell 身分徽章（只在偵測到 Windows PowerShell 5.1 時出現）
    shell_badge_legacy_powershell: "PowerShell 5.1",
    shell_badge_title: "這個分頁跑的是 Windows PowerShell 5.1",
    shell_badge_why:
      "5.1 計算中文字寬度有誤，dir 這類表格輸出的欄位會對不齊、每列之間多一行空白。",
    shell_badge_found_intro: (path: string) =>
      `您的電腦上已經裝了 PowerShell 7（${path}），但 AITerm 啟動時的 PATH 找不到它。`,
    shell_badge_found_action: "請完全關閉 AITerm 再重新開啟。",
    shell_badge_missing_intro: "建議改用 PowerShell 7。",
    shell_badge_install_command: "winget install --id Microsoft.PowerShell",
    shell_badge_copy: "複製",
    shell_badge_copied: "已複製",
```

- [ ] **Step 2: 加英文**

在 `enRaw` 物件的最後一個鍵之後加：

```ts
    // Shell identity badge (only shown when Windows PowerShell 5.1 is detected)
    shell_badge_legacy_powershell: "PowerShell 5.1",
    shell_badge_title: "This tab is running Windows PowerShell 5.1",
    shell_badge_why:
      "5.1 miscalculates the display width of CJK characters, so table output like dir has misaligned columns and a blank line between every row.",
    shell_badge_found_intro: (path: string) =>
      `PowerShell 7 is already installed (${path}), but it was not on AITerm's PATH at startup.`,
    shell_badge_found_action: "Quit AITerm completely and reopen it.",
    shell_badge_missing_intro: "Switching to PowerShell 7 is recommended.",
    shell_badge_install_command: "winget install --id Microsoft.PowerShell",
    shell_badge_copy: "Copy",
    shell_badge_copied: "Copied",
```

- [ ] **Step 3: 型別檢查**

```bash
npx tsc -b && echo TSC_OK
```

Expected：`TSC_OK`。

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(i18n): shell 身分徽章的中英文案"
```

---

### Task 7: `ShellWarningBadge` 元件

**Files:**
- Create: `src/components/ShellWarningBadge/index.tsx`
- Create: `src/components/ShellWarningBadge/index.css`
- Test: `src/components/ShellWarningBadge/index.test.tsx`

- [ ] **Step 1: 寫會紅的測試**

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

const detectMock = vi.fn();
vi.mock("../../ipc/shell", () => ({
  detectPowerShell7: () => detectMock(),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ShellWarningBadge } from "./index";

afterEach(() => {
  vi.clearAllMocks();
});

const desktop = { shell: "PowerShell", edition: "Desktop", version: "5.1.26100.33158" };
const core = { shell: "PowerShell", edition: "Core", version: "7.6.6" };

function mount(identity: typeof desktop | null) {
  return render(
    <LocaleProvider>
      <ShellWarningBadge identity={identity} />
    </LocaleProvider>,
  );
}

describe("ShellWarningBadge", () => {
  it("PowerShell 7（Core）不顯示徽章", () => {
    detectMock.mockResolvedValue(null);
    mount(core);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("還沒收到身分（例如 cmd.exe）不顯示徽章", () => {
    detectMock.mockResolvedValue(null);
    mount(null);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("Core 的情況完全不呼叫後端探測", () => {
    detectMock.mockResolvedValue(null);
    mount(core);
    expect(detectMock).not.toHaveBeenCalled();
  });

  it("5.1 且找得到 pwsh.exe：告訴使用者重開 AITerm，不叫他安裝", async () => {
    detectMock.mockResolvedValue("C:\\Program Files\\PowerShell\\7\\pwsh.exe");
    mount(desktop);
    const badge = await screen.findByRole("button");
    await userEvent.click(badge);

    await screen.findByText(/C:\\Program Files\\PowerShell\\7\\pwsh\.exe/);
    expect(screen.queryByText(/winget install/)).toBeNull();
  });

  it("5.1 且找不到 pwsh.exe：給安裝指令", async () => {
    detectMock.mockResolvedValue(null);
    mount(desktop);
    const badge = await screen.findByRole("button");
    await userEvent.click(badge);

    await screen.findByText(/winget install --id Microsoft\.PowerShell/);
  });

  it("再點一次收起面板", async () => {
    detectMock.mockResolvedValue(null);
    mount(desktop);
    const badge = await screen.findByRole("button");
    await userEvent.click(badge);
    await screen.findByText(/winget install/);
    await userEvent.click(badge);
    await waitFor(() => expect(screen.queryByText(/winget install/)).toBeNull());
  });
});
```

- [ ] **Step 2: 跑測試確認它是紅的**

```bash
npx vitest run src/components/ShellWarningBadge 2>&1 | tail -10
```

Expected：FAIL，`Failed to resolve import "./index"`。

- [ ] **Step 3: 實作元件**

`src/components/ShellWarningBadge/index.tsx`：

```tsx
import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { detectPowerShell7 } from "../../ipc/shell";
import type { ShellIdentity } from "../../hooks/useShellIdentity";
import "./index.css";

interface Props {
  identity: ShellIdentity | null;
}

/** Windows PowerShell 5.1 回報的 edition 值。7.x 回 "Core"。 */
const LEGACY_EDITION = "Desktop";

/**
 * 只在這個分頁跑的是 Windows PowerShell 5.1 時才出現的警告徽章。
 *
 * 5.1 算全形字寬度有誤，dir 這類表格輸出每列都會溢出換行。使用者無從得知
 * 跑的不是 PowerShell 7——實機上發生過「明明裝了 7.6.6，AITerm 啟動時的
 * PATH 卻找不到 pwsh.exe」，查了很久才發現。
 */
export function ShellWarningBadge({ identity }: Props) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const [pwsh7Path, setPwsh7Path] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const isLegacy = identity?.shell === "PowerShell" && identity.edition === LEGACY_EDITION;

  useEffect(() => {
    // 只有真的要顯示徽章時才去問後端——正常情況一次都不會呼叫。
    if (!isLegacy) return;
    let alive = true;
    void detectPowerShell7().then((p) => {
      if (alive) setPwsh7Path(p);
    });
    return () => {
      alive = false;
    };
  }, [isLegacy]);

  if (!isLegacy) return null;

  async function onCopy() {
    await navigator.clipboard.writeText(t.shell_badge_install_command);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }

  return (
    <span className="aiterm-shellwarn">
      <button
        className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm aiterm-shellwarn__btn"
        title={t.shell_badge_title}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((v) => !v);
        }}
      >
        <span>⚠ {t.shell_badge_legacy_powershell}</span>
      </button>

      {open && (
        <div className="aiterm-shellwarn__panel" onClick={(e) => e.stopPropagation()}>
          <div className="aiterm-shellwarn__title">{t.shell_badge_title}</div>
          <div className="aiterm-shellwarn__why">{t.shell_badge_why}</div>

          {pwsh7Path ? (
            <div className="aiterm-shellwarn__body">
              <div>{t.shell_badge_found_intro(pwsh7Path)}</div>
              <div className="aiterm-shellwarn__action">{t.shell_badge_found_action}</div>
            </div>
          ) : (
            <div className="aiterm-shellwarn__body">
              <div>{t.shell_badge_missing_intro}</div>
              <div className="aiterm-shellwarn__cmdrow">
                <code className="aiterm-shellwarn__cmd">{t.shell_badge_install_command}</code>
                <button
                  className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                  onClick={() => void onCopy()}
                >
                  {copied ? t.shell_badge_copied : t.shell_badge_copy}
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </span>
  );
}
```

- [ ] **Step 4: CSS**

`src/components/ShellWarningBadge/index.css`（照 `SharePanel/index.css` 的
定位方式，面板靠右展開）：

```css
.aiterm-shellwarn {
  position: relative;
  display: inline-flex;
}

.aiterm-shellwarn__btn {
  border-color: #f59e0b;
  color: #f59e0b;
}

.aiterm-shellwarn__panel {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  z-index: 40;
  width: 340px;
  padding: 12px;
  border-radius: 8px;
  background: var(--aiterm-surface-2, #1e293b);
  border: 1px solid var(--aiterm-border, #334155);
  box-shadow: 0 8px 24px #0008;
  font-size: 13px;
  line-height: 1.6;
  white-space: normal;
}

.aiterm-shellwarn__title {
  font-weight: 600;
  margin-bottom: 8px;
}

.aiterm-shellwarn__why {
  color: var(--aiterm-text-muted, #94a3b8);
  margin-bottom: 10px;
}

.aiterm-shellwarn__action {
  margin-top: 6px;
  font-weight: 600;
}

.aiterm-shellwarn__cmdrow {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
}

.aiterm-shellwarn__cmd {
  flex: 1;
  font-family: var(--aiterm-mono, monospace);
  font-size: 12px;
  word-break: break-all;
}
```

- [ ] **Step 5: 跑測試確認變綠**

```bash
npx vitest run src/components/ShellWarningBadge 2>&1 | tail -8
```

Expected：`6 passed`。

- [ ] **Step 6: Commit**

```bash
git add src/components/ShellWarningBadge/
git commit -m "feat(terminal): PowerShell 5.1 警告徽章與說明面板"
```

---

### Task 8: 接上 TerminalView 狀態列

**Files:**
- Modify: `src/components/TerminalView.tsx`（import、hook 呼叫、狀態列 JSX 約第 1936 行）

- [ ] **Step 1: 加 import**

在既有的 import 區塊加：

```tsx
import { useShellIdentity } from "../hooks/useShellIdentity";
import { ShellWarningBadge } from "./ShellWarningBadge";
```

- [ ] **Step 2: 呼叫 hook**

在元件內、`termState` 已經可用的位置（跟 `cellHeightPx` 的計算同一區，
約第 730 行）加：

```tsx
  const shellIdentity = useShellIdentity(termState);
```

- [ ] **Step 3: 放進狀態列**

在 `{sessionId && <SharePanel sessionId={sessionId} />}`（約第 1936 行）的
**前面**插入：

```tsx
          <ShellWarningBadge identity={shellIdentity} />
```

- [ ] **Step 4: 型別檢查 + 全套測試**

```bash
npx tsc -b && echo TSC_OK
npx vitest run 2>&1 | grep -E "Test Files|Tests |FAIL"
npx eslint src/hooks/useShellIdentity.ts src/components/ShellWarningBadge/ src/components/TerminalView.tsx src/ipc/shell.ts
```

Expected：`TSC_OK`；全部測試 pass；eslint 除了 `TerminalView.tsx` 既有的
`react-hooks/refs` 那條之外沒有新錯誤。

**注意**：`useTerminalBlocks.ts:144`（`writeRef.current = write`）有一條
**既有的** `react-hooks/refs` 錯誤，不是這次改出來的，不要順手修。

- [ ] **Step 5: Rust 全套測試**

```bash
cd src-tauri && cargo test --workspace 2>&1 | tail -15
```

Expected：全部 pass。**一定要加 `--workspace`**——`src-tauri/Cargo.toml`
同時是 package 與 workspace root，裸的 `cargo test` 只會跑 `app`，
`aiterm-core` 會被整批跳過而且沒有任何徵兆。

- [ ] **Step 6: Commit**

```bash
git add src/components/TerminalView.tsx
git commit -m "feat(terminal): 狀態列接上 shell 身分徽章"
```

---

## 驗收（需要 Windows 實機）

本機（macOS）能驗的到 Task 8 為止。以下只有 Windows 實機能確認，做法是推
一個 `v<版本>-<主題><n>` 形式的 pre-release tag 觸發建置（見
`project_release_test_tag_workflow`），**推 tag 前要先問過使用者**：

1. 在 PowerShell 7 的分頁：狀態列**不該**出現徽章。
2. 在 Windows PowerShell 5.1 的分頁（把 pwsh.exe 暫時移出 PATH 重開
   AITerm 即可重現）：狀態列出現橘色「⚠ PowerShell 5.1」。
3. 點徽章：因為 `C:\Program Files\PowerShell\7\pwsh.exe` 存在，面板應顯示
   「已經裝了 PowerShell 7…請完全關閉 AITerm 再重新開啟」，**不該**出現
   winget 指令。
4. cmd.exe 的分頁：不出現徽章（它送不出 OSC 7000）。
5. 遠端觀看端看這個分頁：畫面上不該出現 `7000;shell=PowerShell` 之類的
   殘留文字。

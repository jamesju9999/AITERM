# 提示使用者設定 Claude Code terminal bell — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 使用者在 AITerm 裡執行 `claude` 而 Claude Code 的 `preferredNotifChannel` 尚未設定時，跳出一次性卡片，同意後直接幫他寫進 `~/.claude/settings.json`。

**Architecture:** 指令文字在 `useTerminalBlocks`（唯一漏斗）往上送 → `TerminalView` 用純函式 `isClaudeCommand` 比對 → `TerminalApp` → `App.tsx` 持有 `claudeSeen` 狀態並仲裁三張角落卡片的優先序 → `ClaudeNotifPrompt` 顯示。設定檔的讀寫在 Rust，核心函式吃路徑參數所以能用 tempfile 測試，`#[tauri::command]` 只是解析 `~/.claude/settings.json` 的薄殼。

**Tech Stack:** React 19、TypeScript、Vitest + React Testing Library、Rust、`serde_json`（要開 `preserve_order`）、`dirs`、`tempfile`。

**設計文件：** `docs/superpowers/specs/2026-08-05-claude-notif-prompt-design.md`

---

## 背景：讀之前要知道的事

**為什麼需要這個功能。** 側邊欄的終端機提示點靠 xterm 的 `onBell` 偵測「CLI 停下來等回答」。但 Claude Code 依官方文件預設**只在 Ghostty、Kitty、iTerm2 送通知**，AITerm 不在名單內——所以最主要的使用情境預設完全不會亮，而且是靜默失效。加一行 `"preferredNotifChannel": "terminal_bell"` 就能解決，問題是沒人會知道要設。

**角落卡片會重疊。** `UpdateModal.css` 的 `.update-modal-backdrop` 名字有誤導性，它其實是 `position: fixed; right: 16px; bottom: 16px; max-width: 380px` 的**角落卡片**，不是全螢幕遮罩。既有的更新提示與 `AppImageIntegrationPrompt` 都用它，兩張同時出現會完全重疊。所以優先序不是美觀問題，是顯示錯誤。

**`AppImageIntegrationPrompt` 是本功能的樣板。** `src/components/AppImageIntegrationPrompt.tsx` 與 `src/components/AppImageIntegrationPrompt.test.tsx` 的形狀（偵測條件 → 卡片 → accept 直接做完 → decline 永久記錄 → 讓位給更新提示 → 8 個測試案例）幾乎可以整組對照。實作前先讀它們兩個。

**檔案結構**

| 檔案 | 責任 |
|---|---|
| `src/lib/claudeCommand.ts`（新增） | `isClaudeCommand` 純函式。整個偵測唯一有分支邏輯的地方。 |
| `src-tauri/src/commands/claude_notif.rs`（新增） | 設定檔的讀取判斷與寫入。核心函式吃路徑參數，指令是薄殼。 |
| `src-tauri/src/config/types.rs` | `AppConfig` 加一個婉拒旗標。 |
| `src-tauri/src/commands/config.rs` | 婉拒旗標的讀寫指令。 |
| `src-tauri/src/lib.rs` | 註冊四個新指令。 |
| `src-tauri/Cargo.toml` | `serde_json` 開 `preserve_order`。 |
| `src/ipc/config.ts` | 四個 IPC 包裝。 |
| `src/components/ClaudeNotifPrompt.tsx`（新增） | 卡片本體。 |
| `src/hooks/useTerminalBlocks.ts` | 多送一個 `onCommandStarted(cmd)`。 |
| `src/components/TerminalView.tsx` | 比對指令，往上送 `onClaudeDetected`。 |
| `src/components/TerminalApp.tsx` | 轉送 `onClaudeDetected`。 |
| `src/App.tsx` | 持有 `claudeSeen`，仲裁三張卡片的優先序。 |
| `src/components/AppImageIntegrationPrompt.tsx` | 多回報一個「我正在顯示」給 `App.tsx`。 |
| `src/lib/i18n.ts` | 6 組字串 × 2 語言。 |

**測試的覆蓋邊界（誠實說明）**

自動測試涵蓋：比對規則（純函式）、設定檔讀寫（Rust + tempfile）、卡片的顯示／隱藏／接受／婉拒（Vitest）。

**偵測的接線（`useTerminalBlocks` → `TerminalView` → `TerminalApp` → `App.tsx`）沒有自動化防護**，與上一個功能相同：那條路徑要真的 mount `TerminalView` 才測得到，而那需要偽造 Tauri IPC、xterm 與 PTY。Task 8 的手動驗證是它唯一的閘門。

**不要為此寫「鏡像 harness」測試。** 上一個功能有人為 `TerminalView` 寫過手抄同樣邏輯的測試，後來刪掉了：它與出貨程式碼零耦合（唯一的 `src/` import 是 `import type`，編譯時就被抹掉），把實作反轉之後仍然全綠。若某段邏輯非鏡像不能測，那是「該把邏輯搬進純函式」的訊號。

---

## Task 1: `isClaudeCommand` 純函式

**Files:**
- Create: `src/lib/claudeCommand.ts`
- Test: `src/lib/claudeCommand.test.ts`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/lib/claudeCommand.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { isClaudeCommand } from "./claudeCommand";

describe("isClaudeCommand", () => {
  it("認得單獨的 claude", () => {
    expect(isClaudeCommand("claude")).toBe(true);
  });

  it("認得帶參數的 claude", () => {
    expect(isClaudeCommand("claude --resume")).toBe(true);
  });

  it("認得完整路徑", () => {
    expect(isClaudeCommand("/usr/local/bin/claude")).toBe(true);
    expect(isClaudeCommand("C:\\Users\\me\\bin\\claude --continue")).toBe(true);
  });

  it("忽略前後空白", () => {
    expect(isClaudeCommand("  claude  ")).toBe(true);
  });

  it("不認別的指令", () => {
    expect(isClaudeCommand("claude-foo")).toBe(false);
    expect(isClaudeCommand("myclaude")).toBe(false);
  });

  it("claude 只是參數時不算", () => {
    expect(isClaudeCommand("echo claude")).toBe(false);
    expect(isClaudeCommand("which claude")).toBe(false);
  });

  it("空字串不算", () => {
    expect(isClaudeCommand("")).toBe(false);
    expect(isClaudeCommand("   ")).toBe(false);
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npx vitest run src/lib/claudeCommand.test.ts`
Expected: FAIL，訊息類似 `Failed to resolve import "./claudeCommand"`。

- [ ] **Step 3: 寫最小實作**

建立 `src/lib/claudeCommand.ts`：

```ts
/**
 * 這一行指令是不是在啟動 Claude Code？
 *
 * 只認「第一個 token 的檔名剛好是 claude」。刻意不支援環境變數前綴
 * （`FOO=1 claude`）與 `npx claude`：前者罕見，後者不是 Claude Code 的
 * 安裝方式，而支援它們要把單純的字串比對變成 shell 語法解析。漏報的
 * 代價只是這次不提示，下次執行照樣有機會。
 */
export function isClaudeCommand(cmd: string): boolean {
  const first = cmd.trim().split(/\s+/)[0];
  if (!first) return false;
  // Windows 的路徑用反斜線，POSIX 用斜線——兩種都要切。
  const basename = first.split(/[/\\]/).pop();
  return basename === "claude";
}
```

- [ ] **Step 4: 執行測試，確認它通過**

Run: `npx vitest run src/lib/claudeCommand.test.ts`
Expected: PASS，7 個 test 全綠。

- [ ] **Step 5: 證明測試有鑑別力**

把實作暫時改成 `return cmd.includes("claude");`，重跑。
Expected: 「不認別的指令」與「claude 只是參數時不算」兩個 test 轉紅。

還原實作，再跑一次確認 7 個全綠。**回報這兩次的實際輸出。**

- [ ] **Step 6: Commit**

```bash
git add src/lib/claudeCommand.ts src/lib/claudeCommand.test.ts
git commit -m "feat(claude-notif): 判斷指令是否在啟動 Claude Code"
```

---

## Task 2: Rust — 設定檔的讀取判斷與寫入

這是全功能最需要測試的部分，**因為它動的是別人的設定檔**。核心函式吃路徑參數，所以能用 `tempfile` 測；`#[tauri::command]` 留到 Task 3。

**Files:**
- Modify: `src-tauri/Cargo.toml`（`serde_json` 加 feature）
- Create: `src-tauri/src/commands/claude_notif.rs`
- Modify: `src-tauri/src/commands/mod.rs`（宣告新模組）

- [ ] **Step 1: 開啟 `serde_json` 的 `preserve_order`**

`src-tauri/Cargo.toml` 第 21 行 `serde_json = "1.0"` 改成：

```toml
serde_json = { version = "1.0", features = ["preserve_order"] }
```

沒有這個 feature 的話，`serde_json::Map` 是 `BTreeMap`，「解析 → 插入 → 序列化」會把使用者的 key **全部按字母重排**，整個設定檔改頭換面。這個 feature 把它換成 `IndexMap`，保留插入順序。只影響無型別 `Value` 的序列化順序（`derive` 的結構本來就照宣告順序），對送往 API 的 JSON 無關緊要。

- [ ] **Step 2: 宣告模組**

`src-tauri/src/commands/mod.rs` 的 `pub mod appimage;` 後面加入：

```rust
pub mod claude_notif;
```

- [ ] **Step 3: 寫失敗的測試**

建立 `src-tauri/src/commands/claude_notif.rs`，**只放測試**（實作下一步才寫）：

```rust
//! 判斷並設定 Claude Code 的 terminal bell 通知channel。

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// 建一個假的 ~/.claude 目錄，回傳 settings.json 的路徑。
    /// 目錄本身要活到測試結束，所以 leak 掉——測試裡可以接受，
    /// 與 config/mod.rs 既有的 temp_store() 同樣做法。
    fn claude_dir_with(contents: Option<&str>) -> std::path::PathBuf {
        let dir = tempdir().unwrap();
        let claude = dir.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        let path = claude.join("settings.json");
        if let Some(text) = contents {
            fs::write(&path, text).unwrap();
        }
        std::mem::forget(dir);
        path
    }

    #[test]
    fn no_claude_dir_means_no_prompt() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".claude").join("settings.json");
        assert!(!needs_prompt_at(&path));
    }

    #[test]
    fn missing_settings_file_asks() {
        let path = claude_dir_with(None);
        assert!(needs_prompt_at(&path));
    }

    #[test]
    fn absent_key_asks() {
        let path = claude_dir_with(Some(r#"{"model":"sonnet"}"#));
        assert!(needs_prompt_at(&path));
    }

    #[test]
    fn auto_asks() {
        let path = claude_dir_with(Some(r#"{"preferredNotifChannel":"auto"}"#));
        assert!(needs_prompt_at(&path));
    }

    #[test]
    fn already_terminal_bell_does_not_ask() {
        let path = claude_dir_with(Some(r#"{"preferredNotifChannel":"terminal_bell"}"#));
        assert!(!needs_prompt_at(&path));
    }

    #[test]
    fn explicit_other_channel_does_not_ask() {
        // 使用者已經為別的終端機做過決定，不要打擾。
        let path = claude_dir_with(Some(r#"{"preferredNotifChannel":"iterm2"}"#));
        assert!(!needs_prompt_at(&path));
        let path = claude_dir_with(Some(r#"{"preferredNotifChannel":"notifications_disabled"}"#));
        assert!(!needs_prompt_at(&path));
    }

    #[test]
    fn broken_json_does_not_ask() {
        // 看不懂的檔案就不要碰。
        let path = claude_dir_with(Some("{ this is not json"));
        assert!(!needs_prompt_at(&path));
    }

    #[test]
    fn creates_file_when_missing() {
        let path = claude_dir_with(None);
        enable_bell_at(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""preferredNotifChannel": "terminal_bell""#));
    }

    #[test]
    fn preserves_existing_keys_and_their_order() {
        let original = "{\n  \"zeta\": 1,\n  \"alpha\": 2,\n  \"middle\": {\n    \"nested\": true\n  }\n}\n";
        let path = claude_dir_with(Some(original));
        enable_bell_at(&path).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        // 原本的 key 一個都不能少，值也不能變。
        assert!(text.contains("\"zeta\": 1"));
        assert!(text.contains("\"alpha\": 2"));
        assert!(text.contains("\"nested\": true"));
        // 順序不變：zeta 仍在 alpha 前面（沒有被字母重排）。
        let zeta = text.find("zeta").unwrap();
        let alpha = text.find("alpha").unwrap();
        assert!(zeta < alpha, "key 順序被重排了：{text}");
        // 新的 key 附加在最後。
        assert!(text.find("preferredNotifChannel").unwrap() > alpha);
    }

    #[test]
    fn overwrites_auto() {
        let path = claude_dir_with(Some(r#"{"preferredNotifChannel":"auto"}"#));
        enable_bell_at(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("terminal_bell"));
        assert!(!text.contains("\"auto\""));
    }

    #[test]
    fn broken_json_errors_and_leaves_file_untouched() {
        // 使用者的設定檔壞掉是他自己要處理的事，不是我們拿來重置的理由。
        let original = "{ this is not json";
        let path = claude_dir_with(Some(original));
        assert!(enable_bell_at(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn non_object_root_errors_and_leaves_file_untouched() {
        let original = "[1, 2, 3]";
        let path = claude_dir_with(Some(original));
        assert!(enable_bell_at(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }
}
```

- [ ] **Step 4: 執行測試，確認它失敗**

Run: `cd src-tauri && cargo test claude_notif`
Expected: **編譯失敗**，`cannot find function 'needs_prompt_at' in this scope`（以及 `enable_bell_at`）。

> 若 `cargo test` 因為找不到 `src-tauri/binaries/` 裡的 sidecar 而失敗，先跑 `scripts/setup-uv-mac.sh`（或你平台對應的腳本）。`tauri-build` 的 `build.rs` 會在**編譯期**檢查每個 `externalBin` 都存在——這是 CLAUDE.md 記載的陷阱。

- [ ] **Step 5: 寫實作**

在 `src-tauri/src/commands/claude_notif.rs` 的 `#[cfg(test)] mod tests` **之前**插入：

```rust
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

const KEY: &str = "preferredNotifChannel";
const BELL: &str = "terminal_bell";

/// `~/.claude/settings.json`。取不到 home 目錄時回 None。
fn claude_settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

/// 讀成頂層 JSON 物件。檔案不存在或是空的都算「還沒有設定」→ 空物件。
/// 解析失敗或根不是物件都回 Err——呼叫端各自決定要怎麼處理。
fn read_object(settings_path: &Path) -> Result<Map<String, Value>, String> {
    if !settings_path.exists() {
        return Ok(Map::new());
    }
    let text = fs::read_to_string(settings_path)
        .map_err(|e| format!("讀取 {} 失敗：{e}", settings_path.display()))?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(format!("{} 的最外層不是 JSON 物件", settings_path.display())),
        Err(e) => Err(format!("{} 不是合法的 JSON：{e}", settings_path.display())),
    }
}

/// 要不要提示使用者設定 terminal bell？
///
/// 條件：`~/.claude/` 存在，且 `preferredNotifChannel` 不存在或等於 "auto"。
/// 任何其他明確的值都代表使用者已經做過決定，不打擾。解析不了的檔案一律
/// 回 false——看不懂的東西就不要碰。
pub(crate) fn needs_prompt_at(settings_path: &Path) -> bool {
    match settings_path.parent() {
        Some(dir) if dir.is_dir() => {}
        _ => return false,
    }
    let Ok(map) = read_object(settings_path) else { return false };
    match map.get(KEY) {
        None => true,
        Some(Value::String(s)) => s == "auto",
        Some(_) => false,
    }
}

/// 把 `preferredNotifChannel` 設成 `terminal_bell`，其餘內容原樣保留。
///
/// 檔案不存在就建立（含上層目錄）。JSON 壞掉時回 Err 而且**不寫入**：
/// 使用者的設定檔壞掉是他自己要處理的事，不是我們拿來重置的理由。
pub(crate) fn enable_bell_at(settings_path: &Path) -> Result<(), String> {
    let mut map = read_object(settings_path)?;
    map.insert(KEY.to_string(), Value::String(BELL.to_string()));

    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("建立 {} 失敗：{e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(map))
        .map_err(|e| format!("序列化設定失敗：{e}"))?;
    fs::write(settings_path, text + "\n")
        .map_err(|e| format!("寫入 {} 失敗：{e}", settings_path.display()))
}
```

- [ ] **Step 6: 執行測試，確認它通過**

Run: `cd src-tauri && cargo test claude_notif`
Expected: PASS，12 個 test 全綠。

> `claude_settings_path` 此刻還沒有呼叫者，編譯器會警告 `never used`。Task 3 會用到它，暫時的警告可以忽略——**不要**為此加 `#[allow(dead_code)]`，那會在真的用不到時掩蓋問題。

- [ ] **Step 7: 證明測試有鑑別力**

把 `read_object` 裡 `Err(e) => Err(...)` 那一行暫時改成 `Err(_) => Ok(Map::new())`（也就是「JSON 壞掉就當成空設定」），重跑。
Expected: `broken_json_does_not_ask` 與 `broken_json_errors_and_leaves_file_untouched` 轉紅——證明我們真的守住了「不覆寫壞檔案」。

還原，再跑一次確認 12 個全綠。**回報這兩次的實際輸出。**

- [ ] **Step 8: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/commands/mod.rs src-tauri/src/commands/claude_notif.rs
git commit -m "feat(claude-notif): 讀寫 Claude Code settings.json 的 terminal bell 設定"
```

---

## Task 3: Rust — Tauri 指令與婉拒旗標

**Files:**
- Modify: `src-tauri/src/commands/claude_notif.rs`（加兩個指令）
- Modify: `src-tauri/src/config/types.rs`（`AppConfig` 加欄位）
- Modify: `src-tauri/src/commands/config.rs`（兩個旗標指令）
- Modify: `src-tauri/src/lib.rs`（註冊）

- [ ] **Step 1: 加入兩個 Tauri 指令**

在 `src-tauri/src/commands/claude_notif.rs` 的 `#[cfg(test)]` **之前**追加：

```rust
#[tauri::command]
pub fn claude_notif_needs_prompt() -> bool {
    claude_settings_path().map(|p| needs_prompt_at(&p)).unwrap_or(false)
}

#[tauri::command]
pub fn claude_notif_enable_bell() -> Result<(), String> {
    let path = claude_settings_path().ok_or_else(|| "找不到使用者的 home 目錄".to_string())?;
    enable_bell_at(&path)
}
```

- [ ] **Step 2: `AppConfig` 加婉拒旗標**

在 `src-tauri/src/config/types.rs` 第 36 行 `pub appimage_integration_declined: bool,` **後面**加入：

```rust
    /// Set when the user declines the Claude Code terminal-bell prompt, so it is
    /// asked once rather than on every `claude` run.
    #[serde(default)]
    pub claude_notif_declined: bool,
```

在同檔案的 `Default` 實作裡，第 134 行 `appimage_integration_declined: false,` **後面**加入：

```rust
            claude_notif_declined: false,
```

- [ ] **Step 3: 加入旗標指令**

在 `src-tauri/src/commands/config.rs` 的 `set_appimage_integration_declined` 函式（第 35-40 行）**後面**加入：

```rust
#[tauri::command]
pub fn is_claude_notif_declined(config: State<Arc<ConfigStore>>) -> bool {
    config.get().claude_notif_declined
}

#[tauri::command]
pub fn set_claude_notif_declined(config: State<Arc<ConfigStore>>) -> Result<(), String> {
    config
        .update(|cfg| { cfg.claude_notif_declined = true; })
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 4: 註冊四個指令**

`src-tauri/src/lib.rs`：

1. 在既有的 `use` 區塊裡，`commands::config::{...}` 那一組加入 `is_claude_notif_declined, set_claude_notif_declined`（第 37 行附近，維持原本的字母順序排列風格）。
2. 另外加入 `use crate::commands::claude_notif::{claude_notif_enable_bell, claude_notif_needs_prompt};`（放在其他 `commands::` 匯入的旁邊）。
3. 在 `invoke_handler` 的 `// Config` 區塊裡，`set_appimage_integration_declined,` 後面加入：

```rust
            is_claude_notif_declined,
            set_claude_notif_declined,
            claude_notif_needs_prompt,
            claude_notif_enable_bell,
```

先讀 `lib.rs` 確認匯入的實際寫法再動手——上面的行號是寫計畫時讀到的，Task 2 沒有改過那個檔案，但仍要以實際內容為準。

- [ ] **Step 5: 編譯並跑既有測試**

Run: `cd src-tauri && cargo test`
Expected: 全部通過，且沒有 `never used` 警告（`claude_settings_path` 現在有呼叫者了）。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/claude_notif.rs src-tauri/src/config/types.rs src-tauri/src/commands/config.rs src-tauri/src/lib.rs
git commit -m "feat(claude-notif): 註冊 IPC 指令與婉拒旗標"
```

---

## Task 4: IPC 包裝與 i18n 字串

**Files:**
- Modify: `src/ipc/config.ts`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 加入 IPC 包裝**

在 `src/ipc/config.ts` 第 97 行 `setAppImageIntegrationDeclined` 的定義**後面**加入：

```ts
export const isClaudeNotifDeclined = (): Promise<boolean> =>
  invoke<boolean>("is_claude_notif_declined");

export const setClaudeNotifDeclined = (): Promise<void> =>
  invoke<void>("set_claude_notif_declined");

export const claudeNotifNeedsPrompt = (): Promise<boolean> =>
  invoke<boolean>("claude_notif_needs_prompt");

export const claudeNotifEnableBell = (): Promise<void> =>
  invoke<void>("claude_notif_enable_bell");
```

- [ ] **Step 2: 加入 zh-TW 字串**

先用 `grep -n "appimage_prompt_title" src/lib/i18n.ts` 找到兩處（zh-TW 一處、en 一處）。在 **zh-TW** 那一處所屬的 appimage 字串群組後面加入：

```ts
    // Claude Code terminal bell 提示卡片
    claude_notif_title: "讓 Claude Code 在背景分頁提醒你？",
    claude_notif_body: "Claude Code 預設只在 Ghostty、Kitty、iTerm2 送通知，所以它在 AITerm 裡停下來等你回答時，側邊欄不會有任何提示。",
    claude_notif_detail: "同意的話，AITerm 會在 ~/.claude/settings.json 補上一行 \"preferredNotifChannel\": \"terminal_bell\"，其他設定原樣不動。",
    claude_notif_enable: "幫我設定",
    claude_notif_decline: "不用了",
    claude_notif_done: "已設定。請開一個新的終端機分頁——Claude Code 只在啟動時讀設定，現在正在跑的那個不會有反應。",
    claude_notif_dismiss: "知道了",
```

- [ ] **Step 3: 加入 en 字串**

在 **en** 那一處對應位置加入：

```ts
    claude_notif_title: "Let Claude Code alert you in background tabs?",
    claude_notif_body: "Claude Code only sends notifications in Ghostty, Kitty and iTerm2 by default, so when it stops to ask you something in AITerm, the sidebar shows nothing.",
    claude_notif_detail: "If you agree, AITerm adds one line to ~/.claude/settings.json — \"preferredNotifChannel\": \"terminal_bell\" — and leaves everything else untouched.",
    claude_notif_enable: "Set it up",
    claude_notif_decline: "No thanks",
    claude_notif_done: "Done. Open a new terminal tab — Claude Code only reads its settings at startup, so the session running right now won't change.",
    claude_notif_dismiss: "Got it",
```

- [ ] **Step 4: 型別檢查**

Run: `npx tsc -b`
Expected: 沒有輸出、exit 0。

> **不要**用 `tsc --noEmit`。根目錄 `tsconfig.json` 是 solution file（`"files": []`），那樣跑什麼都不會檢查而且一定回傳 0。這是 CLAUDE.md 明文記載的陷阱。

- [ ] **Step 5: Commit**

```bash
git add src/ipc/config.ts src/lib/i18n.ts
git commit -m "feat(claude-notif): IPC 包裝與提示卡片字串"
```

---

## Task 5: 卡片元件

**Files:**
- Create: `src/components/ClaudeNotifPrompt.tsx`
- Test: `src/components/ClaudeNotifPrompt.test.tsx`

實作前先讀 `src/components/AppImageIntegrationPrompt.tsx` 與 `src/components/AppImageIntegrationPrompt.test.tsx`——本任務是它們的對照版本。

- [ ] **Step 1: 寫失敗的測試**

建立 `src/components/ClaudeNotifPrompt.test.tsx`：

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { ClaudeNotifPrompt } from "./ClaudeNotifPrompt";

const DEFAULTS: Record<string, unknown> = {
  claude_notif_needs_prompt: true,
  is_claude_notif_declined: false,
  is_onboarding_done: true,
};

function mockCommands(overrides: Record<string, unknown> = {}) {
  const table = { ...DEFAULTS, ...overrides };
  invokeMock.mockImplementation((cmd: string) =>
    Promise.resolve(cmd in table ? table[cmd] : null),
  );
}

const TITLE = "讓 Claude Code 在背景分頁提醒你？";

beforeEach(() => { invokeMock.mockReset(); });

describe("ClaudeNotifPrompt", () => {
  it("在偵測到 claude 且設定缺失時提示", async () => {
    mockCommands();
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);

    expect(await screen.findByText(TITLE)).toBeInTheDocument();
  });

  it("還沒偵測到 claude 就不提示", async () => {
    mockCommands();
    render(<ClaudeNotifPrompt claudeSeen={false} blocked={false} />);

    // 連查都不該查——沒偵測到就沒有理由碰使用者的設定檔。
    await waitFor(() => expect(invokeMock).not.toHaveBeenCalled());
    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });

  it("設定已經有值時不提示", async () => {
    mockCommands({ claude_notif_needs_prompt: false });
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });

  it("婉拒過就不再問", async () => {
    mockCommands({ is_claude_notif_declined: true });
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });

  it("onboarding 還沒完成就不提示", async () => {
    mockCommands({ is_onboarding_done: false });
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });

  it("讓位給更優先的角落卡片", async () => {
    // 三張卡片都固定在右下角同一個位置，同時出現會完全重疊。
    mockCommands();
    render(<ClaudeNotifPrompt claudeSeen blocked />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });

  it("接受後寫入設定，並告訴使用者要開新分頁", async () => {
    mockCommands();
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);

    await userEvent.click(await screen.findByRole("button", { name: "幫我設定" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("claude_notif_enable_bell"));
    // 這句是使用者最容易誤判成「設了也沒用」的地方，必須真的出現。
    expect(await screen.findByText(/請開一個新的終端機分頁/)).toBeInTheDocument();
  });

  it("寫入失敗時顯示錯誤，不假裝成功", async () => {
    mockCommands();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "claude_notif_enable_bell") return Promise.reject("權限不足");
      return Promise.resolve(cmd in DEFAULTS ? DEFAULTS[cmd] : null);
    });
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);

    await userEvent.click(await screen.findByRole("button", { name: "幫我設定" }));

    expect(await screen.findByText(/權限不足/)).toBeInTheDocument();
    expect(screen.queryByText(/請開一個新的終端機分頁/)).not.toBeInTheDocument();
  });

  it("婉拒會被記錄下來", async () => {
    mockCommands();
    render(<ClaudeNotifPrompt claudeSeen blocked={false} />);

    await userEvent.click(await screen.findByRole("button", { name: "不用了" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_claude_notif_declined"));
    expect(screen.queryByText(TITLE)).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npx vitest run src/components/ClaudeNotifPrompt.test.tsx`
Expected: FAIL，`Failed to resolve import "./ClaudeNotifPrompt"`。

- [ ] **Step 3: 寫實作**

建立 `src/components/ClaudeNotifPrompt.tsx`：

```tsx
import { useEffect, useState } from "react";
import {
  claudeNotifEnableBell,
  claudeNotifNeedsPrompt,
  isClaudeNotifDeclined,
  isOnboardingDone,
  setClaudeNotifDeclined,
} from "../ipc/config";
import { useLocale } from "../contexts/LocaleContext";
// Deliberate reuse: this is the same bottom-right card as the update toast and
// the AppImage prompt, and a third copy of those rules would drift from them.
import "./UpdateModal.css";

interface Props {
  /** 這個 session 已經偵測到使用者執行 claude。 */
  claudeSeen: boolean;
  /** 有更優先的角落卡片正在顯示（更新提示或 AppImage 提示）。 */
  blocked: boolean;
}

export function ClaudeNotifPrompt({ claudeSeen, blocked }: Props) {
  const { t } = useLocale();
  const [offer, setOffer] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // 沒偵測到 claude 就完全不查——沒有理由去碰使用者的設定檔。
    if (!claudeSeen) return;
    let cancelled = false;
    (async () => {
      try {
        const [needs, declined, onboarded] = await Promise.all([
          claudeNotifNeedsPrompt(),
          isClaudeNotifDeclined(),
          isOnboardingDone(),
        ]);
        if (cancelled) return;
        setOffer(needs && !declined && onboarded);
      } catch {
        // Best-effort: a failure here must never block the app.
      }
    })();
    return () => { cancelled = true; };
    // claudeSeen 只會 false → true 一次，所以這個查詢一個 session 只跑一次。
  }, [claudeSeen]);

  if (!offer || blocked) return null;

  const accept = async () => {
    try {
      await claudeNotifEnableBell();
      setDone(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const decline = async () => {
    setOffer(false);
    await setClaudeNotifDeclined().catch(() => {});
  };

  return (
    <div className="update-modal-backdrop">
      <div className="update-modal" role="status" aria-label={t.claude_notif_title}>
        <p className="update-modal-title">{t.claude_notif_title}</p>
        {done ? (
          <>
            {/* 這句不能省：Claude Code 只在啟動時讀設定，不講的話使用者會
                以為設了沒用——我們自己驗證時就踩過這一步。 */}
            <p className="update-modal-notes">{t.claude_notif_done}</p>
            <div className="update-modal-actions">
              <button className="aiterm-btn aiterm-btn--primary" onClick={() => setOffer(false)}>
                {t.claude_notif_dismiss}
              </button>
            </div>
          </>
        ) : (
          <>
            <p className="update-modal-notes">{t.claude_notif_body}</p>
            <p className="update-modal-notes">{t.claude_notif_detail}</p>
            {error && <p className="update-modal-error">{error}</p>}
            <div className="update-modal-actions">
              <button className="aiterm-btn aiterm-btn--secondary" onClick={() => void decline()}>
                {t.claude_notif_decline}
              </button>
              <button className="aiterm-btn aiterm-btn--primary" onClick={() => void accept()}>
                {t.claude_notif_enable}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: 執行測試，確認它通過**

Run: `npx vitest run src/components/ClaudeNotifPrompt.test.tsx`
Expected: PASS，9 個 test 全綠。

- [ ] **Step 5: 型別檢查與 scoped lint**

Run: `npx tsc -b && npx eslint src/components/ClaudeNotifPrompt.tsx src/components/ClaudeNotifPrompt.test.tsx src/lib/claudeCommand.ts src/ipc/config.ts`
Expected: 都沒有輸出、exit 0。

> 不要用 `npm run lint` 當通過標準。全庫 lint 現況本來就有約 91 個既有問題，全落在與本功能無關的檔案。只檢查自己動過的檔案。

- [ ] **Step 6: Commit**

```bash
git add src/components/ClaudeNotifPrompt.tsx src/components/ClaudeNotifPrompt.test.tsx
git commit -m "feat(claude-notif): 提示卡片元件"
```

---

## Task 6: 接線

把指令文字從 `useTerminalBlocks` 一路送到 `App.tsx`，並讓三張角落卡片排出優先序。

**Files:**
- Modify: `src/hooks/useTerminalBlocks.ts`
- Modify: `src/components/TerminalView.tsx`
- Modify: `src/components/TerminalApp.tsx`
- Modify: `src/components/AppImageIntegrationPrompt.tsx`
- Modify: `src/App.tsx`

- [ ] **Step 1: `useTerminalBlocks` 送出指令文字**

在 `src/hooks/useTerminalBlocks.ts` 的函式簽章加入第六個參數（接在 `onCommandSettled` 後面）：

```ts
  /** 每次開始追蹤一個新指令就呼叫，帶上指令文字。給「偵測使用者跑了什麼」用。
   *  必須是穩定的參考（useCallback 空依賴或 ref 橋接）——它進了 submitCommand
   *  與 beginTrackedBlock 的依賴陣列，每次換身分都會讓兩者的識別跟著變。 */
  onCommandStarted?: (cmd: string) => void,
```

在 `submitCommand` 的 `useCallback` 內，**最前面**（`if (!term || !sessionId) return;` 之後）加入：

```ts
      onCommandStarted?.(cmd);
```

在 `beginTrackedBlock` 的 `useCallback` 內，**最前面**（`if (!sessionId) return;` 之後）加入：

```ts
      onCommandStarted?.(cmd);
```

兩者的依賴陣列都加上 `onCommandStarted`。

**放在這裡而不是 `TerminalView`**：`TerminalView` 有 8 處呼叫 `submitCommand`、另有一處呼叫 `beginTrackedBlock`。逐一攔截既脆弱，又會在下次有人新增呼叫點時默默失效。

**放在函式最前面而不是成功建立 block 之後**：`clear`／`cls` 會提早 return，但那也是使用者確實執行過的指令。我們要的是「使用者跑了什麼」，不是「產生了哪些 block」。

- [ ] **Step 2: `TerminalView` 比對並往上送**

在 `src/components/TerminalView.tsx` 的 import 區塊加入：

```ts
import { isClaudeCommand } from "../lib/claudeCommand";
```

在 `TerminalViewProps` 介面裡，`onAttention` 後面加入：

```ts
  /** 使用者在這個分頁執行了 Claude Code。用來提示他設定 terminal bell。 */
  onClaudeDetected?: () => void;
```

在元件參數的解構加上 `onClaudeDetected`。

在既有的 `onAttentionRef` / `emitAttention` 那一段**後面**加入（同樣的 ref 橋接理由）：

```tsx
  const onClaudeDetectedRef = useRef(onClaudeDetected);
  useEffect(() => { onClaudeDetectedRef.current = onClaudeDetected; }, [onClaudeDetected]);

  const handleCommandStarted = useCallback((cmd: string) => {
    if (isClaudeCommand(cmd)) onClaudeDetectedRef.current?.();
  }, []);
```

把 `handleCommandStarted` 當第六個參數傳給 `useTerminalBlocks`：

```tsx
  const { blocks, isAlternateBuffer, submitCommand, beginTrackedBlock, appendOutput, setBlockGitInfo } = useTerminalBlocks(
    sessionId,
    termState,
    lastCwdRef,
    forceLiveRepaint,
    handleCommandSettled,
    handleCommandStarted,
  );
```

- [ ] **Step 3: `TerminalApp` 轉送**

在 `src/components/TerminalApp.tsx` 的 props 介面加入 `onClaudeDetected?: () => void;`，解構出來，並在渲染 `TerminalView` 的地方（`onAttention` 那一行旁邊）加入：

```tsx
                  onClaudeDetected={onClaudeDetected}
```

`TerminalApp` 只是轉送，不做任何判斷。

- [ ] **Step 4: `AppImageIntegrationPrompt` 回報自己是否顯示**

在 `src/components/AppImageIntegrationPrompt.tsx` 的 `Props` 介面加入：

```ts
  /** 回報這張卡片是否正在顯示，讓 App 能排出角落卡片的優先序。 */
  onOfferingChange?: (offering: boolean) => void;
```

解構出 `onOfferingChange`，並在 `if (!offer || hasUpdate) return null;` **之前**加入：

```tsx
  const offering = offer && !hasUpdate;
  useEffect(() => { onOfferingChange?.(offering); }, [offering, onOfferingChange]);
```

然後把原本的判斷改成用 `offering`：

```tsx
  if (!offering) return null;
```

這是為了讓 `App.tsx` 能仲裁優先序而做的最小改動——三張卡片固定在右下角同一個位置，同時出現會完全重疊。

- [ ] **Step 5: `App.tsx` 持有狀態並仲裁**

在 `src/App.tsx` 的 import 區塊加入：

```ts
import { ClaudeNotifPrompt } from "./components/ClaudeNotifPrompt";
```

在 `AppRoutes` 的 `const { hasUpdate } = useUpdaterContext();` 後面加入：

```tsx
  const [claudeSeen, setClaudeSeen] = useState(false);
  const [appImageOffering, setAppImageOffering] = useState(false);
  // 三張角落卡片固定在右下角同一個位置，同時出現會完全重疊。
  // 優先序：更新 > AppImage > Claude 通知。
  const onClaudeDetected = useCallback(() => setClaudeSeen(true), []);
```

`useCallback` 需要從 react 匯入——把第 1 行的 `import { useEffect, useState } from "react";` 改成 `import { useCallback, useEffect, useState } from "react";`。

把第 61 行改成：

```tsx
        <TerminalApp hasUpdate={hasUpdate} onClaudeDetected={onClaudeDetected} />
```

把第 74 行改成：

```tsx
      <AppImageIntegrationPrompt hasUpdate={hasUpdate} onOfferingChange={setAppImageOffering} />
      <ClaudeNotifPrompt claudeSeen={claudeSeen} blocked={hasUpdate || appImageOffering} />
```

- [ ] **Step 6: 型別檢查與全套測試**

Run: `npx tsc -b && npx vitest run`
Expected: 兩者皆 exit 0，包含 `AppImageIntegrationPrompt` 既有的 8 個測試仍然全綠（新 prop 是選用的）。

- [ ] **Step 7: scoped lint**

Run: `npx eslint src/App.tsx src/components/TerminalApp.tsx src/components/TerminalView.tsx src/components/AppImageIntegrationPrompt.tsx src/hooks/useTerminalBlocks.ts`
Expected: 沒有**新增**的問題。先用 `git stash` 量出基準再比對——`TerminalView.tsx` 與 `TerminalApp.tsx` 本來就各有既有問題，不要期待 0。

- [ ] **Step 8: Commit**

```bash
git add src/hooks/useTerminalBlocks.ts src/components/TerminalView.tsx src/components/TerminalApp.tsx src/components/AppImageIntegrationPrompt.tsx src/App.tsx
git commit -m "feat(claude-notif): 把 claude 偵測接到提示卡片"
```

---

## Task 7: 手動驗證

自動測試涵蓋比對規則、設定檔讀寫、卡片渲染。**偵測的接線只有這一步能保護。**

**Files:** 無（純驗證）

- [ ] **Step 1: 備份並清掉現有設定**

```bash
cp ~/.claude/settings.json ~/.claude/settings.json.bak
```

然後手動編輯 `~/.claude/settings.json`，把 `"preferredNotifChannel"` 那一行刪掉。

**先備份再動手**——後面幾步會由 app 寫入這個檔案，出錯時要能還原。

- [ ] **Step 2: 啟動 app**

Run: `npm run tauri:dev`

> 若 port 1420 被占用，先找出殘留的 vite：`lsof -ti :1420`，確認該 PID 的 command 路徑在本專案的 `node_modules` 底下再結束它。關掉 Tauri 視窗不會連帶結束前端 dev server。

- [ ] **Step 3: 驗證卡片會出現**

在終端機分頁執行 `claude`。右下角應出現「讓 Claude Code 在背景分頁提醒你？」卡片。

- [ ] **Step 4: 驗證接受路徑，並確認沒有弄壞設定檔**

1. 按「幫我設定」。
2. 卡片應換成成功訊息，且**含有「請開一個新的終端機分頁」**這句。
3. 檢查檔案：

```bash
diff <(jq -S 'del(.preferredNotifChannel)' ~/.claude/settings.json) <(jq -S 'del(.preferredNotifChannel)' ~/.claude/settings.json.bak)
```

Expected: 沒有輸出——除了新增的那個 key，其他設定一字未改。

4. 再看一次原始檔案，確認 key 的**順序**沒有被字母重排：

```bash
diff <(jq -r 'keys_unsorted[]' ~/.claude/settings.json | grep -v preferredNotifChannel) <(jq -r 'keys_unsorted[]' ~/.claude/settings.json.bak)
```

Expected: 沒有輸出。

- [ ] **Step 5: 驗證設定真的生效（這是整個功能的目的）**

1. **開一個新的終端機分頁**，執行 `claude`。
2. 給它一個需要權限確認的指令，例如「請在這個目錄寫出一個 abc.txt，內容是 test」。
3. 趁它跳出權限提示**之前**切到別的分頁。
4. 側邊欄該分頁應出現**橘色脈動點**。

- [ ] **Step 6: 驗證不會重複打擾**

重開 app，再執行一次 `claude`。卡片**不應**出現（設定已存在，`needs_prompt` 為 false）。

- [ ] **Step 7: 驗證婉拒路徑**

1. 再次移除 `~/.claude/settings.json` 裡的 `preferredNotifChannel`。
2. 重開 app，執行 `claude`，卡片出現後按「不用了」。
3. 重開 app，再執行 `claude` → 卡片**不應**出現（婉拒已記錄）。
4. 驗證完把設定加回去：重新編輯檔案或從 `~/.claude/settings.json.bak` 還原後再跑一次接受路徑。

- [ ] **Step 8: 記錄結果與清理**

把每一步的實際結果寫下來。確認 `~/.claude/settings.json` 最終狀態正確之後刪掉備份：

```bash
rm ~/.claude/settings.json.bak
```

---

## 已知限制（實作時不要試圖「修好」）

- **只涵蓋使用者直接輸入 `claude` 的情況。** 包在腳本裡、透過 alias、或帶環境變數前綴（`FOO=1 claude`）啟動的都偵測不到。漏報的代價只是這次不提示，下次執行照樣有機會——不要為此把單純的字串比對改成 shell 語法解析。
- **設定寫入後不影響已在執行的 claude session。** Claude Code 只在啟動時讀設定。這是限制，也是卡片成功訊息必須明講的原因。
- **只處理使用者層級的 `~/.claude/settings.json`。** 專案層級與企業管理設定不在範圍內。

## 跨平台

`~/.claude/settings.json` 的路徑由 `dirs::home_dir()` 推導（`Cargo.toml:48` 已有 `dirs = "6"` 依賴），三個平台一致。`isClaudeCommand` 的 basename 切分同時處理 `/` 與 `\`，所以 Windows 的完整路徑也認得。`serde_json` 的 `preserve_order` 是純 Rust 行為，無平台差異。

# 命令輸入框歷史灰字建議 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (或 subagent-driven-development) 逐 Task 執行；步驟用 `- [ ]` 追蹤。

**Goal:** 輸入框以灰字顯示最近一筆符合開頭的歷史指令，接受鍵（Tab／→／關閉）可在設定頁選擇。

**Architecture:** Rust config 新增 `suggestion_accept_key`；前端純函式 `findSuggestion` 負責找建議；`WarpInput` 疊一層 overlay 顯示灰字並依設定處理 Tab／→；`TerminalView`／`RemoteTerminalView` 讀 config 傳入。

**Tech Stack:** Tauri 2（Rust、serde、toml）、React 19、Vitest + RTL。

Spec：`docs/superpowers/specs/2026-09-21-input-history-suggestion-design.md`

`cargo` 指令一律 `cd src-tauri && cargo test --workspace --no-fail-fast -- <filter>`，看每一行 `test result:`。

---

### Task 1: Rust config 欄位與 command

**Files:** `src-tauri/src/config/types.rs`、`src-tauri/src/commands/config.rs`、`src-tauri/src/lib.rs`（imports 約 53 行、`generate_handler!` 約 464 行）

- [ ] **Step 1: 寫失敗的測試**（`types.rs` 的 `mod tests` 內）

```rust
#[test]
fn suggestion_accept_key_defaults_to_tab_and_old_configs_without_it_still_parse() {
    assert_eq!(AppConfig::default().suggestion_accept_key, SuggestionAcceptKey::Tab);
    // 舊版 config.toml 沒有這個欄位
    let parsed: AppConfig = toml::from_str("onboarding_done = true\n").unwrap();
    assert_eq!(parsed.suggestion_accept_key, SuggestionAcceptKey::Tab);
}

#[test]
fn suggestion_accept_key_round_trips_through_toml_in_kebab_case() {
    for (key, text) in [
        (SuggestionAcceptKey::Tab, "tab"),
        (SuggestionAcceptKey::Right, "right"),
        (SuggestionAcceptKey::Off, "off"),
    ] {
        let cfg = AppConfig { suggestion_accept_key: key, ..AppConfig::default() };
        let s = toml::to_string_pretty(&cfg).unwrap();
        assert!(s.contains(&format!("suggestion_accept_key = \"{text}\"")), "{s}");
        let back: AppConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.suggestion_accept_key, key);
    }
}
```

- [ ] **Step 2: 確認紅**：`cargo test --workspace --no-fail-fast -- suggestion_accept_key` → 編譯失敗（型別不存在）。這是新型別，編譯失敗即紅。

- [ ] **Step 3: 實作**

`types.rs`：`SubmitShortcut` 之後加

```rust
/// Which key accepts the inline history suggestion in the command input box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SuggestionAcceptKey {
    #[default]
    Tab,
    Right,
    Off,
}
```

`AppConfig` 在 `submit_shortcut` 之後加：

```rust
    /// Which key accepts the inline history suggestion (Tab, →, or off).
    #[serde(default)]
    pub suggestion_accept_key: SuggestionAcceptKey,
```

`Default` impl（約 305 行）加 `suggestion_accept_key: SuggestionAcceptKey::default(),`。
`commands/config.rs`：import 加 `SuggestionAcceptKey`（`config/mod.rs` 若需要就 re-export），並加

```rust
#[tauri::command]
pub fn set_suggestion_accept_key(
    key: SuggestionAcceptKey,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config.update(|cfg| { cfg.suggestion_accept_key = key; }).map_err(|e| e.to_string())
}
```

`lib.rs`：兩處（`use` 列表與 `generate_handler!`）在 `set_submit_shortcut` 旁加 `set_suggestion_accept_key`。

- [ ] **Step 4: 確認綠**：同一指令 → 兩題 ok；再跑完整 `cargo test --workspace --no-fail-fast` 確認沒有其他 `AppConfig { .. }` 字面量因缺欄位而編譯失敗。
- [ ] **Step 5: 變異驗證**：暫時拿掉 `#[serde(default)]` → 「舊 config」那題要紅；還原。
- [ ] **Step 6: Commit** `feat(suggest): suggestion_accept_key config field and setter`

---

### Task 2: 純函式 `findSuggestion`

**Files:** Create `src/lib/commandSuggestion.ts`、`src/lib/commandSuggestion.test.ts`

- [ ] **Step 1: 失敗的測試**

```ts
import { describe, it, expect } from "vitest";
import { findSuggestion } from "./commandSuggestion";

// 歷史順序：舊 → 新（與 WarpInput 的 history 相同）
describe("findSuggestion", () => {
  const history = ["git status", "git stash", "ls -la", "git commit -m x"];

  it("回傳最新一筆符合開頭者的剩餘部分", () => {
    expect(findSuggestion(history, "git s")).toBe("tash"); // git stash 比 git status 新
  });
  it("空輸入不建議", () => expect(findSuggestion(history, "")).toBeNull());
  it("沒有符合的歷史", () => expect(findSuggestion(history, "docker")).toBeNull());
  it("與歷史完全相同時不建議（沒有剩餘部分）", () => {
    expect(findSuggestion(history, "ls -la")).toBeNull();
  });
  it("區分大小寫", () => expect(findSuggestion(history, "Git")).toBeNull());
  it("輸入含換行不建議", () => expect(findSuggestion(history, "git\nst")).toBeNull());
  it("歷史裡含換行的多行指令不會被拿來建議", () => {
    expect(findSuggestion(["echo a\necho b"], "echo")).toBeNull();
  });
  it("較舊但更長的符合者，不會蓋過較新的符合者", () => {
    expect(findSuggestion(["git commit --amend", "git commit"], "git c")).toBe("ommit");
  });
});
```

- [ ] **Step 2: 確認紅**（找不到模組）。
- [ ] **Step 3: 實作**

```ts
/**
 * 行內建議：從歷史（舊 → 新）找最新一筆「以目前輸入開頭、且比它長」的單行指令，
 * 回傳要顯示的剩餘部分；沒有就回 null。區分大小寫。
 * 只處理文字本身的規則；游標位置、選單是否開啟等 UI 條件由呼叫端負責。
 */
export function findSuggestion(history: readonly string[], value: string): string | null {
  if (!value || value.includes("\n")) return null;
  for (let i = history.length - 1; i >= 0; i--) {
    const h = history[i];
    if (h.length > value.length && !h.includes("\n") && h.startsWith(value)) {
      return h.slice(value.length);
    }
  }
  return null;
}
```

- [ ] **Step 4: 綠**；**Step 5: 變異**（把迴圈改成由舊到新 → 「最新一筆」與「較舊更長」兩題要紅）；**Step 6: Commit**。

---

### Task 3: `WarpInput` 灰字與接受鍵

**Files:** `src/components/WarpInput.tsx`、`WarpInput.css`、`WarpInput.test.tsx`；`src/ipc/config.ts`（先加型別）

- [ ] **Step 1: `ipc/config.ts`** 加

```ts
export type SuggestionAcceptKey = "tab" | "right" | "off";
// AppConfig 內、submit_shortcut 之後：
//   suggestion_accept_key: SuggestionAcceptKey;
export const setSuggestionAcceptKey = (key: SuggestionAcceptKey): Promise<void> =>
  invoke("set_suggestion_accept_key", { key });
```

- [ ] **Step 2: 失敗的測試**（`WarpInput.test.tsx` 新增 `describe("WarpInput — 歷史灰字建議")`）。輔助：

```tsx
const HISTORY_KEY = "aiterm-command-history";
function renderWith(suggestionKey: "tab" | "right" | "off", opts: { history?: string[]; onSubmit?: () => void; isCommandRunning?: boolean } = {}) {
  localStorage.setItem(HISTORY_KEY, JSON.stringify(opts.history ?? ["git status", "git stash"]));
  const onSubmit = opts.onSubmit ?? vi.fn();
  render(
    <LocaleProvider>
      <WarpInput onSubmit={onSubmit} sessionId="s1" suggestionKey={suggestionKey} isCommandRunning={opts.isCommandRunning} />
    </LocaleProvider>,
  );
  const ta = screen.getByRole("textbox") as HTMLTextAreaElement;
  return { ta, onSubmit };
}
const ghost = () => document.querySelector(".warp-input-ghost-suggestion");
async function typeValue(ta: HTMLTextAreaElement, text: string) {
  await userEvent.click(ta);
  await userEvent.type(ta, text);
}
```

測試（每題先紅）：
1. `tab` 模式輸入 `git s` → 灰字為 `tash`（最新），textarea 的值仍是 `git s`。
2. 空輸入、無符合、`off` 模式 → 沒有灰字元素。
3. `tab` 模式按 Tab → 值變 `git stash`、不呼叫 `onSubmit`、Tab 事件被 `preventDefault`（用 `fireEvent.keyDown` 回傳值為 false 判斷）。
4. `tab` 模式按 → → 不補上（值不變）。
5. `right` 模式游標在結尾按 → → 補上；Tab → 不補上、不 preventDefault。
6. `right` 模式游標不在結尾（`setSelectionRange(2,2)`）→ 沒有灰字，→ 不補上。
7. `off` 模式 Tab 與 → 皆不補上、不 preventDefault。
8. `tab` 模式沒有建議時 Tab 不 preventDefault。
9. 指令執行中（`isCommandRunning`）→ 沒有灰字。
10. 按 ↑ 開歷史清單後 → 沒有灰字。
11. 輸入法組字中（`compositionStart`）→ 沒有灰字。
12. 送出（Enter）後灰字消失、`onSubmit` 收到的是輸入的原文（不含灰字）。

- [ ] **Step 3: 確認紅**（`suggestionKey` 不是 prop → 沒有灰字元素）。
- [ ] **Step 4: 實作**

`WarpInputProps` 加 `suggestionKey?: SuggestionAcceptKey`（缺省 `"off"`）。元件內：

```tsx
const [caretAtEnd, setCaretAtEnd] = useState(true);
const [composing, setComposing] = useState(false);
const suggestion =
  suggestionKey !== "off" && caretAtEnd && !composing && !historyOpen && !dirPickerOpen &&
  !disabled && !isCommandRunning
    ? findSuggestion(history, value)
    : null;

const syncCaret = () => {
  const el = textareaRef.current;
  if (el) setCaretAtEnd(el.selectionStart === el.selectionEnd && el.selectionEnd === el.value.length);
};

const acceptSuggestion = () => {
  if (!suggestion) return;
  fillInput(value + suggestion);
  requestAnimationFrame(() => {
    const el = textareaRef.current;
    if (el) { el.selectionStart = el.selectionEnd = el.value.length; }
    setCaretAtEnd(true);
  });
};
```

`handleKeyDown`：緊接 `isImeComposing` 檢查與 `isCommandRunning` 轉發區塊**之後**、`let shouldSubmit` 之前加：

```tsx
if (suggestion && !historyOpen && !dirPickerOpen) {
  if (e.key === "Tab" && suggestionKey === "tab" && !e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey) {
    e.preventDefault();
    acceptSuggestion();
    return;
  }
  if (e.key === "ArrowRight" && suggestionKey === "right" && !e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey) {
    e.preventDefault();
    acceptSuggestion();
    return;
  }
}
```

textarea 加 `onSelect={syncCaret}`、`onKeyUp={syncCaret}`、`onClick={syncCaret}`、`onCompositionStart={() => setComposing(true)}`、`onCompositionEnd={() => setComposing(false)}`（先確認 `isImeComposing` 與現有測試如何模擬組字，沿用其事件）。`handleChange` 內也呼叫 `syncCaret()`（輸入後游標在結尾）。

JSX：把 `<textarea>` 包進

```tsx
<div className="warp-input-field">
  {suggestion && (
    <div className="warp-input-ghost" aria-hidden="true">
      <span className="warp-input-ghost-typed">{value}</span>
      <span className="warp-input-ghost-suggestion">{suggestion}</span>
    </div>
  )}
  <textarea ... />
</div>
```

`WarpInput.css`：

```css
.warp-input-field { position: relative; flex: 1; display: flex; min-width: 0; }
.warp-input-field .warp-input-textarea { position: relative; flex: 1; min-width: 0; }
.warp-input-ghost {
  position: absolute; inset: 0; pointer-events: none; overflow: hidden;
  font-family: inherit; font-size: 14px; line-height: 1.4;
  white-space: pre-wrap; word-break: break-word; overflow-wrap: anywhere;
}
.warp-input-ghost-typed { visibility: hidden; }
.warp-input-ghost-suggestion { color: #6b6b6b; }
```

（textarea 原本的 `flex: 1` 保留；overlay 的換行規則要與 textarea 一致——textarea 預設 `white-space: pre-wrap; overflow-wrap: break-word`，實作時以真機截圖確認兩者換行位置相同，不一致就調整。）

- [ ] **Step 5: 綠 + 既有 WarpInput 測試全綠**。
- [ ] **Step 6: 變異驗證**：(a) 把接受條件的 `suggestionKey === "tab"` 改成永遠 true → 「`right` 模式 Tab 不接受」要紅；(b) 拿掉 `caretAtEnd` 條件 → 第 6 題紅；還原。
- [ ] **Step 7: Commit**。

---

### Task 4: 設定頁

**Files:** `src/components/Settings/GeneralPage.tsx`、`src/lib/i18n.ts`；測試沿用該頁既有測試檔（若沒有則新增最小掛載測試，參考 `reference-frontend-test-mounting`）

- [ ] **Step 1: i18n**（zh-TW 與 en 各 8 個 key，插在 `submit_shortcut_desc` 之後；en 缺 key 會靜默 fallback，完成後 `grep -c "suggest_key_" src/lib/i18n.ts` 應為 16）

```
suggest_key: "行內建議的接受鍵" / "Accept-suggestion Key"
suggest_key_desc: "輸入時，輸入框會用灰字顯示最近一筆符合開頭的歷史指令，按下這個鍵補上（不會自動送出）。" / "While you type, the input shows the most recent matching history command in grey; press this key to accept it (it is not submitted automatically)."
suggest_key_tab_label: "Tab" / "Tab"
suggest_key_tab_desc: "按 Tab 補上建議；沒有建議時 Tab 維持原本行為。" / ...
suggest_key_right_label: "→（向右鍵）" / "→ (Right arrow)"
suggest_key_right_desc: "游標在結尾時按 → 補上建議；否則 → 照常移動游標。" / ...
suggest_key_off_label: "關閉建議" / "Off"
suggest_key_off_desc: "不顯示灰字建議。" / "Do not show inline suggestions."
```

- [ ] **Step 2: 失敗的測試**：載入時 `getConfig` 回 `suggestion_accept_key: "right"` → 「→」單選被選取；缺值 → 「Tab」被選取；點「關閉建議」→ `setSuggestionAcceptKey("off")` 被呼叫。
- [ ] **Step 3: 實作**：`GeneralPage` 加 state、載入時 `setSuggestKey(cfg.suggestion_accept_key ?? "tab")`、`handleSuggestKeyChange`（仿 `handleShortcutChange`）、在「輸入組合鍵」`</section>` 之後加一段 `mode-list` 單選（`name="suggestion_accept_key"`）。
- [ ] **Step 4: 綠；Step 5: i18n 計數；Step 6: Commit**。

---

### Task 5: 接線

**Files:** `src/components/TerminalView.tsx`（約 244、1035、2479 行）、`src/components/RemoteTerminalView/index.tsx`（約 172、785 行）

- [ ] `TerminalView`：`const [suggestionKey, setSuggestionKeyState] = useState<SuggestionAcceptKey>("tab");`；`refreshConfig()` 的 `getConfig().then` 內加 `setSuggestionKeyState(cfg.suggestion_accept_key ?? "tab");`；`<WarpInput ... suggestionKey={suggestionKey} />`。
- [ ] `RemoteTerminalView`：同樣的 state；既有 `getConfig().then` 內加同一行；`<WarpInput ... suggestionKey={suggestionKey} />`。
- [ ] 測試：延伸既有的 TerminalView 掛載測試（`getConfig` mock 回 `suggestion_accept_key: "off"`）——確認傳到輸入框（輸入框無灰字）；若既有 mock 走 `invoke` 永不 resolve，改為新增一個小測試 mock `../ipc/config`。先讀該檔再決定，不要憑猜測寫。
- [ ] `npx tsc -b`、Commit。

---

### Task 6: 全面驗證與真機驗收

- [ ] `cd src-tauri && cargo test --workspace --no-fail-fast`（每個 `test result:` 皆 ok）、`npx tsc -b`、`npm run lint`（changed files 不得新增錯誤，與 master 比對）、`npm run test`。
- [ ] 真機（隔離 `tauri dev`，`env -u APPLE_SIGNING_IDENTITY`）：送出 `echo hello-world-test` → 輸入 `echo hel` 看到灰字；Tab 接受；設定頁切到 → 後 → 接受、Tab 不接受；切到關閉後無灰字；長字串換行時灰字與輸入對齊（截圖）。
- [ ] 更新 spec 驗收結果與未驗證項目；CHANGELOG 由使用者決定發版時再寫（本輪不 push、不打 tag）。

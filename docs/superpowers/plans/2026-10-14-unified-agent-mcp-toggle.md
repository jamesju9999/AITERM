# ⚡ 與 MCP 開關合併成一套工具迴圈 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ask AI 的 ⚡ 決定「AI 能不能自己動手」、MCP 決定「動手時能不能用 MCP／電腦操作工具」；⚡ 關＝純聊天。舊的 `<cmd>` Agent 迴圈拿掉，全部走 `useMcpChat` 的工具迴圈。

**Architecture:** 後端 `ai_chat` 加選用參數 `tool_scope`（`none | terminal | all`），沒帶時沿用 `use_mcp`，其他呼叫端不受影響。前端 `useMcpChat` 改收 scope、可注入額外 system 提示詞、對外提供步數／階段／目前 scope；`AiPanel` 刪掉自己的迴圈，⚡ 送 `terminal` 或 `all`，⚡ 關送 `none`。安全沿用 `terminal_command_gate`（完全照設定的執行模式，不改）。

**Tech Stack:** Rust（Tauri 2 command）、React 19 + TypeScript、Vitest + RTL、cargo test。

**Spec:** `docs/superpowers/specs/2026-10-14-unified-agent-mcp-toggle-design.md`

**通用規則（每個 task 都適用）：**
- 每個新測試寫完先跑、**親眼看到它紅**，再寫實作。若一寫就綠，代表測試資料分辨不出對錯——停下來修測試，不要繼續。
- 不可為了讓測試好寫而扭曲實作；有疑慮就停下來回報。
- `git add` 只加本 task 列出的路徑，不要 `git add -A`。`docs/` 被 gitignore，加文件要 `git add -f`。
- 本 repo 有 CRLF 檔案：改完 `git diff --stat` 看行數，暴增代表行尾被換掉。

---

## 檔案地圖

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src-tauri/src/commands/ai.rs` | 改 | `ToolScope`、`resolve_tool_scope`、`scoped_tool_infos`；`ai_chat` 依 scope 選工具與提示詞 |
| `src/ipc/ai.ts` | 改 | `ToolScope` 型別；`aiChat` 多一個選用參數 `toolScope` |
| `src/hooks/useMcpChat.ts` | 改 | `send(text, scope, …, extraSystemPrompt)`；`step`／`phase`／`activeScope` |
| `src/hooks/useMcpChat.test.ts`、`useMcpChat.streamDup.test.tsx` | 改 | 呼叫改傳 scope；新增 scope／system／步數測試 |
| `src/components/AiPanel/ModeHint.tsx`（＋test） | 改 | `PanelMode` 加 `agent-terminal`／`agent-mcp`，拿掉 `mcp` |
| `src/lib/i18n.ts` | 改 | 新增兩個模式說明字串（中英），移除 `mode_hint_mcp` |
| `src/components/ChatPanel/styles.css` | 改 | 模式說明列的色條 class |
| `src/components/AiPanel/index.tsx` | 改 | 刪舊迴圈；⚡／MCP 按鈕邏輯；卡住偵測搬進 `runTerminalCommand`；統一停止 |
| `src/components/AiPanel/AiPanel.test.tsx`、`stuckPrompt.test.tsx` | 改 | Agent 測試改用 `run_command` 工具呼叫；新增 scope 與按鈕測試 |
| `CHANGELOG.md` | 改 | 新版段落（寫給使用者） |
| spec | 改 | 補記實作時的兩個決定 |

---

### Task 1: Rust — `ToolScope` 與兩個純函式

**Files:**
- Modify: `src-tauri/src/commands/ai.rs`（在 `with_tool_use_rule` 之後新增；檔尾新增測試模組）

- [ ] **Step 1: 寫會紅的測試**

在 `src-tauri/src/commands/ai.rs` 檔尾新增：

```rust
#[cfg(test)]
mod tool_scope_tests {
    use super::{resolve_tool_scope, scoped_tool_infos, ToolScope};
    use crate::mcp::McpToolInfo;

    fn info(name: &str) -> McpToolInfo {
        McpToolInfo {
            server_id: "s".into(),
            server_name: "S".into(),
            name: name.into(),
            description: String::new(),
            input_schema: serde_json::json!({}),
        }
    }

    // 沒帶 tool_scope 的舊呼叫端：行為必須和改之前一模一樣（use_mcp 且全域 MCP 開才給工具）。
    #[test]
    fn legacy_callers_follow_use_mcp_and_global_switch() {
        assert_eq!(resolve_tool_scope(None, true, true), ToolScope::All);
        assert_eq!(resolve_tool_scope(None, true, false), ToolScope::None);
        assert_eq!(resolve_tool_scope(None, false, true), ToolScope::None);
    }

    // ⚡＋MCP 但全域 MCP 關閉：⚡ 照樣要能跑指令，退成只有終端機工具。
    #[test]
    fn explicit_all_falls_back_to_terminal_when_mcp_is_disabled() {
        assert_eq!(resolve_tool_scope(Some(ToolScope::All), false, false), ToolScope::Terminal);
        assert_eq!(resolve_tool_scope(Some(ToolScope::All), false, true), ToolScope::All);
    }

    // 明確指定的 scope 不看 use_mcp，也不看全域開關（terminal／none 不受 MCP 設定影響）。
    #[test]
    fn explicit_terminal_and_none_ignore_use_mcp_and_global_switch() {
        assert_eq!(resolve_tool_scope(Some(ToolScope::Terminal), true, false), ToolScope::Terminal);
        assert_eq!(resolve_tool_scope(Some(ToolScope::None), true, true), ToolScope::None);
    }

    #[test]
    fn tool_lists_per_scope() {
        let mcp = vec![info("srv__read_file")];
        let computer = vec![info("aiterm_computer__computer_click")];

        assert!(scoped_tool_infos(ToolScope::None, mcp.clone(), computer.clone()).is_empty());

        let terminal: Vec<String> = scoped_tool_infos(ToolScope::Terminal, mcp.clone(), computer.clone())
            .into_iter().map(|t| t.name).collect();
        assert_eq!(terminal, vec!["aiterm_terminal__run_command".to_string()]);

        let all: Vec<String> = scoped_tool_infos(ToolScope::All, mcp, computer)
            .into_iter().map(|t| t.name).collect();
        assert_eq!(all, vec![
            "srv__read_file".to_string(),
            "aiterm_computer__computer_click".to_string(),
            "aiterm_terminal__run_command".to_string(),
        ]);
    }

    // 前端送的是小寫字串。
    #[test]
    fn deserializes_lowercase() {
        let s: ToolScope = serde_json::from_str("\"terminal\"").unwrap();
        assert_eq!(s, ToolScope::Terminal);
    }
}
```

- [ ] **Step 2: 確認它紅（編譯失敗）**

Run: `cd src-tauri && cargo test -p app --lib tool_scope_tests 2>&1 | tail -20`
Expected: 編譯錯誤 `cannot find type ToolScope` / `cannot find function resolve_tool_scope`。

- [ ] **Step 3: 實作**

在 `with_tool_use_rule` 函式之後新增：

```rust
/// Ask AI 這一輪給模型哪些工具（spec 2026-10-14-unified-agent-mcp-toggle）。
/// ⚡ 關＝None（純聊天）；只有 ⚡＝Terminal；⚡＋MCP＝All。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolScope {
    None,
    Terminal,
    All,
}

/// 沒帶 `tool_scope` 的呼叫端（Ask AI 以外的十幾個畫面）完全沿用舊規則：`use_mcp` 且全域 MCP 開才給工具。
/// 明確要 All 但全域 MCP 關閉時退成 Terminal——⚡ 不能因為 MCP 被關就跑不了指令。
pub(crate) fn resolve_tool_scope(requested: Option<ToolScope>, use_mcp: bool, mcp_enabled: bool) -> ToolScope {
    match requested {
        Some(ToolScope::All) if !mcp_enabled => ToolScope::Terminal,
        Some(scope) => scope,
        None if use_mcp && mcp_enabled => ToolScope::All,
        None => ToolScope::None,
    }
}

/// 依 scope 組出要給模型的工具清單。順序：MCP 伺服器、電腦操作、終端機（與改之前相同）。
pub(crate) fn scoped_tool_infos(
    scope: ToolScope,
    mcp: Vec<crate::mcp::McpToolInfo>,
    computer: Vec<crate::mcp::McpToolInfo>,
) -> Vec<crate::mcp::McpToolInfo> {
    match scope {
        ToolScope::None => vec![],
        ToolScope::Terminal => crate::terminal_tool::tool_infos(),
        ToolScope::All => {
            let mut infos = mcp;
            infos.extend(computer);
            infos.extend(crate::terminal_tool::tool_infos());
            infos
        }
    }
}
```

若 `crate::mcp::McpToolInfo` 路徑不對，看 `src-tauri/src/commands/mcp.rs` 頂端怎麼 import `McpToolInfo`，用同一個路徑。

- [ ] **Step 4: 確認綠**

Run: `cd src-tauri && cargo test -p app --lib tool_scope_tests 2>&1 | grep "test result"`
Expected: `test result: ok. 5 passed`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/ai.rs
git commit -m "feat(ai): ToolScope (none/terminal/all) with legacy-compatible resolution

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Rust — `ai_chat` 依 scope 選工具與提示詞

**Files:**
- Modify: `src-tauri/src/commands/ai.rs`（`ai_chat`，約 531–592 行）

這一段是把 Task 1 的純函式接上；行為由 Task 1 的單元測試與 Task 6 的前端測試覆蓋，這裡不另寫測試。

- [ ] **Step 1: 改簽章**

`ai_chat` 參數列在 `supports_artifacts: bool,` 之後加：

```rust
    tool_scope: Option<ToolScope>,
```

- [ ] **Step 2: 先算 scope，再檢查最後一則訊息的角色**

把「Reject empty history…」那段到 `let cfg = config.get();` 改成（`cfg` 提前讀）：

```rust
    if messages.is_empty() {
        return Err(AiError::InvalidInput { reason: "empty messages".into() });
    }
    let cfg = config.get();
    let scope = resolve_tool_scope(tool_scope, use_mcp, cfg.mcp_enabled);
    // 有工具的迴圈可能以 "tool" 結果訊息結尾（多輪工具呼叫），所以 scope 不是 None 時也放行。
    let last_role = messages.last().map(|m| m.role.as_str());
    let last_role_ok = last_role == Some("user")
        || (scope != ToolScope::None && last_role == Some("tool"));
    if !last_role_ok {
        return Err(AiError::InvalidInput { reason: "last message must be from user".into() });
    }

    let snapshot = context::snapshot(&pty_manager, &session_id);
```

並刪掉原本 `// ── MCP tool calling path` 下面那行重複的 `let cfg = config.get();`。

- [ ] **Step 3: 工具清單與提示詞依 scope**

把 `if use_mcp && cfg.mcp_enabled {` 到 `let prompt = format!("{prompt}\n\n{}", crate::terminal_tool::PROMPT_HINT);` 改成：

```rust
    if scope != ToolScope::None {
        let tools: Vec<crate::ai::McpToolDefinition> = {
            let (mcp_infos, computer_infos) = if scope == ToolScope::All {
                (mcp_manager.lock().await.list_tool_infos(), crate::computer::builtin_tool_infos(&cfg))
            } else {
                (vec![], vec![])
            };
            scoped_tool_infos(scope, mcp_infos, computer_infos)
                .into_iter()
                .map(|t| crate::ai::McpToolDefinition {
                    name: t.name,
                    description: t.description,
                    input_schema: t.input_schema,
                })
                .collect()
        };

        if !tools.is_empty() {
            let provider = match provider_id.as_deref() {
                Some(id) => router.resolve_by_id(id).await?,
                None => router.resolve().await?,
            };
            // 電腦操作心法只在真的給了電腦操作工具時附上（只有 ⚡ 時沒有）。
            let prompt = with_computer_hint(
                with_tool_use_rule(build_chat_prompt(&snapshot, locale, supports_artifacts)),
                scope == ToolScope::All && cfg.mcp_tool_server.computer_enabled,
            );
            let prompt = format!("{prompt}\n\n{}", crate::terminal_tool::PROMPT_HINT);
```

其餘（`GenerateRequest` 之後到 `// ── End MCP path`）不動。

- [ ] **Step 4: 編譯與全套 Rust 測試**

Run: `cd src-tauri && cargo test --workspace --no-fail-fast 2>&1 | grep -E "test result|error\[|FAILED|panicked"`
Expected: 每一行 `test result:` 都是 `ok`，沒有 `error[`。若有既有測試直接呼叫 `ai_chat(...)` 而缺參數，補上 `None`。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/ai.rs
git commit -m "feat(ai): ai_chat picks tools and prompt by tool_scope

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: IPC — `aiChat` 帶 `toolScope`

**Files:**
- Modify: `src/ipc/ai.ts`（`aiChat`，約 131 行）

- [ ] **Step 1: 實作**

在 `aiChat` 上方加型別，並改 `aiChat`：

```ts
/** Ask AI 這一輪給模型哪些工具：⚡ 關＝none、只有 ⚡＝terminal、⚡＋MCP＝all。見 Rust `ToolScope`。 */
export type ToolScope = "none" | "terminal" | "all";

export const aiChat = (
  messages: ChatMessage[],
  sessionId: string,
  providerId?: string,
  useMcp = false,
  locale: Locale = "zh-TW",
  supportsArtifacts = false,
  toolScope?: ToolScope,
): Promise<AiChatReply> =>
  invoke("ai_chat", {
    messages,
    sessionId,
    providerId: providerId ?? null,
    useMcp,
    locale,
    supportsArtifacts,
    // 只有指定時才帶：其他呼叫端的 payload 維持原樣（有測試比對整個 payload）。
    ...(toolScope ? { toolScope } : {}),
  });
```

保留 `aiChat` 原本的其他部分（若原本結尾還有別的欄位，照原樣留著）。

- [ ] **Step 2: 型別檢查與既有測試**

Run: `npx tsc -b && npx vitest run src/ipc/ai.test.ts`
Expected: tsc 無輸出；測試全綠。

- [ ] **Step 3: Commit**

```bash
git add src/ipc/ai.ts
git commit -m "feat(ipc): aiChat accepts an optional toolScope

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: `useMcpChat` — scope、額外 system 提示詞、步數／階段

**Files:**
- Modify: `src/hooks/useMcpChat.ts`
- Modify: `src/hooks/useMcpChat.test.ts`、`src/hooks/useMcpChat.streamDup.test.tsx`

- [ ] **Step 1: 既有測試改傳 scope**

兩個測試檔裡所有 `sendMessage(…, true` 改成 `sendMessage(…, "all"`，`sendMessage(…, false` 改成 `sendMessage(…, "none"`（`.send(` 同理）。用編輯器逐一改，改完：

Run: `grep -nE "(sendMessage|\.send)\([^)]*, (true|false)" src/hooks/useMcpChat.test.ts src/hooks/useMcpChat.streamDup.test.tsx`
Expected: 沒有輸出。

- [ ] **Step 2: 寫會紅的新測試**

在 `src/hooks/useMcpChat.test.ts` 的 `describe("useMcpChat", …)` 內最後加：

```ts
  describe("⚡ 合併後的工具範圍（spec 2026-10-14）", () => {
    const say = (content: string) => ({ content, tool_calls: [], tool_calling_unsupported: false });
    const runCmd = (command: string) => ({
      content: null,
      tool_calls: [{ id: "t1", tool_name: "aiterm_terminal__run_command", server_id: "aiterm-terminal", args: { command } }],
      tool_calling_unsupported: false,
    });

    it("把 scope 傳給後端（第 7 個參數）", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValueOnce(say("好"));
      const { result } = renderHook(() => useMcpChat("s-scope"), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("hi", "terminal"); });
      expect(vi.mocked(aiChat).mock.calls[0][6]).toBe("terminal");
    });

    it("額外的 system 提示詞放在送給模型的最前面，但不出現在對話裡", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValueOnce(say("好"));
      const { result } = renderHook(() => useMcpChat("s-sys"), { wrapper: LocaleProvider });
      await act(async () => {
        await result.current.sendMessage("hi", "terminal", undefined, undefined, "REMOTE-NOTE");
      });
      const sent = vi.mocked(aiChat).mock.calls[0][0];
      expect(sent[0]).toEqual({ role: "system", content: "REMOTE-NOTE" });
      expect(sent.at(-1)).toEqual({ role: "user", content: "hi" });
      expect(result.current.messages.some((m) => String(m.content).includes("REMOTE-NOTE"))).toBe(false);
    });

    it("執行工具時 phase 是 running、步數是 1，結束後 activeScope 回到 null", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(terminalCommandGate).mockResolvedValue({ allowed: true, reason: null });
      vi.mocked(aiChat).mockResolvedValueOnce(runCmd("ls")).mockResolvedValueOnce(say("完成"));
      let finishCommand: (out: string) => void = () => {};
      const runCommand = vi.fn(() => new Promise<string>((r) => { finishCommand = r; }));
      const { result } = renderHook(() => useMcpChat("s-phase", 10, runCommand), { wrapper: LocaleProvider });

      let done: Promise<void> = Promise.resolve();
      act(() => { done = result.current.sendMessage("列檔案", "terminal"); });
      await vi.waitFor(() => expect(runCommand).toHaveBeenCalledWith("ls"));
      expect(result.current.activeScope).toBe("terminal");
      expect(result.current.phase).toBe("running");
      expect(result.current.step).toBe(1);

      await act(async () => { finishCommand("ok"); await done; });
      expect(result.current.activeScope).toBeNull();
      expect(result.current.step).toBe(2);
    });

    it("純聊天（none）不催模型動手", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValueOnce(say("讓我先取得 UI 快照。"));
      const { result } = renderHook(() => useMcpChat("s-none"), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("hi", "none"); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(1);
    });
  });
```

`terminalCommandGate` 的回傳型別若不是 `{ allowed, reason }`，看 `src/ipc/computer.ts` 照實際型別寫。

- [ ] **Step 3: 確認紅**

Run: `npx vitest run src/hooks/useMcpChat.test.ts`
Expected: tsc 型別錯誤或執行失敗——`sendMessage` 第二個參數型別不符、`activeScope`／`phase`／`step` 不存在、`mock.calls[0][6]` 是 `undefined`。

- [ ] **Step 4: 實作**

`src/hooks/useMcpChat.ts`：

1. import 改成：

```ts
import { abortAi, aiChat, formatAiError, type AiToolCall, type AiError, type ToolFallbackReason, type ToolScope } from "../ipc/ai";
```

2. `lastSendRef` 型別改成：

```ts
  const lastSendRef = useRef<{ text: string; scope: ToolScope; attachments?: Attachment[]; extraSystemPrompt?: string } | null>(null);
```

3. 在 `const [error, setError] …` 之後加：

```ts
  // ⚡ 模式的狀態列要顯示「第幾步、在等 AI 還是在等指令」；activeScope 是進行中那一輪的範圍，沒在跑時是 null。
  const [activeScope, setActiveScope] = useState<ToolScope | null>(null);
  const [step, setStep] = useState(0);
  const [phase, setPhase] = useState<"thinking" | "running">("thinking");
```

4. `sendMessage` 簽章與開頭改成：

```ts
  const sendMessage = useCallback(async (
    text: string,
    scope: ToolScope,
    existingMessages?: McpChatMessage[],
    attachments?: Attachment[],
    /** 只送給模型、不顯示也不存的 system 提示詞（⚡ 模式的提權規則、遠端連線說明）。 */
    extraSystemPrompt?: string,
  ) => {
    if (!text.trim() && (!attachments || attachments.length === 0)) return;
    lastSendRef.current = { text, scope, attachments, extraSystemPrompt };
```

並在 `setError(null);` 之後加：

```ts
    setActiveScope(scope);
    setStep(0);
    setPhase("thinking");
```

5. `let iterHistory = [...history];` 改成：

```ts
      let iterHistory: ChatMessage[] = extraSystemPrompt
        ? [{ role: "system", content: extraSystemPrompt }, ...history]
        : [...history];
```

6. 迴圈內 `iterations++;` 之後、`const reply = …` 改成：

```ts
        iterations++;
        setStep(iterations);
        setPhase("thinking");
        const reply = await aiChat(iterHistory, sessionId, undefined, scope !== "none", locale, true, scope);
```

7. `if (reply.tool_calls.length > 0) {` 下一行加 `setPhase("running");`。

8. 催促條件 `if (useMcp && nudges < …` 改成 `if (scope !== "none" && nudges < …`。

9. `finally` 裡 `if (!stale()) {` 區塊內加 `setActiveScope(null);`。

10. `stop` 裡 `setIsLoading(false);` 之前加 `setActiveScope(null);`。

11. `resend` 改成：

```ts
    const { text, scope, attachments, extraSystemPrompt } = lastSendRef.current;
    …
    await sendMessage(text, scope, trimmedMessages, attachments, extraSystemPrompt);
```

12. return 物件加 `activeScope, step, phase,`。

- [ ] **Step 5: 確認綠**

Run: `npx vitest run src/hooks/useMcpChat.test.ts src/hooks/useMcpChat.streamDup.test.tsx`
Expected: 全綠。（`npx tsc -b` 此時會在 `AiPanel/index.tsx` 報錯——Task 6 處理，這裡先不管。）

- [ ] **Step 6: 突變驗證**

逐一做以下修改，各跑一次 Step 5 的指令確認有測試紅，再還原（**用 `cp` 先備份檔案、還原時 `cp` 回來，不要用 `git checkout`**——會洗掉未提交的修改）：
- 把 `aiChat(…, scope)` 的最後一個參數拿掉 → 「把 scope 傳給後端」應紅。
- 把 `setPhase("running")` 拿掉 → 「phase 是 running」應紅。
- 催促條件改回 `true &&` → 「純聊天不催」應紅。

- [ ] **Step 7: Commit**

```bash
git add src/hooks/useMcpChat.ts src/hooks/useMcpChat.test.ts src/hooks/useMcpChat.streamDup.test.tsx
git commit -m "feat(chat): useMcpChat takes a tool scope, extra system prompt, and reports step/phase

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: 模式說明列 — `agent-terminal`／`agent-mcp`

**Files:**
- Modify: `src/components/AiPanel/ModeHint.tsx`、`src/components/AiPanel/ModeHint.test.tsx`
- Modify: `src/lib/i18n.ts`（zh-TW 約 1334–1338 行、en 約 2944–2947 行）
- Modify: `src/components/ChatPanel/styles.css`（約 445–446 行）

遠端終端機的 `RemoteAiPanel` 仍用舊的 `<cmd>` 迴圈與 `mode="agent"`，所以 `agent` 這個值與 `mode_hint_agent` 字串**保留不動**；本機改用兩個新值。

- [ ] **Step 1: 改測試（會紅）**

`ModeHint.test.tsx` 中，把兩個 `mode="mcp"` 的測試整個換成：

```tsx
  it("只有 ⚡：說明會自己在終端機執行、帶步數、提到執行模式、說明沒開 MCP", () => {
    render(<ModeHint mode="agent-terminal" maxAgentSteps={700} mcpToolCount={20} />);
    expect(screen.getByText(/終端機執行指令/)).toBeInTheDocument();
    expect(screen.getByText(/700 步/)).toBeInTheDocument();
    expect(screen.getByText(/執行模式/)).toBeInTheDocument();
    expect(screen.getByText(/未開啟 MCP/)).toBeInTheDocument();
  });

  it("⚡＋MCP：帶出工具數量與步數，提到執行模式", () => {
    render(<ModeHint mode="agent-mcp" maxAgentSteps={9999} mcpToolCount={20} />);
    expect(screen.getByText(/20 個 MCP 工具/)).toBeInTheDocument();
    expect(screen.getByText(/∞ 步/)).toBeInTheDocument();
    expect(screen.getByText(/執行模式/)).toBeInTheDocument();
  });
```

其餘 `mode="agent"`、`mode="suggest"` 的測試保留。

Run: `npx vitest run src/components/AiPanel/ModeHint.test.tsx`
Expected: 新的兩條紅（型別不符／找不到文字）。

- [ ] **Step 2: i18n**

zh-TW 段：刪掉 `mode_hint_mcp`，在 `mode_hint_agent` 之後加：

```ts
    mode_hint_agent_terminal: (steps: string) =>
      `AI 會自己在終端機執行指令並看輸出迭代，最多 ${steps} 步（依「執行模式」可能先請你確認；未開啟 MCP 工具）`,
    mode_hint_agent_mcp: (steps: string, n: number) =>
      `AI 會自己執行指令，也能呼叫 ${n} 個 MCP 工具，最多 ${steps} 步（依「執行模式」可能先請你確認）`,
```

en 段：刪掉 `mode_hint_mcp`，加：

```ts
    mode_hint_agent_terminal: (steps: string) =>
      `AI runs commands in the terminal itself and iterates on the output, up to ${steps} steps (you may be asked to confirm, depending on the execution mode; MCP tools are off)`,
    mode_hint_agent_mcp: (steps: string, n: number) =>
      `AI runs commands itself and can call ${n} MCP tools, up to ${steps} steps (you may be asked to confirm, depending on the execution mode)`,
```

確認沒有其他地方用 `mode_hint_mcp`：`grep -rn mode_hint_mcp src` 應無輸出。

- [ ] **Step 3: ModeHint.tsx**

```tsx
/** 「誰按下執行鍵、能用什麼工具」。agent 是遠端終端機（舊的 <cmd> 迴圈）用的；本機用 agent-terminal／agent-mcp。 */
export type PanelMode = "suggest" | "agent" | "agent-terminal" | "agent-mcp";

const ICONS: Record<PanelMode, string> = {
  suggest: "💬",
  agent: "⚡",
  "agent-terminal": "⚡",
  "agent-mcp": "⚡",
};
```

本體的判斷改成：

```tsx
  const steps = maxAgentSteps >= 9999 ? "∞" : String(maxAgentSteps);
  let text: string;
  if (mode === "agent") {
    text = t.mode_hint_agent(steps);
  } else if (mode === "agent-terminal") {
    text = t.mode_hint_agent_terminal(steps);
  } else if (mode === "agent-mcp") {
    text = t.mode_hint_agent_mcp(steps, mcpToolCount);
  } else {
    text = t.mode_hint_suggest;
  }
```

（保留原本「9999 是無限的內部值」那條註解，移到 `steps` 那行上面。）

- [ ] **Step 4: CSS**

`styles.css` 的 `.aiterm-mode-hint--mcp { … }` 那行換成：

```css
.aiterm-mode-hint--agent-terminal { border-left-color: var(--accent, #a855f7); }
.aiterm-mode-hint--agent-mcp { border-left-color: var(--success, #34d399); }
```

- [ ] **Step 5: 確認綠**

Run: `npx vitest run src/components/AiPanel/ModeHint.test.tsx src/lib/i18n.test.ts src/lib/i18n.remoteTerminal.test.ts`
Expected: 全綠。

- [ ] **Step 6: Commit**

```bash
git add src/components/AiPanel/ModeHint.tsx src/components/AiPanel/ModeHint.test.tsx src/lib/i18n.ts src/components/ChatPanel/styles.css
git commit -m "feat(ask-ai): mode hints for agent with terminal only and agent with MCP

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: `AiPanel` — 刪舊迴圈、接上新模式

**Files:**
- Modify: `src/components/AiPanel/index.tsx`
- Modify: `src/components/AiPanel/AiPanel.test.tsx`、`src/components/AiPanel/stuckPrompt.test.tsx`

- [ ] **Step 1: 測試基礎設施**

兩個測試檔的 `invoke` mock 都要：
- `aiChatQueue` 型別的 `content` 改成 `string | null`。
- 在 `if (cmd === "ai_chat")` 之前加：`if (cmd === "terminal_command_gate") return Promise.resolve({ allowed: true, reason: null });`
- `AiPanel.test.tsx`：新增 `const aiChatScopes: (string | undefined)[] = [];`，在 `ai_chat` 分支記錄 `aiChatScopes.push((payload as { toolScope?: string } | undefined)?.toolScope);`（payload 型別加上 `toolScope?: string`），`beforeEach` 清空它。
- 兩個檔的 `beforeEach` 加 `localStorage.clear();`（⚡／MCP 開關存在 localStorage，測試之間會互相汙染）。

兩個檔都加一個共用的工具呼叫產生器：

```ts
const runCmdCall = (command: string) => ({
  content: null,
  tool_calls: [{ id: `c-${command}`, tool_name: "aiterm_terminal__run_command", server_id: "aiterm-terminal", args: { command } }],
});
```

- [ ] **Step 2: 改寫既有 Agent 測試（AiPanel.test.tsx）**

- 「Agent Mode recurses to a second AI call after executing a <cmd>」：`aiChatQueue.push({ content: "<cmd>ls</cmd>" })` 改成 `aiChatQueue.push(runCmdCall("ls"))`；標題改成「⚡ 執行 run_command 後把結果交回 AI 再問一次」。最後一段斷言保留（第二次呼叫的歷史含 `ls`）。
- 「Agent 狀態列要區分「思考中」與「執行指令中」」：同樣改成 `runCmdCall("ls")`。
- 「Agent Mode surfaces the error and stops when the follow-up AI call fails」：改成 `runCmdCall("ls")`；把 `/Agent 呼叫 AI 失敗，已停止/` 那行斷言刪掉（錯誤改由對話的錯誤列顯示），保留 `API Key 驗證失敗`、`[object Object]` 不出現、`aiChatCalls.length === 2`。
- 「Agent 執行中時讓位給狀態列」：改成 `runCmdCall("ls")`，兩處 `/不使用 MCP 工具/` 改成 `/未開啟 MCP 工具/`。
- 「切換模式時說明列跟著換」整條換成：

```tsx
    it("切換模式時說明列跟著換", async () => {
      mcpTools.push({ name: "read_file" }, { name: "write_file" });
      renderPanel();

      // 預設 ⚡ 關：純聊天
      expect(await screen.findByText(/只會建議指令/)).toBeInTheDocument();

      // 開 ⚡：MCP 預設開著且有工具 → ⚡＋MCP
      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      expect(await screen.findByText(/2 個 MCP 工具/)).toBeInTheDocument();

      // 關 MCP → 只有 ⚡
      await userEvent.click(screen.getByRole("button", { name: /MCP \(2\)/ }));
      expect(screen.getByText(/未開啟 MCP 工具/)).toBeInTheDocument();
    });
```

- 「Agent 模式開啟時，MCP 按鈕要停用並說明原因」整條換成：

```tsx
  it("⚡ 關時 MCP 按鈕停用；開 ⚡ 後可用，而且 MCP 原本的開關狀態被保留", async () => {
    mcpTools.push({ name: "read_file" }, { name: "write_file" });
    render(
      <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
        onExecuteCommand={vi.fn()} onOpenProviderPalette={vi.fn()} />,
    );

    const mcpBtn = await screen.findByRole("button", { name: /MCP \(2\)/ });
    expect(mcpBtn).toBeDisabled();
    expect(mcpBtn.getAttribute("title")).toMatch(/⚡|Agent/);
    expect(mcpBtn.className).not.toMatch(/--on/);

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    expect(mcpBtn).toBeEnabled();
    expect(mcpBtn.className).toMatch(/--on/);

    // 關掉 MCP、關 ⚡、再開 ⚡：MCP 應該還是關的
    await userEvent.click(mcpBtn);
    await userEvent.click(screen.getByTitle(/停用 Agent 模式/));
    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    expect(mcpBtn.className).not.toMatch(/--on/);
  });
```

- 「切換方案時要告訴使用者」、「訂閱計費造成的切換…」、「沒有切換方案時不顯示…」：送出前加 `await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));`（⚡ 關時是純聊天，不會有工具降級）。

- [ ] **Step 3: 新增測試（AiPanel.test.tsx，會紅）**

```tsx
  describe("⚡／MCP 決定送給後端的工具範圍", () => {
    const send = async (text: string) => {
      await userEvent.type(screen.getByRole("textbox"), text);
      await userEvent.keyboard("{Enter}");
    };
    const mount = () => render(
      <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
        onExecuteCommand={vi.fn()} onOpenProviderPalette={vi.fn()} />,
    );

    it("⚡ 關：none", async () => {
      mcpTools.push({ name: "read_file" });
      aiChatQueue.push({ content: "好" });
      mount();
      await send("hi");
      await waitFor(() => expect(aiChatScopes).toEqual(["none"]));
    });

    it("⚡ 開、MCP 開：all", async () => {
      mcpTools.push({ name: "read_file" });
      aiChatQueue.push({ content: "好" });
      mount();
      await screen.findByRole("button", { name: /MCP \(1\)/ });
      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      await send("hi");
      await waitFor(() => expect(aiChatScopes).toEqual(["all"]));
    });

    it("⚡ 開、MCP 關：terminal", async () => {
      mcpTools.push({ name: "read_file" });
      aiChatQueue.push({ content: "好" });
      mount();
      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      await userEvent.click(await screen.findByRole("button", { name: /MCP \(1\)/ }));
      await send("hi");
      await waitFor(() => expect(aiChatScopes).toEqual(["terminal"]));
    });

    // 終端機工具就是 ⚡ 本身，不算 MCP 工具；只有它時 MCP 按鈕不該亮、⚡ 送 terminal。
    it("工具清單只有內建終端機工具時，MCP 視為沒有工具", async () => {
      mcpTools.push({ name: "aiterm_terminal__run_command" });
      aiChatQueue.push({ content: "好" });
      mount();
      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      expect(await screen.findByRole("button", { name: /MCP OFF/ })).toBeDisabled();
      await send("hi");
      await waitFor(() => expect(aiChatScopes).toEqual(["terminal"]));
    });
  });
```

Run: `npx vitest run src/components/AiPanel/AiPanel.test.tsx`
Expected: 新測試與改寫過的 Agent 測試紅（目前 ⚡ 走舊迴圈、`toolScope` 沒帶）。

- [ ] **Step 4: 改 stuckPrompt.test.tsx**

`renderStuckAgent` 裡 `aiChatQueue.push({ content: "<cmd>cat <<EOF</cmd>" })` 改成 `aiChatQueue.push(runCmdCall("cat <<EOF"))`。其餘斷言不變（卡住提示、繼續等待、完成後不殘留、停止時呼叫 `onAgentAborted`）。

- [ ] **Step 5: 實作 AiPanel**

`src/components/AiPanel/index.tsx`：

(a) import：刪掉 `invokeAiChat, formatAiError, type AiError, type ChatMessage as AiChatMessage`、`getSessionCwd, listDirectory`、`contentToDisplayString`（若刪後 eslint／tsc 說仍有使用就保留）；加 `import { TERMINAL_TOOL_NAME } from "../../hooks/useMcpChat";`（與既有 `useMcpChat` import 合併）與 `type ToolScope` from `../../ipc/ai`。

(b) 把「卡住偵測」那段（`stuckPromptVisible`、`stuckIntervalRef`、`snoozeUntilRef`、卸載清 interval 的 effect、`handleStuckWait`、`handleStuckInterrupt`）**整段搬到 `runTerminalCommand` 之前**，並加一個清理函式：

```ts
  const clearStuckWatch = useCallback(() => {
    if (stuckIntervalRef.current !== null) {
      clearInterval(stuckIntervalRef.current);
      stuckIntervalRef.current = null;
    }
    setStuckPromptVisible(false);
  }, []);
```

(c) `runTerminalCommand` 改成（卡住偵測從舊迴圈搬進來）：

```ts
  const runTerminalCommand = useCallback(
    (rawCommand: string) =>
      new Promise<string>((resolve) => {
        // 見 heredocGuard.ts：缺結束標記的 heredoc 會讓 shell 停在 heredoc>，指令永遠不結束。
        const cmd = repairUnterminatedHeredocs(rawCommand.trim());
        terminalRunningRef.current = true;
        clearStuckWatch();
        snoozeUntilRef.current = 0;
        // 指令跑著卻完全沒有輸出多久，就提示使用者可能卡住了（見 STUCK_IDLE_MS）。
        if (getIdleMs) {
          stuckIntervalRef.current = setInterval(() => {
            if (Date.now() < snoozeUntilRef.current) return;
            if (getIdleMs() >= STUCK_IDLE_MS) setStuckPromptVisible(true);
          }, STUCK_CHECK_INTERVAL_MS);
        }
        onExecuteCommand(cmd, async (block) => {
          terminalRunningRef.current = false;
          clearStuckWatch();
          const rawOutput = (await getPtyRecentOutput(sessionId).catch(() => null)) ?? block.rawOutput ?? "";
          resolve(
            `Command \`${cmd}\` finished (${describeExit(block)}).\nOutput:\n\`\`\`\n${rawOutput.slice(-2000)}\n\`\`\``,
          );
        });
      }),
    [onExecuteCommand, sessionId, getIdleMs, clearStuckWatch],
  );
```

(d) `getMcpTools` 結果計數排除終端機工具：

```ts
      // 內建的終端機工具就是 ⚡ 本身，不算「MCP 工具」。
      setMcpToolCount(tools.filter((tool) => tool.name !== TERMINAL_TOOL_NAME).length);
```

(e) 刪掉整個舊 Agent 迴圈：`agentRunning`／`agentStep`／`agentPhase`／`agentAbortRef` 這些 state、`buildAgentSystemPrompt`、`runAgentLoopRef`、`runAgentLoop` 與同步 ref 的 effect、`submitAgent`。保留 `agentMode` state 與它的 localStorage effect。

(f) 新增 ⚡ 模式的額外提示詞（取代 `buildAgentSystemPrompt` 裡仍需要的兩段；cwd 與目錄由後端 snapshot 提供）：

```ts
  /** ⚡ 模式額外送給模型的規則：Windows 提權（exit 740）與遠端連線說明。只送給模型，不顯示。 */
  const buildAgentExtraPrompt = useCallback((): string => {
    // Windows 專屬：沒有這條規則時，AI 遇到 exit 740 會自己發明 `Start-Process -Verb RunAs`，
    // 把指令送進一個 AITerm 看不到輸出的獨立提權視窗。AITerm 自己會偵測 740 並顯示提權橫幅，
    // 使用者確認後在提權 session 重跑並把結果交回（見 TerminalView 的 heldCompletionRef）。
    const elevationRule = IS_WINDOWS
      ? `Elevation: if a command fails with exit code 740, or reports that it needs elevated/administrator privileges, do NOT try to work around it — never use \`Start-Process -Verb RunAs\`, \`runas\`, or redirect an elevated command's output to a temp file and read it back. Those run in a separate window whose output this terminal cannot see. AITerm handles elevation itself: when a command fails for lack of privileges it asks the user to elevate this session, and if they approve, it re-runs the command elevated and gives you that result instead. So if you still receive a privilege failure, the user declined elevation — stop calling aiterm_terminal__run_command, and tell the user in ${languageDirective(locale)} that the command needs administrator rights and that they can ask you again and approve AITerm's elevation prompt.`
      : "";
    return (elevationRule + remoteSessionPromptNote(getRemoteSession?.())).trim();
  }, [locale, getRemoteSession]);
```

(g) 模式與按鈕：

```ts
  // MCP 只在 ⚡ 開時有作用；送出時與模式說明列共用同一個判斷，拆成兩份遲早有一邊漏改。
  const mcpActive = agentMode && useMcp && mcpEnabled && mcpToolCount > 0;
  const mode: PanelMode = agentMode ? (mcpActive ? "agent-mcp" : "agent-terminal") : "suggest";
  const agentRunning = chat.activeScope === "terminal" || chat.activeScope === "all";
  const isDisabled = chat.isStreaming;

  const sendTurn = (text: string) => {
    const currentAttachments = attachments;
    setAttachments([]);
    const scope: ToolScope = agentMode ? (mcpActive ? "all" : "terminal") : "none";
    void chat.send(
      text,
      scope,
      undefined,
      currentAttachments.length > 0 ? currentAttachments : undefined,
      agentMode ? buildAgentExtraPrompt() || undefined : undefined,
    );
  };

  // 停止：中斷 AI 呼叫；有指令在跑就 Ctrl+C 並強制結案；並通知外層放行被扣住的完成回呼（提權橫幅）。
  const stopTurn = () => {
    chat.stop();
    if (terminalRunningRef.current) {
      terminalRunningRef.current = false;
      clearStuckWatch();
      if (onInterruptCommand) onInterruptCommand();
      else writePty(sessionId, "\x03").catch(() => {});
    }
    onAgentAborted?.();
  };
```

(h) `ChatPanelShell` 的 props 改成：

```tsx
      isStreaming={chat.isStreaming}
      thinkingLabel={
        agentRunning
          ? chat.phase === "thinking" ? t.ai_agent_thinking : t.ai_agent_executing
          : chat.isStreaming ? t.ai_thinking : null
      }
      …
      onSend={sendTurn}
      onSubmitAgent={sendTurn}
      mode={mode}
      agentRunning={agentRunning}
      agentPhase={chat.phase}
      agentStep={chat.step}
      onStop={stopTurn}
      onAbortAgent={stopTurn}
```

（保留其餘 props；原本 `onSend`／`onSubmitAgent`／`onStop`／`onAbortAgent` 的 inline 函式整段換掉。）

(i) MCP 按鈕：

```tsx
        <button
          type="button"
          // MCP 只在 ⚡ 開時生效：⚡ 關＝純聊天，按鈕停用且不亮，但記住原本的開關（localStorage）。
          className={`aiterm-mcp-toggle${mcpActive ? " aiterm-mcp-toggle--on" : ""}`}
          title={
            !agentMode
              ? "開啟 ⚡ 後才能使用 MCP 工具（目前只跟 AI 聊天）"
              : mcpToolCount === 0
                ? t.mcp_toggle_no_servers
                : (useMcp ? "MCP 開啟" : "MCP 關閉")
          }
          disabled={!agentMode || mcpToolCount === 0 || isDisabled}
          onClick={() => setUseMcp((v) => !v)}
        >
```

（內容的 `WrenchIcon` 與 `MCP (n)`／`MCP OFF` 不變。）

(j) `extraAboveInput` 的卡住提示條件 `agentRunning && stuckPromptVisible` 保留（`agentRunning` 現在是上面算出來的常數）。

- [ ] **Step 6: 確認綠**

Run: `npx tsc -b && npx vitest run src/components/AiPanel src/hooks/useMcpChat.test.ts`
Expected: tsc 無輸出；全綠。

- [ ] **Step 7: 突變驗證**

各做一次、確認有測試紅、再用 `cp` 還原：
- `scope` 改成永遠 `"all"` → scope 測試紅。
- `mcpActive` 拿掉 `agentMode &&` → 「⚡ 關時 MCP 按鈕停用」或 `none` 測試紅（按鈕條件另外也要拿掉 `!agentMode ||` 才會紅，兩者分開試）。
- 拿掉 `tools.filter(...)` → 「只有內建終端機工具」紅。
- `runTerminalCommand` 裡拿掉 `setInterval` 那段 → stuckPrompt 測試紅。
- `stopTurn` 拿掉 `onAgentAborted?.()` → 停止鍵測試紅。

- [ ] **Step 8: Commit**

```bash
git add src/components/AiPanel/index.tsx src/components/AiPanel/AiPanel.test.tsx src/components/AiPanel/stuckPrompt.test.tsx
git commit -m "feat(ask-ai): one tool loop — agent runs via run_command, MCP only with agent, plain chat otherwise

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: CHANGELOG 與 spec 補記

**Files:**
- Modify: `CHANGELOG.md`（最上方，在 `## v1.40.2` 之前）
- Modify: `docs/superpowers/specs/2026-10-14-unified-agent-mcp-toggle-design.md`

- [ ] **Step 1: CHANGELOG**

版本號用 `v1.41.0`（行為改變）；打 tag 前若使用者要別的版號再改。寫給使用者看：

```markdown
## v1.41.0

**Ask AI 的 ⚡ 和 MCP 兩顆按鈕改成一起用**
- ⚡ 關閉：只跟 AI 聊天。AI 不會自己執行任何東西，回覆裡的指令照樣可以按 ▶ 自己執行；這時 MCP 按鈕是灰的。
- 只開 ⚡：AI 會自己在終端機執行指令、看結果再繼續，直到完成。
- ⚡ 和 MCP 都開：除了終端機指令，AI 也能使用 MCP 工具和電腦操作。以前開了 ⚡ 就不能用 MCP，現在可以同時用。
- 關掉 ⚡ 時 MCP 的開關會被記住，下次開 ⚡ 會恢復。

**⚡ 執行指令前會照「執行模式」設定詢問你**
以前 ⚡ 模式下 AI 的指令都直接執行，不會問。現在會照「設定 → 執行模式」：「一律確認」每個指令都會先問你（這是預設值）；「分級自動」只有需要確認或危險的指令會問；「全自動 Agent」只有危險的指令會問。被安全規則封鎖的指令一律不執行。不想每次被問，可以到設定改成「分級自動」或「全自動 Agent」。

**其他**
- ⚡ 模式下附加的檔案和圖片現在也會一起送給 AI（以前會被略過）。
```

- [ ] **Step 2: spec 補記**

在 spec 的「## 不做」之前加：

```markdown
## 實作時補充的決定

- 面板的 MCP 工具數量排除內建終端機工具（`get_mcp_tools` 會回傳它）：終端機工具就是 ⚡ 本身，算進去的話 MCP 按鈕永遠至少有 1 個工具。
- 遠端終端機的 `RemoteAiPanel` 仍走 `lib/agentLoop.ts` 的 `<cmd>` 迴圈、模式值 `agent` 與 `mode_hint_agent` 字串不動；本機改用 `agent-terminal`／`agent-mcp`。
- ⚡ 模式的 system 提示詞補充不再包含 cwd 與目錄清單：後端 `context::snapshot` 已放進 `build_chat_prompt`。
```

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git add -f docs/superpowers/specs/2026-10-14-unified-agent-mcp-toggle-design.md
git commit -m "docs: v1.41.0 notes and spec addendum for the unified agent/MCP toggle

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: 完整驗證與真機

- [ ] **Step 1: 全套自動化**

```bash
npx tsc -b
npm run test 2>&1 | tail -15
cd src-tauri && cargo test --workspace --no-fail-fast 2>&1 | grep -E "test result|FAILED|panicked"
```

Expected：tsc 無輸出；vitest 全綠（若有已知 flaky 的 MailView／TaskBoard 測試失敗，單獨重跑三次確認與本次無關並在回報中寫明）；Rust 每一行 `test result:` 都是 `ok`。

- [ ] **Step 2: 確認 snapshot 有 cwd**

Run: `grep -n "cwd\|dir_listing" src-tauri/src/commands/ai.rs | sed -n 1,20p`，確認 `build_chat_prompt` 有把 `snapshot.cwd` 與目錄清單放進提示詞。沒有的話停下來回報（spec 假設它有）。

- [ ] **Step 3: 真機（`npm run tauri:dev`，要使用者在場）**

1. ⚡ 關：問「這個資料夾有什麼」→ AI 只回答或給 ▶ 指令，不自己執行；MCP 按鈕是灰的。
2. 只開 ⚡（MCP 關）：「列出目前資料夾的檔案」→ 終端機自己跑 `ls`、AI 看結果回答；狀態列顯示步驟數與「執行指令中」。
3. ⚡＋MCP：「打開計算機」→ 用電腦操作工具開 App。
4. 執行模式設「一律確認」再用 ⚡ 跑 `ls` → 跳授權框；設「全自動 Agent」→ 直接跑。
5. ⚡ 跑一個不會結束的指令（例如 `cat`）→ 兩分鐘後出現卡住提示；按停止 → 終端機收到 Ctrl+C、狀態列消失。

每一項的結果都要使用者確認；沒看過的項目在回報中標成「未驗證」，CHANGELOG 不可宣稱。

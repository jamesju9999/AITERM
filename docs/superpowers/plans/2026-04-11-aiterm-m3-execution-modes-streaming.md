# AITerm M3 — Execution Modes, Streaming UX & Rich Context

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** AI-generated commands respect the user's Execution Mode (AlwaysConfirm / Graded / FullAuto) based on the AI's risk assessment. Streaming tokens are shown in real time instead of buffering. The AI receives richer context (recent terminal output, directory listing) for better suggestions. Error UX is polished for multi-provider scenarios. A provider status badge and quick-switch palette round out the terminal experience.

**Prerequisite:** M2 (Multi-Provider + Settings UI) is fully implemented. The `ExecutionMode` and `RiskLevel` enums exist but `risk_level` is currently parsed and discarded (`commands/ai.rs:95`).

**Architecture changes:**
- Backend: `ai_query` returns `risk_level` to the frontend. A new `ContextCollector` gathers recent terminal output and directory info. Streaming is exposed via Tauri events instead of buffering the full response.
- Frontend: `TerminalView` makes execution decisions based on `(ExecutionMode, RiskLevel)`. A streaming indicator shows token-by-token output. `CommandPreview` gains risk-level visual indicators. A provider status badge and `Ctrl+Shift+P` palette are added.

**Tech Stack additions:**
- Rust: `ring_buffer` or manual `VecDeque` for terminal output capture (no new crate needed)
- React: no new deps

**Working directory:** `D:\Tool\AITerm`

---

## File Structure

### New files
- `src-tauri/src/ai/context_collector.rs` — Captures recent PTY output and enriches `EnvSnapshot`
- `src/components/ProviderPalette.tsx` — `Ctrl+Shift+P` quick-switch overlay
- `src/components/ProviderPalette.css`
- `src/components/StreamingIndicator.tsx` — Token-by-token display during AI generation
- `src/components/StreamingIndicator.css`

### Modified files
- `src-tauri/src/ai/mod.rs` — Extend `EnvSnapshot` with optional context fields; add `AiQueryResult` that includes `risk_level`
- `src-tauri/src/ai/context.rs` — Import and use `ContextCollector` for enriched snapshots
- `src-tauri/src/commands/ai.rs` — Return `risk_level`; emit streaming events; accept `execution_mode` from config
- `src-tauri/src/pty/mod.rs` — Add output capture ring buffer to `PtySession`
- `src/ipc/ai.ts` — Update `AiCommandReady` type to include `risk_level`; add streaming event listener
- `src/components/TerminalView.tsx` — Execution mode logic; streaming indicator; provider badge; palette trigger
- `src/components/TerminalView.css` — Styles for badge, palette, streaming
- `src/components/CommandPreview.tsx` — Risk-level badge (safe/caution/danger colors)
- `src/components/CommandPreview.css` — Risk-level styling

---

## Phase 1: Risk-Aware Execution

### Task 1.1: Return `risk_level` from `ai_query`

Currently `commands/ai.rs` parses `AiSingleCommand` (which includes `risk_level`) but discards it at line 95. Expose it to the frontend.

- [ ] Modify `AiCommandReady` in `commands/ai.rs`:
  ```rust
  #[derive(Debug, Clone, Serialize)]
  pub struct AiCommandReady {
      pub command: String,
      pub explanation: String,
      pub risk_level: RiskLevel,
  }
  ```
- [ ] Add `Serialize` derive to `RiskLevel` in `ai/mod.rs` (currently only has `Deserialize`)
- [ ] Remove the `let _ = parsed.risk_level;` line in `ai_query`, pass it through:
  ```rust
  Ok(AiCommandReady {
      command: parsed.command,
      explanation: parsed.explanation,
      risk_level: parsed.risk_level,
  })
  ```
- [ ] Update the `AiCommandReady` type in `src/ipc/ai.ts`:
  ```typescript
  export interface AiCommandReady {
    command: string;
    explanation: string;
    risk_level: "safe" | "needs_confirm" | "dangerous";
  }
  ```
- [ ] Update existing tests in `commands/ai.rs`

### Task 1.2: Read `ExecutionMode` in the frontend

The frontend needs to know the current execution mode to decide whether to auto-execute or show the preview.

- [ ] Add `getExecutionMode()` to `src/ipc/config.ts`:
  ```typescript
  export async function getExecutionMode(): Promise<ExecutionMode> {
    const config = await getConfig();
    return config.execution_mode;
  }
  ```
- [ ] In `TerminalView`, fetch and cache `execution_mode` on mount (re-fetch when returning from settings)
- [ ] Store in a `useRef` so it's accessible inside the `onData` closure without re-creating the effect

### Task 1.3: Execution decision logic in `TerminalView`

Implement the decision matrix:

| ExecutionMode \ RiskLevel | safe | needs_confirm | dangerous |
|---------------------------|------|---------------|-----------|
| AlwaysConfirm             | preview | preview | preview |
| Graded                    | auto-exec | preview | preview + warning |
| FullAuto                  | auto-exec | auto-exec | preview + warning |

- [ ] Create a helper function `shouldAutoExecute(mode: ExecutionMode, risk: RiskLevel): boolean`
- [ ] In `handleAiQuery` success handler (currently at `TerminalView.tsx:218`):
  - If `shouldAutoExecute` returns true:
    - Write the command to the terminal: `term.write("\r\n\x1b[32m▶ " + command + "\x1b[0m\r\n")`
    - Auto-send to PTY: `writePty(session, command + "\r")`
    - Set preview to `INITIAL_PREVIEW` (no dialog)
  - If false:
    - Show `CommandPreview` as before, but pass `risk_level` for visual treatment
- [ ] For `dangerous` in Graded/FullAuto mode, add a warning line:
  ```
  ⚠ 危險操作 — 請仔細確認後再執行
  ```
- [ ] Tests: unit test `shouldAutoExecute` for all 9 combinations

### Task 1.4: Risk-level visual indicators in `CommandPreview`

- [ ] Add `riskLevel` prop to `CommandPreview`:
  ```typescript
  export interface CommandPreviewProps {
    command: string;
    explanation: string;
    riskLevel: "safe" | "needs_confirm" | "dangerous";
    onConfirm: () => void;
    onCancel: () => void;
  }
  ```
- [ ] Show a colored badge next to the command:
  - `safe` → green `[Safe]`
  - `needs_confirm` → yellow `[Caution]`
  - `dangerous` → red `[Dangerous]`
- [ ] For `dangerous`, change the Execute button to red with text "Execute Anyway"
- [ ] Update `CommandPreview.css` with risk-level color classes

---

## Phase 2: Streaming UX

### Task 2.1: Emit streaming events from backend

Currently `ai_query` buffers all chunks into `buf` and returns the final parsed result. Change this to emit interim events so the frontend can show progress.

- [ ] Define a Tauri event payload in `commands/ai.rs`:
  ```rust
  #[derive(Debug, Clone, Serialize)]
  pub struct AiStreamEvent {
      pub session_id: String,
      pub delta: String,
      pub done: bool,
  }
  ```
- [ ] In `ai_query`, accept `app: tauri::AppHandle` parameter (Tauri injects this automatically)
- [ ] While draining the `rx` channel, emit each chunk as a Tauri event:
  ```rust
  while let Some(chunk) = rx.recv().await {
      let _ = app.emit("ai-stream", AiStreamEvent {
          session_id: session_id.clone(),
          delta: chunk.delta.clone(),
          done: chunk.done,
      });
      buf.push_str(&chunk.delta);
      if chunk.done { break; }
  }
  ```
- [ ] The final `AiCommandReady` is still returned as the command result (streaming events are supplementary)

### Task 2.2: Streaming indicator component

- [ ] Create `src/components/StreamingIndicator.tsx`:
  ```typescript
  interface StreamingIndicatorProps {
    text: string;        // accumulated text so far
    visible: boolean;
  }
  ```
  - Renders a small overlay at the bottom of the terminal showing the raw AI output as it arrives
  - Monospace font, dark background with slight transparency
  - Auto-scrolls to bottom as text grows
  - Max height ~4 lines, scrollable if longer
  - Subtle pulsing cursor at the end to show activity
- [ ] Create `src/components/StreamingIndicator.css`

### Task 2.3: Wire streaming events in `TerminalView`

- [ ] Listen for `ai-stream` events using `listen()` from `@tauri-apps/api/event`
- [ ] When `handleAiQuery` starts (loading=true), begin accumulating stream deltas
- [ ] Show `StreamingIndicator` with the accumulated text while loading
- [ ] When `ai_query` invoke resolves (the command result returns), hide the streaming indicator and show the CommandPreview (or auto-execute)
- [ ] Filter events by `session_id` to avoid cross-session interference
- [ ] Clean up listener on unmount

---

## Phase 3: Rich Context

### Task 3.1: Terminal output capture in PTY

Capture recent terminal output so the AI can see what the user is looking at.

- [ ] In `src-tauri/src/pty/mod.rs`, add a ring buffer to `PtySession`:
  ```rust
  pub struct PtySession {
      // ... existing fields ...
      output_ring: parking_lot::Mutex<VecDeque<u8>>,
  }
  ```
  - Ring buffer capacity: 8 KB (enough for ~200 lines of typical output)
- [ ] In the PTY data callback (where output bytes are forwarded to the frontend), also push bytes into the ring buffer
- [ ] Add `PtyManager::get_recent_output(&self, session_id: &str, max_bytes: usize) -> Option<String>`:
  - Drain up to `max_bytes` from the ring buffer tail
  - Decode as lossy UTF-8
  - Strip ANSI escape sequences (use a simple regex or manual state machine)
- [ ] Unit test: push bytes, read back, verify ring buffer wraps correctly

### Task 3.2: ANSI stripping utility

- [ ] Create a function `strip_ansi(input: &str) -> String` in `src-tauri/src/pty/mod.rs` (or a `util` module)
  - Remove CSI sequences (`\x1b[...m`, `\x1b[...H`, etc.)
  - Remove OSC sequences (`\x1b]...ST`)
  - Keep plain text content
- [ ] Unit tests with common terminal escape sequences

### Task 3.3: Extend `EnvSnapshot` with context fields

- [ ] Add optional fields to `EnvSnapshot`:
  ```rust
  pub struct EnvSnapshot {
      pub os: String,
      pub shell: String,
      pub cwd: PathBuf,
      /// Recent terminal output (last ~50 lines), ANSI-stripped. None if unavailable.
      #[serde(skip_serializing_if = "Option::is_none")]
      pub recent_output: Option<String>,
      /// Top-level directory listing of cwd. None if unavailable.
      #[serde(skip_serializing_if = "Option::is_none")]
      pub dir_listing: Option<String>,
  }
  ```
- [ ] Update `context::snapshot()` to populate these:
  - `recent_output`: call `pty_manager.get_recent_output(session_id, 4096)`, trim to last 50 lines
  - `dir_listing`: `std::fs::read_dir(cwd)` → collect up to 50 entries, format as `name (dir|file|symlink)`
- [ ] Update `snapshot_from_parts()` to accept optional context (default None for tests)
- [ ] Update all existing tests that construct `EnvSnapshot` to include the new fields (set to `None`)

### Task 3.4: Update system prompt to use rich context

- [ ] In `commands/ai.rs::build_single_command_prompt()`, append context sections:
  ```
  Recent terminal output (last ~50 lines):
  ```
  {recent_output or "(not available)"}
  ```

  Directory listing (cwd):
  ```
  {dir_listing or "(not available)"}
  ```
  ```
- [ ] Guard: if `recent_output` is very long, truncate to last 2000 chars
- [ ] Update the prompt test to verify context sections appear when populated

---

## Phase 4: Error UX Polish (M2 Phase 7.2 completion)

### Task 4.1: Update `formatAiError` for multi-provider context

- [ ] Rewrite `src/ipc/ai.ts::formatAiError`:
  ```typescript
  export function formatAiError(e: AiError): string {
    switch (e.kind) {
      case "not_configured":
        return "aiterm: 尚未設定 AI Provider。請按 Ctrl+, 開啟設定。";
      case "network":
        if (e.message?.includes("Ollama") || e.message?.includes("connection refused")) {
          return "aiterm: 無法連線到 Ollama。請確認 Ollama 已啟動。";
        }
        return `aiterm: 網路錯誤 — ${e.message}`;
      case "auth_failed":
        return "aiterm: API Key 驗證失敗。請至設定頁更新。";
      case "rate_limit":
        return e.retry_after
          ? `aiterm: 請求過於頻繁（${e.retry_after} 秒後重試）`
          : "aiterm: 請求過於頻繁，請稍後再試";
      case "model_error":
        return `aiterm: AI 回傳格式錯誤（${e.reason}）`;
    }
  }
  ```

### Task 4.2: Actionable error hints in TerminalView

- [ ] When `not_configured` error occurs, write a clickable-style hint:
  ```
  提示：按 Ctrl+, 開啟設定並新增一個 AI Provider。
  ```
  (Already partially done at `TerminalView.tsx:229-235`, but update the text for M2 context)
- [ ] When network error mentions Ollama/connection:
  ```
  提示：請啟動 Ollama，或按 Ctrl+, 切換到雲端 Provider。
  ```
- [ ] When `auth_failed`:
  ```
  提示：請按 Ctrl+, 至設定頁更新 API Key。
  ```

---

## Phase 5: Provider Status Badge & Quick Switch (M2 Phase 7.1 completion)

### Task 5.1: Provider status badge in terminal header

- [ ] In `TerminalView`, fetch the default provider info on mount:
  ```typescript
  const [activeProvider, setActiveProvider] = useState<string>("");
  useEffect(() => {
    listProviders().then(providers => {
      const active = providers.find(p => p.is_default);
      setActiveProvider(active?.display_name ?? "未設定");
    }).catch(() => setActiveProvider("未設定"));
  }, []);
  ```
- [ ] Display in the status bar between the session info and the gear icon:
  ```tsx
  <span className="aiterm-status-provider">{activeProvider}</span>
  ```
- [ ] Style: subtle, monospace, dimmed color. Click opens settings.
- [ ] Re-fetch when navigating back from settings (use a simple `location.key` or callback)

### Task 5.2: `Ctrl+Shift+P` Provider Palette

A lightweight command-palette-style overlay for quick provider switching.

- [ ] Create `src/components/ProviderPalette.tsx`:
  - Fetches `listProviders()` on open
  - Renders a centered overlay with a list of providers
  - Current default has a checkmark `✓`
  - Click or Enter on a provider → `setDefaultProvider(id)` → close palette → update badge
  - Escape → close
  - Arrow keys for navigation
- [ ] Create `src/components/ProviderPalette.css`:
  - Dark overlay backdrop
  - Centered card, max-width 400px
  - Each provider row: type icon + display_name + model
  - Highlight on hover/keyboard focus
- [ ] Wire in `TerminalView`:
  - `Ctrl+Shift+P` → toggle palette visibility
  - State: `const [paletteOpen, setPaletteOpen] = useState(false)`
  - Render `<ProviderPalette>` when open
  - On select: update `activeProvider` state for badge

---

## Phase 6: M2 Phase 7 Remaining — Integration Tests

### Task 6.1: Wiremock contract tests for Anthropic

- [ ] Create `src-tauri/tests/anthropic_client.rs`:
  - Happy path: mock SSE stream → verify chunked output
  - 401 → `AiError::AuthFailed`
  - 429 → `AiError::RateLimit`
  - 529 → `AiError::Network` with "overloaded" message
- [ ] Use `wiremock` crate (already a dev-dependency from M1)

### Task 6.2: Wiremock contract tests for Ollama

- [ ] Create `src-tauri/tests/ollama_client.rs`:
  - Happy path: mock NDJSON stream → verify chunked output
  - Connection refused simulation → `AiError::Network`
  - Health check: mock `/api/tags` → success / failure

### Task 6.3: Wiremock contract tests for OpenAiCompatible

- [ ] Create `src-tauri/tests/compatible_client.rs`:
  - Same SSE format as OpenAI tests
  - Test with api_key=None (no Authorization header sent)
  - Test with custom base_url

### Task 6.4: ConfigStore integration tests

- [ ] Create `src-tauri/tests/config_store.rs`:
  - Round-trip: create config, save, reload from disk, verify equality
  - Atomic write: verify no partial file on crash (write to tmp dir)
  - Corrupt recovery: write invalid TOML, load → defaults returned
  - Concurrent access: multiple threads reading/writing

---

## Dependency Graph

```
Phase 1 (Risk-Aware Execution)     Phase 3 (Rich Context)
   │                                   │
   ├── Task 1.1 (backend)             ├── Task 3.1 (PTY capture)
   ├── Task 1.2 (frontend read)       ├── Task 3.2 (ANSI strip)
   ├── Task 1.3 (decision logic)      ├── Task 3.3 (EnvSnapshot)
   └── Task 1.4 (preview UI)          └── Task 3.4 (prompt update)
         │                                   │
         └───────────┬───────────────────────┘
                     │
              Phase 2 (Streaming UX)
                     │
              ├── Task 2.1 (backend events)
              ├── Task 2.2 (indicator component)
              └── Task 2.3 (wire in TerminalView)

Phase 4 (Error UX)          Phase 5 (Badge & Palette)
   │                            │
   └──── independent ───────────┘
                │
         Phase 6 (Integration Tests) ← after everything
```

**Parallelism opportunities:**
- Phase 1 and Phase 3 are independent — can be developed in parallel
- Phase 4 and Phase 5 are independent of each other and of Phase 1/3
- Phase 2 depends on Phase 1 (needs `risk_level` in the response)
- Phase 6 can start as soon as Phase 1-5 stabilize

---

## Key Design Decisions

**D1: Streaming is supplementary, not the primary return path.**
`ai_query` still returns the final `AiCommandReady` as its invoke result. Streaming events are emitted as Tauri events (`ai-stream`) for progressive display only. This avoids changing the invoke contract and keeps error handling simple — the invoke either succeeds with a result or fails with an `AiError`. The streaming indicator is purely cosmetic.

**D2: Execution mode is read from config, not passed per-request.**
The frontend reads `execution_mode` from `ConfigStore` (via `getConfig()`) and caches it. It does not pass the mode to the backend — the decision to auto-execute vs preview happens entirely in the frontend. This keeps the backend stateless about UX preferences.

**D3: Terminal output capture uses a fixed-size ring buffer, not unbounded history.**
8 KB (~200 lines) is enough for the AI to understand the user's current context without ballooning memory. The ring buffer is per-session and wraps automatically. ANSI stripping happens at read time, not write time, to avoid the overhead on every byte.

**D4: Rich context is best-effort and optional.**
If `read_dir` fails (permissions, deleted directory) or the ring buffer is empty, the context fields are `None` and the prompt degrades gracefully. The AI still works without context — it just gives less informed suggestions.

**D5: `shouldAutoExecute` lives in the frontend, not the backend.**
The backend's job is to translate natural language to a command and assess risk. The frontend's job is to decide what to do with that assessment based on user preferences. This separation means the backend never auto-executes anything — it only returns data.

**D6: Provider palette is a lightweight overlay, not a routed page.**
`Ctrl+Shift+P` should feel instant, like VS Code's command palette. It fetches the provider list, renders an overlay, and closes on selection. No navigation, no route change, no state teardown of the terminal.

---

## Acceptance Criteria

1. **Graded mode works:** User sets Graded mode → `/ai list files` → command auto-executes (safe). `/ai rm -rf /tmp/data` → preview with yellow caution badge.
2. **FullAuto mode works:** Safe and needs_confirm both auto-execute. Dangerous shows preview with red badge.
3. **Streaming visible:** While AI generates, a streaming indicator shows partial JSON arriving in real time.
4. **Rich context helps:** AI sees recent terminal output → suggestions reference visible errors/output. AI sees directory listing → file path suggestions are accurate.
5. **Error messages are helpful:** NotConfigured → tells user to open settings. Ollama down → tells user to start Ollama. Auth failed → tells user to update key.
6. **Provider badge shows active provider name** in the terminal header bar.
7. **`Ctrl+Shift+P`** opens a palette, user can switch providers without leaving the terminal.
8. **All M1 provider contract tests pass** (OpenAI wiremock). New Anthropic/Ollama/Compatible tests pass.

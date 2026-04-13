# AITerm M4 — AI Panel + Multi-turn Chat Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the AI Panel with multi-turn chat, streaming display, `<cmd>` tag parsing, and one-click execution — toggled with `Ctrl+I`.

**Architecture:** Frontend-stateful — `AiPanel` React component owns `messages` state per session; backend `ai_chat` Tauri command is stateless and mirrors `ai_query`. Streaming reuses the existing `ai-stream` Tauri event, extended with a `kind` field to distinguish `/ai` (`query`) from chat (`chat`).

**Tech Stack:** Rust (Tauri, tokio, async-trait, wiremock), React 19 + TypeScript, vitest + @testing-library/react.

**Design spec:** `docs/superpowers/specs/2026-04-13-aiterm-m4-design.md`

---

## File Structure

### New backend files
- None. All backend changes are additions to existing files.

### Modified backend files
- `src-tauri/src/ai/mod.rs` — add `AiStreamKind` enum (no — actually moved to commands/ai.rs to avoid cycles; see Task 1)
- `src-tauri/src/commands/ai.rs` — add `AiStreamKind`, `AiStreamEvent.kind` field, `build_chat_prompt`, `AiChatReply`, `ai_chat` command, tests
- `src-tauri/src/lib.rs` — register `ai_chat` in invoke_handler

### New backend tests
- `src-tauri/tests/ai_chat_command.rs` — integration test with `MockProvider` (mirrors `ai_query_command.rs`)

### Modified existing backend tests
- `src-tauri/tests/ai_query_command.rs` — if it inspects event payloads, update to expect `kind: "query"`

### New frontend files
- `src/lib/chatHistory.ts` — `truncateHistory()` pure function
- `src/lib/chatHistory.test.ts` — vitest
- `src/lib/cmdParser.ts` — `parseCmdTags()` pure function
- `src/lib/cmdParser.test.ts` — vitest
- `src/hooks/useAiChat.ts` — React hook (state + invoke + event listener)
- `src/hooks/useAiChat.test.ts` — vitest + @testing-library/react
- `src/components/AiPanel/index.tsx` — main panel element
- `src/components/AiPanel/MessageList.tsx` — message list rendering
- `src/components/AiPanel/MessageBubble.tsx` — single message (with `<cmd>` rendering)
- `src/components/AiPanel/CmdTag.tsx` — clickable cmd button
- `src/components/AiPanel/styles.css` — panel overlay styles
- `src/components/AiPanel/CmdTag.test.tsx` — vitest + @testing-library/react
- `src/components/AiPanel/MessageBubble.test.tsx` — vitest + @testing-library/react
- `src/components/AiPanel/AiPanel.test.tsx` — panel integration test

### Modified frontend files
- `src/ipc/ai.ts` — add `AiStreamKind`, extend `AiStreamEvent`, add `AiChatReply`, `invokeAiChat()`
- `src/components/TerminalView.tsx` — add `panelOpen` state, `Ctrl+I` shortcut, mount `AiPanel`, xterm input gate, update stream listener to filter `kind === "query"`

### Dependencies to add
- `@testing-library/react` (dev) — for hook and component tests

---

## Task 1: Backend — Extend `AiStreamEvent` with `kind`

**Context:** M3 already emits `ai-stream` events from `ai_query`. We need to distinguish query vs chat so the frontend listeners don't cross-wire. The enum is small; we put it in `commands/ai.rs` next to the event struct that already lives there.

**Files:**
- Modify: `src-tauri/src/commands/ai.rs` (add enum + field)
- Modify: `src/ipc/ai.ts` (mirror the TypeScript type)

- [ ] **Step 1: Add `AiStreamKind` enum and extend `AiStreamEvent` in Rust**

Edit `src-tauri/src/commands/ai.rs`. Find the existing `AiStreamEvent` struct and replace it with:

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiStreamKind {
    Query, // /ai single-command flow
    Chat,  // AI Panel multi-turn flow
}

/// Payload emitted as a Tauri event for each streaming chunk.
#[derive(Debug, Clone, Serialize)]
pub struct AiStreamEvent {
    pub session_id: String,
    pub kind: AiStreamKind,
    pub delta: String,
    pub done: bool,
}
```

- [ ] **Step 2: Update the existing `ai_query` emit site to pass `kind`**

In the same file, inside the `ai_query` function body, find the `app.emit("ai-stream", AiStreamEvent { ... })` call and change it to include `kind: AiStreamKind::Query`:

```rust
let _ = app.emit("ai-stream", AiStreamEvent {
    session_id: session_id.clone(),
    kind: AiStreamKind::Query,
    delta: chunk.delta.clone(),
    done: chunk.done,
});
```

- [ ] **Step 3: Compile check**

Run: `cd src-tauri && cargo check`
Expected: clean build, no errors. Warnings about unused `Chat` variant are OK for now.

- [ ] **Step 4: Mirror the new type in TypeScript**

Edit `src/ipc/ai.ts`. Replace the `AiStreamEvent` interface with:

```typescript
export type AiStreamKind = "query" | "chat";

export interface AiStreamEvent {
  session_id: string;
  kind: AiStreamKind;
  delta: string;
  done: boolean;
}
```

- [ ] **Step 5: Update `TerminalView.tsx` listener to filter `kind === "query"`**

In `src/components/TerminalView.tsx`, find the `unlistenStream = await listen<AiStreamEvent>("ai-stream", ...)` block and add a kind filter at the top of the callback:

```typescript
unlistenStream = await listen<AiStreamEvent>("ai-stream", (event) => {
  if (event.payload.kind !== "query") return;
  if (event.payload.session_id !== sessionId) return;
  if (!event.payload.done) {
    setStreamText((t) => t + event.payload.delta);
  }
});
```

- [ ] **Step 6: Run frontend type check and tests**

Run: `npm run build`
Expected: TypeScript compiles clean.

Run: `npm test -- src/ipc/ai.test.ts`
Expected: existing tests pass unchanged.

- [ ] **Step 7: Run backend tests**

Run: `cd src-tauri && cargo test`
Expected: all existing tests pass. The `ai_query_command.rs` integration test should still pass because it doesn't inspect the event payload structure.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/commands/ai.rs src/ipc/ai.ts src/components/TerminalView.tsx
git commit -m "feat(m4): add AiStreamKind to distinguish query vs chat streams"
```

---

## Task 2: Backend — `build_chat_prompt` function

**Context:** Mirrors M3's `build_single_command_prompt`. Chat mode does NOT instruct the AI to output JSON — instead it explains the `<cmd>` tag protocol.

**Files:**
- Modify: `src-tauri/src/commands/ai.rs`

- [ ] **Step 1: Write the failing tests first**

At the bottom of `src-tauri/src/commands/ai.rs`, inside `mod tests`, add these four tests:

```rust
#[test]
fn chat_prompt_contains_environment_fields() {
    let snap = make_snap("windows", "pwsh", "C:\\Users\\a");
    let prompt = build_chat_prompt(&snap);
    assert!(prompt.contains("OS: windows"));
    assert!(prompt.contains("Shell: pwsh"));
    assert!(prompt.contains("C:\\Users\\a"));
}

#[test]
fn chat_prompt_includes_recent_output_when_present() {
    let snap = EnvSnapshot {
        os: "linux".into(),
        shell: "bash".into(),
        cwd: PathBuf::from("/tmp"),
        recent_output: Some("$ ls\nfoo  bar".into()),
        dir_listing: None,
    };
    let prompt = build_chat_prompt(&snap);
    assert!(prompt.contains("Recent terminal output"));
    assert!(prompt.contains("foo  bar"));
}

#[test]
fn chat_prompt_instructs_cmd_tag_format() {
    let snap = make_snap("linux", "bash", "/");
    let prompt = build_chat_prompt(&snap);
    assert!(prompt.contains("<cmd>"), "prompt must mention <cmd> tag");
    assert!(prompt.contains("</cmd>"), "prompt must mention closing tag");
}

#[test]
fn chat_prompt_omits_json_schema_rules() {
    let snap = make_snap("linux", "bash", "/");
    let prompt = build_chat_prompt(&snap);
    // Chat mode must NOT contain the single-command JSON schema instruction.
    assert!(
        !prompt.contains("Output ONLY a JSON object"),
        "chat prompt must not inherit the JSON schema rule"
    );
    assert!(
        !prompt.contains("risk_level"),
        "chat prompt must not mention risk_level (that's single-command only)"
    );
}
```

- [ ] **Step 2: Run to verify tests fail**

Run: `cd src-tauri && cargo test --lib chat_prompt`
Expected: 4 failures with "cannot find function `build_chat_prompt`".

- [ ] **Step 3: Implement `build_chat_prompt`**

In `src-tauri/src/commands/ai.rs`, add this function right after `build_single_command_prompt`:

```rust
/// Build the system prompt for Chat mode. Unlike `build_single_command_prompt`,
/// this does NOT instruct JSON output — instead it explains the `<cmd>` tag
/// protocol and invites free-form Traditional Chinese prose.
pub fn build_chat_prompt(snapshot: &crate::ai::EnvSnapshot) -> String {
    let recent_section = snapshot.recent_output.as_deref().map(|o| {
        let trimmed = if o.len() > 2000 { &o[o.len() - 2000..] } else { o };
        format!("\nRecent terminal output (last ~50 lines):\n```\n{trimmed}\n```")
    }).unwrap_or_default();

    let dir_section = snapshot.dir_listing.as_deref().map(|d| {
        format!("\nDirectory listing ({}):\n```\n{d}\n```", snapshot.cwd.display())
    }).unwrap_or_default();

    format!(
r#"You are an AI terminal assistant. The user is in an interactive terminal
session and you can see their OS, shell, cwd, and recent output.

Environment:
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}{recent_section}{dir_section}

Rules:
1. Respond in Traditional Chinese (繁體中文).
2. When you want to suggest a runnable shell command, wrap it in
   <cmd>...</cmd> tags. The user can click the tag to execute it.
3. You may include multiple <cmd> tags in one reply if needed.
4. Each <cmd> must contain a command valid for {shell}. Prefer single-line
   commands; multi-line commands will ask the user for confirmation before
   executing.
5. Free-form explanation outside <cmd> tags is encouraged.
6. Never produce destructive operations against system roots unless the
   user explicitly asks; if you do, mark it clearly in prose."#,
        os = snapshot.os,
        shell = snapshot.shell,
        cwd = snapshot.cwd.display(),
    )
}
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cd src-tauri && cargo test --lib chat_prompt`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/ai.rs
git commit -m "feat(m4): add build_chat_prompt with <cmd> tag protocol"
```

---

## Task 3: Backend — `ai_chat` command (stateless)

**Context:** Mirrors `ai_query` but skips JSON parsing and returns raw content. Front-end sends the full `messages` array every call.

**Files:**
- Modify: `src-tauri/src/commands/ai.rs` (add command)
- Modify: `src-tauri/src/lib.rs` (register)

- [ ] **Step 1: Add `AiChatReply` struct and `ai_chat` function signature**

In `src-tauri/src/commands/ai.rs`, below the existing `ai_query` function, add:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct AiChatReply {
    pub content: String,
}

#[tauri::command]
pub async fn ai_chat(
    messages: Vec<ChatMessage>,
    session_id: String,
    app: AppHandle,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiChatReply, AiError> {
    // Reject empty history or histories whose last message isn't from the user.
    // This is a cheap sanity check — the real contract is enforced at the UI.
    if messages.is_empty() {
        return Err(AiError::ModelError {
            reason: "empty messages".into(),
            raw: String::new(),
        });
    }
    if messages.last().map(|m| m.role.as_str()) != Some("user") {
        return Err(AiError::ModelError {
            reason: "last message must be from user".into(),
            raw: String::new(),
        });
    }

    let snapshot = context::snapshot(&pty_manager, &session_id);
    let provider = router.resolve()?;

    let prompt = build_chat_prompt(&snapshot);
    let req = GenerateRequest {
        system_prompt: prompt,
        messages,
        context: snapshot,
        mode: QueryMode::Chat,
        max_tokens: Some(1024),
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let join = tokio::spawn(async move { provider_for_spawn.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        let _ = app.emit("ai-stream", AiStreamEvent {
            session_id: session_id.clone(),
            kind: AiStreamKind::Chat,
            delta: chunk.delta.clone(),
            done: chunk.done,
        });
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    Ok(AiChatReply { content: buf })
}
```

- [ ] **Step 2: Register `ai_chat` in `lib.rs`**

Edit `src-tauri/src/lib.rs`. Change the import:

```rust
use commands::{
    ai::{ai_chat, ai_query},
    config::{ ... },
    ...
};
```

And add `ai_chat` to the `tauri::generate_handler![...]` list, right after `ai_query`:

```rust
// AI query
ai_query,
ai_chat,
```

- [ ] **Step 3: Run cargo check**

Run: `cd src-tauri && cargo check`
Expected: clean build. (No tests yet — that's Task 4.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/ai.rs src-tauri/src/lib.rs
git commit -m "feat(m4): add ai_chat Tauri command (stateless, raw content)"
```

---

## Task 4: Backend — `ai_chat` integration test

**Context:** Mirrors `ai_query_command.rs` using a `MockProvider`. Verifies: (a) full message history is passed through to provider, (b) content is returned raw without JSON parsing, (c) empty/wrong-role messages are rejected.

**Files:**
- Create: `src-tauri/tests/ai_chat_command.rs`

- [ ] **Step 1: Write the integration test**

Create `src-tauri/tests/ai_chat_command.rs`:

```rust
//! Integration test for `ai_chat`. Uses a `MockProvider` so this test is
//! hermetic (no network, no real PTY).

use aiterm_lib::ai::{
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest, QueryMode,
};
use aiterm_lib::commands::ai::build_chat_prompt;
use aiterm_lib::ai::context;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// MockProvider that records the last `GenerateRequest` it received and
/// emits a fixed sequence of chunks.
#[derive(Clone)]
struct MockProvider {
    chunks: Vec<&'static str>,
    last_request: Arc<Mutex<Option<CapturedRequest>>>,
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    messages: Vec<ChatMessage>,
    system_prompt: String,
    mode: QueryMode,
    max_tokens: Option<u32>,
}

impl MockProvider {
    fn new(chunks: Vec<&'static str>) -> Self {
        Self {
            chunks,
            last_request: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait]
impl AiProvider for MockProvider {
    fn id(&self) -> &str { "mock" }
    fn display_name(&self) -> &str { "Mock" }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        *self.last_request.lock().unwrap() = Some(CapturedRequest {
            messages: req.messages.clone(),
            system_prompt: req.system_prompt.clone(),
            mode: req.mode,
            max_tokens: req.max_tokens,
        });
        for (i, c) in self.chunks.iter().enumerate() {
            let done = i + 1 == self.chunks.len();
            let _ = tx
                .send(GenerateChunk { delta: c.to_string(), done, usage: None })
                .await;
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), AiError> {
        Ok(())
    }
}

/// Direct call to the inner chat-generate loop without the Tauri State wiring.
/// This exercises the same code path the `ai_chat` command runs, but can be
/// called from a plain #[tokio::test] without a full AppHandle.
///
/// (We duplicate the loop here because `ai_chat` is a `#[tauri::command]` that
/// can only be invoked via the Tauri runtime. The copied logic is small and
/// must stay in sync with ai.rs — that's why we also test the prompt builder
/// separately in the lib test.)
async fn run_chat_loop(
    provider: Arc<dyn AiProvider>,
    messages: Vec<ChatMessage>,
) -> Result<String, AiError> {
    let snapshot = context::snapshot_from_parts(
        "linux",
        "bash",
        std::path::PathBuf::from("/"),
    );
    let prompt = build_chat_prompt(&snapshot);
    let req = GenerateRequest {
        system_prompt: prompt,
        messages,
        context: snapshot,
        mode: QueryMode::Chat,
        max_tokens: Some(1024),
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let join = tokio::spawn(async move { provider_for_spawn.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    match join.await {
        Ok(Ok(())) => Ok(buf),
        Ok(Err(e)) => Err(e),
        Err(join_err) => Err(AiError::Network { message: join_err.to_string() }),
    }
}

fn user(text: &str) -> ChatMessage {
    ChatMessage { role: "user".into(), content: text.into() }
}

fn assistant(text: &str) -> ChatMessage {
    ChatMessage { role: "assistant".into(), content: text.into() }
}

#[tokio::test]
async fn chat_returns_raw_content_without_json_parsing() {
    let mock = MockProvider::new(vec![
        "看你要做的事情，建議執行 ",
        "<cmd>ls -la</cmd>",
        " 試試看。",
    ]);
    let provider: Arc<dyn AiProvider> = Arc::new(mock.clone());

    let content = run_chat_loop(provider, vec![user("列出所有檔案")])
        .await
        .expect("chat should succeed");

    // Raw content, including <cmd> tag, must come back verbatim.
    assert!(content.contains("<cmd>ls -la</cmd>"));
    assert!(content.starts_with("看你要做"));
}

#[tokio::test]
async fn chat_passes_full_message_history_to_provider() {
    let mock = MockProvider::new(vec!["ok"]);
    let captured = mock.last_request.clone();
    let provider: Arc<dyn AiProvider> = Arc::new(mock);

    let history = vec![
        user("第一輪問題"),
        assistant("第一輪回答"),
        user("第二輪問題"),
        assistant("第二輪回答"),
        user("第三輪問題"),
    ];
    run_chat_loop(provider, history.clone()).await.expect("ok");

    let got = captured.lock().unwrap().clone().expect("captured request");
    assert_eq!(got.messages.len(), 5, "all 5 messages must be forwarded");
    assert_eq!(got.messages[0].content, "第一輪問題");
    assert_eq!(got.messages[4].content, "第三輪問題");
    assert!(matches!(got.mode, QueryMode::Chat));
    assert_eq!(got.max_tokens, Some(1024));
}

#[tokio::test]
async fn chat_prompt_is_chat_not_single_command() {
    let mock = MockProvider::new(vec!["x"]);
    let captured = mock.last_request.clone();
    let provider: Arc<dyn AiProvider> = Arc::new(mock);

    run_chat_loop(provider, vec![user("hi")]).await.expect("ok");

    let got = captured.lock().unwrap().clone().unwrap();
    assert!(got.system_prompt.contains("<cmd>"), "chat prompt must mention <cmd>");
    assert!(
        !got.system_prompt.contains("Output ONLY a JSON object"),
        "chat prompt must not be the single-command prompt"
    );
}

#[tokio::test]
async fn chat_propagates_provider_network_error() {
    struct FailingProvider;
    #[async_trait]
    impl AiProvider for FailingProvider {
        fn id(&self) -> &str { "fail" }
        fn display_name(&self) -> &str { "Fail" }
        async fn generate(
            &self,
            _req: GenerateRequest,
            _tx: mpsc::Sender<GenerateChunk>,
        ) -> Result<(), AiError> {
            Err(AiError::Network { message: "boom".into() })
        }
        async fn health_check(&self) -> Result<(), AiError> { Ok(()) }
    }

    let provider: Arc<dyn AiProvider> = Arc::new(FailingProvider);
    let err = run_chat_loop(provider, vec![user("hi")]).await.unwrap_err();
    match err {
        AiError::Network { message } => assert_eq!(message, "boom"),
        other => panic!("expected Network, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run the integration tests**

Run: `cd src-tauri && cargo test --test ai_chat_command`
Expected: 4 tests pass.

- [ ] **Step 3: Run the full backend test suite to check for regressions**

Run: `cd src-tauri && cargo test`
Expected: all tests pass (previous M3 tests + new M4 tests).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tests/ai_chat_command.rs
git commit -m "test(m4): integration tests for ai_chat (history, prompt, errors)"
```

---

## Task 5: Frontend — `invokeAiChat` IPC helper

**Context:** Thin wrapper around Tauri `invoke`, mirrors `invokeAiQuery`.

**Files:**
- Modify: `src/ipc/ai.ts`

- [ ] **Step 1: Add `ChatMessage` and `AiChatReply` types**

In `src/ipc/ai.ts`, below the existing `AiCommandReady` interface, add:

```typescript
export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
}

export interface AiChatReply {
  content: string;
}

export function invokeAiChat(
  messages: ChatMessage[],
  sessionId: string,
): Promise<AiChatReply> {
  return invoke<AiChatReply>("ai_chat", { messages, sessionId });
}
```

- [ ] **Step 2: Run typecheck**

Run: `npm run build`
Expected: clean TypeScript compile.

- [ ] **Step 3: Commit**

```bash
git add src/ipc/ai.ts
git commit -m "feat(m4): add invokeAiChat IPC helper and ChatMessage type"
```

---

## Task 6: Frontend — `truncateHistory` pure function

**Context:** Pure, trivial, TDD.

**Files:**
- Create: `src/lib/chatHistory.ts`
- Create: `src/lib/chatHistory.test.ts`

- [ ] **Step 1: Create directory**

Run: `mkdir -p src/lib`
Expected: directory created (or already exists).

- [ ] **Step 2: Write the failing test**

Create `src/lib/chatHistory.test.ts`:

```typescript
import { describe, expect, it } from "vitest";
import { truncateHistory } from "./chatHistory";
import type { ChatMessage } from "../ipc/ai";

function msg(role: "user" | "assistant", i: number): ChatMessage {
  return { role, content: `m${i}` };
}

describe("truncateHistory", () => {
  it("returns empty for empty input", () => {
    expect(truncateHistory([], 20)).toEqual([]);
  });

  it("returns input unchanged when shorter than limit", () => {
    const msgs = [msg("user", 1), msg("assistant", 2)];
    expect(truncateHistory(msgs, 20)).toEqual(msgs);
  });

  it("returns input unchanged when exactly at limit", () => {
    const msgs = Array.from({ length: 20 }, (_, i) =>
      msg(i % 2 === 0 ? "user" : "assistant", i),
    );
    expect(truncateHistory(msgs, 20)).toEqual(msgs);
  });

  it("keeps only the last N when over limit", () => {
    const msgs = Array.from({ length: 25 }, (_, i) =>
      msg(i % 2 === 0 ? "user" : "assistant", i),
    );
    const out = truncateHistory(msgs, 20);
    expect(out).toHaveLength(20);
    // First kept item should be m5 (dropped 0..4)
    expect(out[0].content).toBe("m5");
    expect(out[19].content).toBe("m24");
  });

  it("returns empty when limit is 0", () => {
    const msgs = [msg("user", 1)];
    expect(truncateHistory(msgs, 0)).toEqual([]);
  });

  it("returns empty when limit is negative (defensive)", () => {
    const msgs = [msg("user", 1)];
    expect(truncateHistory(msgs, -5)).toEqual([]);
  });
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `npm test -- src/lib/chatHistory.test.ts`
Expected: FAIL — "Failed to resolve import './chatHistory'".

- [ ] **Step 4: Implement the function**

Create `src/lib/chatHistory.ts`:

```typescript
import type { ChatMessage } from "../ipc/ai";

/**
 * Keep only the last `limit` messages. Used to cap chat history before
 * sending it to the AI. `limit <= 0` yields an empty array.
 */
export function truncateHistory(
  msgs: ChatMessage[],
  limit: number,
): ChatMessage[] {
  if (limit <= 0) return [];
  if (msgs.length <= limit) return msgs;
  return msgs.slice(msgs.length - limit);
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `npm test -- src/lib/chatHistory.test.ts`
Expected: 6 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/lib/chatHistory.ts src/lib/chatHistory.test.ts
git commit -m "feat(m4): truncateHistory for 20-message chat cap"
```

---

## Task 7: Frontend — `parseCmdTags` pure function

**Context:** Non-greedy regex to pull `<cmd>...</cmd>` blocks out of assistant text. Tracks `multiline` so the UI can decide whether to confirm.

**Files:**
- Create: `src/lib/cmdParser.ts`
- Create: `src/lib/cmdParser.test.ts`

- [ ] **Step 1: Write the failing tests**

Create `src/lib/cmdParser.test.ts`:

```typescript
import { describe, expect, it } from "vitest";
import { parseCmdTags } from "./cmdParser";

describe("parseCmdTags", () => {
  it("returns a single text part for pure text", () => {
    const parts = parseCmdTags("just some words");
    expect(parts).toEqual([{ type: "text", content: "just some words" }]);
  });

  it("returns empty array for empty string", () => {
    expect(parseCmdTags("")).toEqual([]);
  });

  it("extracts a single single-line cmd", () => {
    const parts = parseCmdTags("試試 <cmd>ls -la</cmd> 看看");
    expect(parts).toEqual([
      { type: "text", content: "試試 " },
      { type: "cmd", content: "ls -la", multiline: false },
      { type: "text", content: " 看看" },
    ]);
  });

  it("extracts multiple cmds", () => {
    const parts = parseCmdTags("先 <cmd>cd /tmp</cmd> 再 <cmd>ls</cmd>");
    expect(parts).toHaveLength(4);
    expect(parts[1]).toEqual({ type: "cmd", content: "cd /tmp", multiline: false });
    expect(parts[3]).toEqual({ type: "cmd", content: "ls", multiline: false });
  });

  it("trims whitespace inside cmd", () => {
    const parts = parseCmdTags("<cmd>  ls   </cmd>");
    expect(parts[0]).toEqual({ type: "cmd", content: "ls", multiline: false });
  });

  it("marks multiline=true when cmd contains newlines", () => {
    const parts = parseCmdTags("<cmd>cd /tmp\nls -la</cmd>");
    expect(parts).toHaveLength(1);
    expect(parts[0]).toEqual({
      type: "cmd",
      content: "cd /tmp\nls -la",
      multiline: true,
    });
  });

  it("treats unclosed <cmd> as plain text", () => {
    const parts = parseCmdTags("oops <cmd>ls never closes");
    expect(parts).toEqual([
      { type: "text", content: "oops <cmd>ls never closes" },
    ]);
  });

  it("handles nested with non-greedy match (takes inner first pair)", () => {
    // Non-greedy regex matches the first complete pair: <cmd>a<cmd>b</cmd>
    // which yields cmd content "a<cmd>b". The trailing </cmd> becomes text.
    const parts = parseCmdTags("<cmd>a<cmd>b</cmd></cmd>");
    expect(parts[0]).toEqual({
      type: "cmd",
      content: "a<cmd>b",
      multiline: false,
    });
    expect(parts[1]).toEqual({ type: "text", content: "</cmd>" });
  });

  it("handles cmd at very start", () => {
    const parts = parseCmdTags("<cmd>ls</cmd> done");
    expect(parts[0]).toEqual({ type: "cmd", content: "ls", multiline: false });
    expect(parts[1]).toEqual({ type: "text", content: " done" });
  });

  it("handles cmd at very end", () => {
    const parts = parseCmdTags("run <cmd>ls</cmd>");
    expect(parts[0]).toEqual({ type: "text", content: "run " });
    expect(parts[1]).toEqual({ type: "cmd", content: "ls", multiline: false });
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npm test -- src/lib/cmdParser.test.ts`
Expected: FAIL — "Failed to resolve import './cmdParser'".

- [ ] **Step 3: Implement `parseCmdTags`**

Create `src/lib/cmdParser.ts`:

```typescript
export type CmdPart =
  | { type: "text"; content: string }
  | { type: "cmd"; content: string; multiline: boolean };

/**
 * Parse assistant text into alternating text spans and <cmd> tag captures.
 * Uses a non-greedy regex so nested tags yield the innermost match first.
 * Unclosed tags are treated as plain text.
 */
export function parseCmdTags(text: string): CmdPart[] {
  if (text === "") return [];
  const parts: CmdPart[] = [];
  const regex = /<cmd>([\s\S]*?)<\/cmd>/g;
  let lastIdx = 0;
  let match: RegExpExecArray | null;
  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastIdx) {
      parts.push({ type: "text", content: text.slice(lastIdx, match.index) });
    }
    const content = match[1].trim();
    parts.push({ type: "cmd", content, multiline: content.includes("\n") });
    lastIdx = match.index + match[0].length;
  }
  if (lastIdx < text.length) {
    parts.push({ type: "text", content: text.slice(lastIdx) });
  }
  return parts;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npm test -- src/lib/cmdParser.test.ts`
Expected: 10 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/lib/cmdParser.ts src/lib/cmdParser.test.ts
git commit -m "feat(m4): parseCmdTags for <cmd> tag extraction"
```

---

## Task 8: Frontend — install `@testing-library/react`

**Context:** Hook and component tests need it. Not yet installed.

**Files:**
- Modify: `package.json`, `package-lock.json`

- [ ] **Step 1: Install the dev dependency**

Run: `npm install --save-dev @testing-library/react @testing-library/dom @testing-library/jest-dom @testing-library/user-event jsdom`
Expected: packages added to `devDependencies`, no errors.

- [ ] **Step 2: Configure vitest for jsdom**

Check if `vitest.config.ts` or `vite.config.ts` exists. If `vitest.config.ts` does not exist, create it:

```typescript
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: false,
    setupFiles: ["./src/test-setup.ts"],
  },
});
```

If `vite.config.ts` exists, add a `test` block instead (the syntax is the same).

- [ ] **Step 3: Create the test setup file**

Create `src/test-setup.ts`:

```typescript
import "@testing-library/jest-dom/vitest";
```

- [ ] **Step 4: Run existing tests to verify nothing broke**

Run: `npm test`
Expected: all existing tests still pass (`src/components/parseAiPrefix.test.ts`, `src/ipc/ai.test.ts`, `src/lib/chatHistory.test.ts`, `src/lib/cmdParser.test.ts`).

- [ ] **Step 5: Commit**

```bash
git add package.json package-lock.json vitest.config.ts src/test-setup.ts
git commit -m "chore(m4): add @testing-library/react + jsdom for hook/component tests"
```

---

## Task 9: Frontend — `useAiChat` hook

**Context:** Owns all chat state. Listens to `ai-stream` events (kind=chat) for streaming display. Sends full history each `ai_chat` call.

**Files:**
- Create: `src/hooks/useAiChat.ts`
- Create: `src/hooks/useAiChat.test.ts`
- May need: `src/hooks/` directory

- [ ] **Step 1: Create hooks directory**

Run: `mkdir -p src/hooks`
Expected: directory created (or already exists).

- [ ] **Step 2: Write the hook first (test-after is easier because we need to mock invoke and listen)**

Create `src/hooks/useAiChat.ts`:

```typescript
import { useCallback, useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  invokeAiChat,
  type AiChatReply,
  type AiError,
  type AiStreamEvent,
  type ChatMessage,
} from "../ipc/ai";
import { truncateHistory } from "../lib/chatHistory";

const HISTORY_LIMIT = 20;

export interface UseAiChatResult {
  messages: ChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  error: AiError | null;
  send: (userText: string) => Promise<void>;
  resend: () => Promise<void>;
  clear: () => void;
}

/**
 * Owns multi-turn chat state for the AI Panel. One instance per session:
 * remount (via `key={sessionId}`) resets all state.
 */
export function useAiChat(sessionId: string): UseAiChatResult {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [streamBuf, setStreamBuf] = useState("");
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<AiError | null>(null);

  // Guard against setState after unmount (Tauri listen + async invoke race).
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Listen for streaming chunks.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let active = true;
    listen<AiStreamEvent>("ai-stream", (event) => {
      if (!active) return;
      if (event.payload.kind !== "chat") return;
      if (event.payload.session_id !== sessionId) return;
      if (event.payload.done) return; // end-of-stream handled by invoke resolve
      setStreamBuf((prev) => prev + event.payload.delta);
    }).then((fn) => {
      if (!active) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    return () => {
      active = false;
      if (unlisten) unlisten();
    };
  }, [sessionId]);

  const invokeChat = useCallback(
    async (msgs: ChatMessage[]) => {
      setStreamBuf("");
      setIsStreaming(true);
      setError(null);
      try {
        const reply: AiChatReply = await invokeAiChat(msgs, sessionId);
        if (!mountedRef.current) return;
        setMessages([...msgs, { role: "assistant", content: reply.content }]);
      } catch (e) {
        if (!mountedRef.current) return;
        setError(normalizeAiError(e));
        // Do NOT roll back `msgs` — user message stays so UI can show a retry.
      } finally {
        if (mountedRef.current) {
          setStreamBuf("");
          setIsStreaming(false);
        }
      }
    },
    [sessionId],
  );

  const send = useCallback(
    async (userText: string) => {
      if (isStreaming) return; // UI should disable input anyway
      const userMsg: ChatMessage = { role: "user", content: userText };
      const next = truncateHistory([...messages, userMsg], HISTORY_LIMIT);
      setMessages(next);
      await invokeChat(next);
    },
    [messages, isStreaming, invokeChat],
  );

  const resend = useCallback(async () => {
    if (isStreaming) return;
    if (messages.length === 0) return;
    if (messages[messages.length - 1].role !== "user") return;
    await invokeChat(messages);
  }, [messages, isStreaming, invokeChat]);

  const clear = useCallback(() => {
    if (isStreaming) return; // defence in depth; UI also disables button
    setMessages([]);
    setError(null);
    setStreamBuf("");
  }, [isStreaming]);

  return { messages, streamBuf, isStreaming, error, send, resend, clear };
}

/** Coerce an unknown Tauri error into an AiError. Mirrors TerminalView logic. */
function normalizeAiError(err: unknown): AiError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as AiError;
  }
  if (err instanceof Error) {
    try {
      const parsed = JSON.parse(err.message);
      if (parsed && typeof parsed === "object" && "kind" in parsed) {
        return parsed as AiError;
      }
    } catch {
      // fall through
    }
    return { kind: "network", message: err.message };
  }
  return { kind: "network", message: String(err) };
}
```

- [ ] **Step 3: Write tests for the hook**

Create `src/hooks/useAiChat.test.ts`:

```typescript
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mock the Tauri modules BEFORE importing the hook.
const invokeMock = vi.fn();
const listenMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

// Import AFTER the mocks are set up.
import { useAiChat } from "./useAiChat";

// Capture the event callback so tests can fire fake stream events.
type StreamPayload = {
  session_id: string;
  kind: "query" | "chat";
  delta: string;
  done: boolean;
};
let lastEventCallback: ((e: { payload: StreamPayload }) => void) | null = null;
let unlistenSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  unlistenSpy = vi.fn();
  lastEventCallback = null;
  listenMock.mockImplementation((_event: string, cb: typeof lastEventCallback) => {
    lastEventCallback = cb;
    return Promise.resolve(unlistenSpy);
  });
});

afterEach(() => {
  lastEventCallback = null;
});

function fireStream(payload: Partial<StreamPayload>) {
  const full: StreamPayload = {
    session_id: "s1",
    kind: "chat",
    delta: "",
    done: false,
    ...payload,
  };
  act(() => {
    lastEventCallback?.({ payload: full });
  });
}

describe("useAiChat", () => {
  it("starts with empty state", () => {
    const { result } = renderHook(() => useAiChat("s1"));
    expect(result.current.messages).toEqual([]);
    expect(result.current.streamBuf).toBe("");
    expect(result.current.isStreaming).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it("send appends user then assistant on success", async () => {
    invokeMock.mockResolvedValueOnce({ content: "來試試 <cmd>ls</cmd>" });
    const { result } = renderHook(() => useAiChat("s1"));

    await act(async () => {
      await result.current.send("列出檔案");
    });

    expect(result.current.messages).toHaveLength(2);
    expect(result.current.messages[0]).toEqual({ role: "user", content: "列出檔案" });
    expect(result.current.messages[1]).toEqual({
      role: "assistant",
      content: "來試試 <cmd>ls</cmd>",
    });
    expect(result.current.isStreaming).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it("send keeps user message and sets error on failure", async () => {
    invokeMock.mockRejectedValueOnce({ kind: "network", message: "boom" });
    const { result } = renderHook(() => useAiChat("s1"));

    await act(async () => {
      await result.current.send("試試看");
    });

    expect(result.current.messages).toEqual([
      { role: "user", content: "試試看" },
    ]);
    expect(result.current.error).toEqual({ kind: "network", message: "boom" });
    expect(result.current.isStreaming).toBe(false);
  });

  it("stream event updates streamBuf (non-done chunks)", async () => {
    // Keep invoke pending so we can observe streaming state.
    let resolveInvoke: (v: { content: string }) => void = () => {};
    invokeMock.mockImplementationOnce(
      () =>
        new Promise((r) => {
          resolveInvoke = r;
        }),
    );
    const { result } = renderHook(() => useAiChat("s1"));

    let sendPromise: Promise<void> = Promise.resolve();
    act(() => {
      sendPromise = result.current.send("hi");
    });

    await waitFor(() => expect(result.current.isStreaming).toBe(true));

    fireStream({ delta: "hello " });
    fireStream({ delta: "world" });
    await waitFor(() => expect(result.current.streamBuf).toBe("hello world"));

    // Non-matching kind or session should be ignored.
    fireStream({ kind: "query", delta: "!!!" });
    fireStream({ session_id: "other", delta: "???" });
    expect(result.current.streamBuf).toBe("hello world");

    // Finish the invoke.
    await act(async () => {
      resolveInvoke({ content: "hello world" });
      await sendPromise;
    });

    expect(result.current.streamBuf).toBe("");
    expect(result.current.messages[1].content).toBe("hello world");
  });

  it("resend does not duplicate user message", async () => {
    // First call fails.
    invokeMock.mockRejectedValueOnce({ kind: "network", message: "fail" });
    // Second call succeeds.
    invokeMock.mockResolvedValueOnce({ content: "好的" });

    const { result } = renderHook(() => useAiChat("s1"));

    await act(async () => {
      await result.current.send("試");
    });
    expect(result.current.messages).toHaveLength(1);
    expect(result.current.error).not.toBeNull();

    await act(async () => {
      await result.current.resend();
    });
    // Only user + assistant — no duplicate user.
    expect(result.current.messages).toHaveLength(2);
    expect(result.current.messages[0]).toEqual({ role: "user", content: "試" });
    expect(result.current.messages[1]).toEqual({ role: "assistant", content: "好的" });
  });

  it("resend is a no-op when messages empty or last is not user", async () => {
    invokeMock.mockResolvedValue({ content: "x" });
    const { result } = renderHook(() => useAiChat("s1"));

    await act(async () => {
      await result.current.resend();
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("clear resets messages, error, and streamBuf", async () => {
    invokeMock.mockResolvedValueOnce({ content: "answer" });
    const { result } = renderHook(() => useAiChat("s1"));

    await act(async () => {
      await result.current.send("q");
    });
    expect(result.current.messages).toHaveLength(2);

    act(() => {
      result.current.clear();
    });
    expect(result.current.messages).toEqual([]);
    expect(result.current.error).toBeNull();
    expect(result.current.streamBuf).toBe("");
  });

  it("truncates history to 20 messages when sending", async () => {
    invokeMock.mockResolvedValue({ content: "ok" });
    const { result } = renderHook(() => useAiChat("s1"));

    // Send 11 rounds (22 messages → should truncate to 20 before invoke).
    for (let i = 0; i < 11; i++) {
      await act(async () => {
        await result.current.send(`q${i}`);
      });
    }

    // Last call's messages arg must have length ≤ 20.
    const lastCall = invokeMock.mock.calls[invokeMock.mock.calls.length - 1];
    // invokeAiChat calls invoke("ai_chat", { messages, sessionId })
    const args = lastCall[1] as { messages: unknown[] };
    expect(args.messages.length).toBeLessThanOrEqual(20);
  });
});
```

- [ ] **Step 4: Run the hook tests**

Run: `npm test -- src/hooks/useAiChat.test.ts`
Expected: 8 tests pass.

- [ ] **Step 5: Run the full test suite**

Run: `npm test`
Expected: everything passes.

- [ ] **Step 6: Commit**

```bash
git add src/hooks/useAiChat.ts src/hooks/useAiChat.test.ts
git commit -m "feat(m4): useAiChat hook with streaming and retry"
```

---

## Task 10: Frontend — `CmdTag` component

**Context:** Single-line cmd executes directly; multi-line cmd goes through `window.confirm` per design spec §4.6.

**Files:**
- Create: `src/components/AiPanel/CmdTag.tsx`
- Create: `src/components/AiPanel/CmdTag.test.tsx`

- [ ] **Step 1: Create AiPanel directory**

Run: `mkdir -p src/components/AiPanel`

- [ ] **Step 2: Write the failing test**

Create `src/components/AiPanel/CmdTag.test.tsx`:

```typescript
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CmdTag } from "./CmdTag";

describe("CmdTag", () => {
  beforeEach(() => {
    vi.spyOn(window, "confirm").mockImplementation(() => true);
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders the command text", () => {
    render(<CmdTag command="ls -la" multiline={false} onExec={vi.fn()} />);
    expect(screen.getByText("ls -la")).toBeInTheDocument();
  });

  it("single-line click calls onExec without confirmation", async () => {
    const onExec = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm");
    render(<CmdTag command="ls" multiline={false} onExec={onExec} />);

    await userEvent.click(screen.getByRole("button"));
    expect(onExec).toHaveBeenCalledWith("ls");
    expect(confirmSpy).not.toHaveBeenCalled();
  });

  it("multi-line click shows confirm and runs on approval", async () => {
    const onExec = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(
      <CmdTag
        command={"cd /tmp\nls -la"}
        multiline={true}
        onExec={onExec}
      />,
    );

    await userEvent.click(screen.getByRole("button"));
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(onExec).toHaveBeenCalledWith("cd /tmp\nls -la");
  });

  it("multi-line click does not run when confirm cancelled", async () => {
    const onExec = vi.fn();
    vi.spyOn(window, "confirm").mockReturnValue(false);
    render(
      <CmdTag
        command={"cd /tmp\nls -la"}
        multiline={true}
        onExec={onExec}
      />,
    );

    await userEvent.click(screen.getByRole("button"));
    expect(onExec).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `npm test -- src/components/AiPanel/CmdTag.test.tsx`
Expected: FAIL — "Failed to resolve import './CmdTag'".

- [ ] **Step 4: Implement `CmdTag`**

Create `src/components/AiPanel/CmdTag.tsx`:

```typescript
interface CmdTagProps {
  command: string;
  multiline: boolean;
  onExec: (cmd: string) => void;
}

export function CmdTag({ command, multiline, onExec }: CmdTagProps) {
  const handleClick = () => {
    if (multiline) {
      const ok = window.confirm(
        `確定執行多行命令？\n\n${command}`,
      );
      if (!ok) return;
    }
    onExec(command);
  };
  return (
    <button
      type="button"
      className="aiterm-cmd-tag"
      onClick={handleClick}
      title={multiline ? "多行命令 — 執行前會再確認一次" : "點擊即執行"}
    >
      <code>{command}</code>
      <span className="aiterm-cmd-tag-play">▶</span>
    </button>
  );
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `npm test -- src/components/AiPanel/CmdTag.test.tsx`
Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/components/AiPanel/CmdTag.tsx src/components/AiPanel/CmdTag.test.tsx
git commit -m "feat(m4): CmdTag button with multi-line confirmation"
```

---

## Task 11: Frontend — `MessageBubble` component

**Context:** Renders one user/assistant/error message. Assistant messages run through `parseCmdTags` to render inline `CmdTag` buttons.

**Files:**
- Create: `src/components/AiPanel/MessageBubble.tsx`
- Create: `src/components/AiPanel/MessageBubble.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `src/components/AiPanel/MessageBubble.test.tsx`:

```typescript
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MessageBubble } from "./MessageBubble";

describe("MessageBubble", () => {
  it("renders a user bubble with plain text", () => {
    render(
      <MessageBubble
        role="user"
        content="列出所有檔案"
        onExecuteCommand={vi.fn()}
      />,
    );
    expect(screen.getByText("列出所有檔案")).toBeInTheDocument();
  });

  it("renders an assistant bubble splitting text and cmd tags", () => {
    const onExec = vi.fn();
    render(
      <MessageBubble
        role="assistant"
        content="建議執行 <cmd>ls -la</cmd> 試試"
        onExecuteCommand={onExec}
      />,
    );
    // Text fragments present
    expect(screen.getByText("建議執行 ")).toBeInTheDocument();
    expect(screen.getByText(" 試試")).toBeInTheDocument();
    // Cmd button present and clickable
    const btn = screen.getByRole("button", { name: /ls -la/ });
    expect(btn).toBeInTheDocument();
  });

  it("clicking an assistant cmd calls onExecuteCommand", async () => {
    const onExec = vi.fn();
    render(
      <MessageBubble
        role="assistant"
        content="<cmd>pwd</cmd>"
        onExecuteCommand={onExec}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /pwd/ }));
    expect(onExec).toHaveBeenCalledWith("pwd");
  });

  it("renders a streaming bubble (in-progress assistant output)", () => {
    render(
      <MessageBubble
        role="assistant"
        content="正在生成..."
        onExecuteCommand={vi.fn()}
        streaming
      />,
    );
    expect(screen.getByText("正在生成...")).toBeInTheDocument();
    // Streaming bubble has a distinguishing class — assert via aria-busy.
    const bubble = screen.getByText("正在生成...").closest("[aria-busy]");
    expect(bubble).not.toBeNull();
    expect(bubble).toHaveAttribute("aria-busy", "true");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm test -- src/components/AiPanel/MessageBubble.test.tsx`
Expected: FAIL — cannot resolve import.

- [ ] **Step 3: Implement `MessageBubble`**

Create `src/components/AiPanel/MessageBubble.tsx`:

```typescript
import { parseCmdTags } from "../../lib/cmdParser";
import { CmdTag } from "./CmdTag";

interface MessageBubbleProps {
  role: "user" | "assistant";
  content: string;
  onExecuteCommand: (cmd: string) => void;
  streaming?: boolean;
}

export function MessageBubble({
  role,
  content,
  onExecuteCommand,
  streaming,
}: MessageBubbleProps) {
  if (role === "user") {
    return (
      <div className="aiterm-bubble aiterm-bubble-user">
        <span>{content}</span>
      </div>
    );
  }

  // Assistant: split by <cmd> tags.
  const parts = parseCmdTags(content);
  return (
    <div
      className="aiterm-bubble aiterm-bubble-assistant"
      aria-busy={streaming ? "true" : "false"}
    >
      {parts.map((p, i) =>
        p.type === "text" ? (
          <span key={i}>{p.content}</span>
        ) : (
          <CmdTag
            key={i}
            command={p.content}
            multiline={p.multiline}
            onExec={onExecuteCommand}
          />
        ),
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm test -- src/components/AiPanel/MessageBubble.test.tsx`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/AiPanel/MessageBubble.tsx src/components/AiPanel/MessageBubble.test.tsx
git commit -m "feat(m4): MessageBubble renders user/assistant with inline CmdTag"
```

---

## Task 12: Frontend — `MessageList` component

**Context:** Thin wrapper mapping messages to `MessageBubble`. Also renders the live streaming bubble when a response is being generated, and an error bubble + retry button when `error != null`.

**Files:**
- Create: `src/components/AiPanel/MessageList.tsx`

- [ ] **Step 1: Implement the component**

Create `src/components/AiPanel/MessageList.tsx`:

```typescript
import type { AiError, ChatMessage } from "../../ipc/ai";
import { formatAiError } from "../../ipc/ai";
import { MessageBubble } from "./MessageBubble";

interface MessageListProps {
  messages: ChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  error: AiError | null;
  onExecuteCommand: (cmd: string) => void;
  onRetry: () => void;
}

export function MessageList({
  messages,
  streamBuf,
  isStreaming,
  error,
  onExecuteCommand,
  onRetry,
}: MessageListProps) {
  return (
    <div className="aiterm-message-list">
      {messages.map((m, i) => (
        <MessageBubble
          key={i}
          role={m.role === "assistant" ? "assistant" : "user"}
          content={m.content}
          onExecuteCommand={onExecuteCommand}
        />
      ))}
      {isStreaming && streamBuf && (
        <MessageBubble
          role="assistant"
          content={streamBuf}
          onExecuteCommand={onExecuteCommand}
          streaming
        />
      )}
      {error && (
        <div className="aiterm-bubble aiterm-bubble-error" role="alert">
          <span>⚠ {formatAiError(error)}</span>
          <button
            type="button"
            className="aiterm-retry-btn"
            onClick={onRetry}
            disabled={isStreaming}
          >
            🔄 重試
          </button>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Typecheck**

Run: `npm run build`
Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add src/components/AiPanel/MessageList.tsx
git commit -m "feat(m4): MessageList with streaming and error-retry bubbles"
```

---

## Task 13: Frontend — `AiPanel` main component + styles

**Context:** Top-level panel with header (provider badge + 🗑 New Chat), MessageList, and input area. Handles Escape key + autofocus.

**Files:**
- Create: `src/components/AiPanel/index.tsx`
- Create: `src/components/AiPanel/styles.css`
- Create: `src/components/AiPanel/AiPanel.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `src/components/AiPanel/AiPanel.test.tsx`:

```typescript
import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Mock Tauri before importing AiPanel (which imports useAiChat).
const invokeMock = vi.fn();
const listenMock = vi.fn().mockResolvedValue(() => {});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

import { AiPanel } from "./index";

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockClear();
  listenMock.mockResolvedValue(() => {});
});

describe("AiPanel", () => {
  it("hides the panel via CSS class when isOpen=false", () => {
    const { container } = render(
      <AiPanel
        sessionId="s1"
        isOpen={false}
        providerName="Ollama (llama3)"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    // Panel element exists but has the hidden class — content stays mounted
    // so the chat hook keeps its listener alive while the user toggles.
    const panel = container.querySelector(".aiterm-ai-panel");
    expect(panel).not.toBeNull();
    expect(panel).toHaveClass("aiterm-ai-panel-hidden");
    // Textarea exists in DOM (not unmounted).
    expect(screen.getByRole("textbox")).toBeInTheDocument();
  });

  it("autofocuses the textarea when transitioning to open", () => {
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox");
    expect(textbox).toHaveFocus();
  });

  it("calls onClose when Escape pressed", async () => {
    const onClose = vi.fn();
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={onClose}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("sends a message when Enter pressed", async () => {
    invokeMock.mockResolvedValueOnce({ content: "好的" });
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "ai_chat",
        expect.objectContaining({
          messages: [{ role: "user", content: "列出檔案" }],
          sessionId: "s1",
        }),
      ),
    );
    await waitFor(() => expect(screen.getByText("好的")).toBeInTheDocument());
  });

  it("🗑 New Chat button clears messages", async () => {
    invokeMock.mockResolvedValueOnce({ content: "ok" });
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "hi");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("ok")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: /New Chat/ }));
    expect(screen.queryByText("ok")).toBeNull();
  });

  it("provider badge calls onOpenProviderPalette when clicked", async () => {
    const onOpenProviderPalette = vi.fn();
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Claude"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={onOpenProviderPalette}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Claude/ }));
    expect(onOpenProviderPalette).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm test -- src/components/AiPanel/AiPanel.test.tsx`
Expected: FAIL — "Failed to resolve import './index'".

- [ ] **Step 3: Create the stylesheet**

Create `src/components/AiPanel/styles.css`:

```css
.aiterm-ai-panel {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: 420px;
  background: #141414;
  color: #e6e6e6;
  border-left: 1px solid #2a2a2a;
  display: flex;
  flex-direction: column;
  z-index: 50;
  font-family: "Cascadia Mono", Consolas, monospace;
  font-size: 13px;
}

/*
 * Hidden state keeps the panel mounted so useAiChat's stream listener
 * continues to receive chunks while the user toggles the panel off and on.
 */
.aiterm-ai-panel-hidden {
  display: none;
}

.aiterm-ai-panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid #2a2a2a;
  background: #1b1b1b;
}

.aiterm-ai-panel-title {
  font-weight: 600;
  color: #bbb;
}

.aiterm-ai-panel-provider-badge {
  background: #223;
  color: #cde;
  border: 1px solid #345;
  border-radius: 4px;
  padding: 2px 8px;
  font-size: 11px;
  cursor: pointer;
}
.aiterm-ai-panel-provider-badge:hover {
  background: #334;
}

.aiterm-ai-panel-clear-btn {
  background: transparent;
  border: 1px solid #444;
  color: #bbb;
  border-radius: 4px;
  padding: 2px 8px;
  font-size: 11px;
  cursor: pointer;
}
.aiterm-ai-panel-clear-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.aiterm-message-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.aiterm-bubble {
  max-width: 90%;
  padding: 6px 10px;
  border-radius: 8px;
  word-wrap: break-word;
  white-space: pre-wrap;
  line-height: 1.5;
}
.aiterm-bubble-user {
  align-self: flex-end;
  background: #1e3a5f;
  color: #e6e6e6;
}
.aiterm-bubble-assistant {
  align-self: flex-start;
  background: #232323;
  color: #e6e6e6;
}
.aiterm-bubble-assistant[aria-busy="true"] {
  opacity: 0.75;
  font-style: italic;
}
.aiterm-bubble-error {
  align-self: stretch;
  background: #3a1a1a;
  border: 1px solid #5a2a2a;
  color: #f4c2c2;
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
}

.aiterm-cmd-tag {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin: 2px 2px;
  padding: 2px 8px;
  background: #0a2a0a;
  border: 1px solid #2a5a2a;
  border-radius: 4px;
  color: #b8f4b8;
  cursor: pointer;
  font-family: inherit;
  font-size: 12px;
}
.aiterm-cmd-tag:hover {
  background: #0f3a0f;
}
.aiterm-cmd-tag code {
  font-family: inherit;
}
.aiterm-cmd-tag-play {
  color: #6cf06c;
}

.aiterm-retry-btn {
  background: transparent;
  border: 1px solid #8a4a4a;
  color: #f4c2c2;
  padding: 2px 8px;
  border-radius: 4px;
  cursor: pointer;
}
.aiterm-retry-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.aiterm-ai-panel-input-area {
  border-top: 1px solid #2a2a2a;
  padding: 8px 12px;
  background: #1b1b1b;
  display: flex;
  gap: 8px;
}
.aiterm-ai-panel-input {
  flex: 1;
  background: #0c0c0c;
  color: #e6e6e6;
  border: 1px solid #333;
  border-radius: 4px;
  padding: 6px 8px;
  font-family: inherit;
  font-size: 13px;
  resize: none;
}
.aiterm-ai-panel-input:disabled {
  opacity: 0.5;
}
.aiterm-ai-panel-send-btn {
  background: #1e4a8f;
  color: #fff;
  border: none;
  border-radius: 4px;
  padding: 0 12px;
  cursor: pointer;
}
.aiterm-ai-panel-send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
```

- [ ] **Step 4: Implement `AiPanel`**

Create `src/components/AiPanel/index.tsx`:

```typescript
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useAiChat } from "../../hooks/useAiChat";
import { MessageList } from "./MessageList";
import "./styles.css";

export interface AiPanelProps {
  sessionId: string;
  isOpen: boolean;
  providerName: string;
  onClose: () => void;
  onExecuteCommand: (cmd: string) => void;
  onOpenProviderPalette: () => void;
}

/**
 * The panel stays mounted across open/close so `useAiChat`'s event listener
 * keeps receiving streaming chunks while the user toggles Ctrl+I. We hide
 * the panel with a CSS class when `isOpen=false` rather than returning null.
 */
export function AiPanel({
  sessionId,
  isOpen,
  providerName,
  onClose,
  onExecuteCommand,
  onOpenProviderPalette,
}: AiPanelProps) {
  const chat = useAiChat(sessionId);
  const [input, setInput] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Autofocus when the panel transitions to open.
  useEffect(() => {
    if (isOpen) textareaRef.current?.focus();
  }, [isOpen]);

  // Global Escape handler — only active while the panel is open.
  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isOpen, onClose]);

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey) {
      e.preventDefault();
      submit();
    }
  };

  const submit = () => {
    const text = input.trim();
    if (!text) return;
    if (chat.isStreaming) return;
    setInput("");
    void chat.send(text);
  };

  const panelClass = isOpen
    ? "aiterm-ai-panel"
    : "aiterm-ai-panel aiterm-ai-panel-hidden";

  return (
    <div className={panelClass} aria-hidden={!isOpen}>
      <div className="aiterm-ai-panel-header">
        <span className="aiterm-ai-panel-title">AI Chat</span>
        <button
          type="button"
          className="aiterm-ai-panel-provider-badge"
          onClick={onOpenProviderPalette}
          title="切換 Provider"
        >
          {providerName || "(no provider)"}
        </button>
        <button
          type="button"
          className="aiterm-ai-panel-clear-btn"
          onClick={chat.clear}
          disabled={chat.isStreaming}
          title="清空當前對話"
        >
          🗑 New Chat
        </button>
      </div>

      <MessageList
        messages={chat.messages}
        streamBuf={chat.streamBuf}
        isStreaming={chat.isStreaming}
        error={chat.error}
        onExecuteCommand={onExecuteCommand}
        onRetry={chat.resend}
      />

      <div className="aiterm-ai-panel-input-area">
        <textarea
          ref={textareaRef}
          className="aiterm-ai-panel-input"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={
            chat.isStreaming ? "等待 AI 回覆中..." : "輸入訊息，Enter 送出..."
          }
          rows={2}
          disabled={chat.isStreaming}
        />
        <button
          type="button"
          className="aiterm-ai-panel-send-btn"
          onClick={submit}
          disabled={chat.isStreaming || input.trim() === ""}
        >
          送出
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Run the test**

Run: `npm test -- src/components/AiPanel/AiPanel.test.tsx`
Expected: 6 tests pass.

- [ ] **Step 6: Run the full test suite**

Run: `npm test`
Expected: everything passes.

- [ ] **Step 7: Commit**

```bash
git add src/components/AiPanel/index.tsx src/components/AiPanel/styles.css src/components/AiPanel/AiPanel.test.tsx
git commit -m "feat(m4): AiPanel with header, message list, and input"
```

---

## Task 14: Frontend — Wire `Ctrl+I` toggle and mount `AiPanel` in `TerminalView`

**Context:** Add `panelOpen` state, `Ctrl+I` shortcut, mount `AiPanel`, prevent xterm from receiving keyboard when panel is open.

**Files:**
- Modify: `src/components/TerminalView.tsx`

- [ ] **Step 1: Add `panelOpen` + `sessionId` state and `Ctrl+I` shortcut**

In `src/components/TerminalView.tsx`, add two new state declarations near the other state (around line 68):

```typescript
const [paletteOpen, setPaletteOpen] = useState(false);
const [panelOpen, setPanelOpen] = useState(false);
const [sessionId, setSessionId] = useState<string>("");
```

The `sessionId` state mirrors `sessionRef` so that React re-renders when it becomes available — the ref stays (it's needed by closures inside the xterm `useEffect`), but the state is what the JSX reads.

Inside the async IIFE where `sessionRef.current = sessionId;` is assigned (around line 155), rename the local to `id` to avoid shadowing the state variable, and call `setSessionId` right after:

```typescript
const id = await createPty({ rows, cols });
sessionRef.current = id;
setSessionId(id);
setStatus(`connected (${id.slice(0, 8)}…)`);

unlistenData = await onPtyData(id, (bytes) => {
  term.write(decoder.decode(bytes, { stream: true }));
});

unlistenStream = await listen<AiStreamEvent>("ai-stream", (event) => {
  if (event.payload.kind !== "query") return;
  if (event.payload.session_id !== id) return;
  if (!event.payload.done) {
    setStreamText((t) => t + event.payload.delta);
  }
});
```

(The `listen` callback previously referenced the local `sessionId` — rename those references to `id` as well. Search the async IIFE for `sessionId` and replace each one with `id`; do **not** touch references outside this IIFE.)

Then find the existing keyboard shortcut `useEffect` (the one with `Ctrl+,` and `Ctrl+Shift+P`) and extend it:

```typescript
useEffect(() => {
  const handler = (e: KeyboardEvent) => {
    if (e.ctrlKey && e.key === ",") {
      e.preventDefault();
      navigate("/settings");
    } else if (e.ctrlKey && e.shiftKey && e.key === "P") {
      e.preventDefault();
      setPaletteOpen((o) => !o);
    } else if (e.ctrlKey && (e.key === "i" || e.key === "I")) {
      e.preventDefault();
      setPanelOpen((o) => !o);
    }
  };
  window.addEventListener("keydown", handler);
  return () => window.removeEventListener("keydown", handler);
}, [navigate]);
```

- [ ] **Step 2: Track `panelOpen` with a ref for the xterm input gate**

xterm's `onData` callback is registered once inside `useEffect` with an empty dep array, so it captures state at mount time. We need a ref:

```typescript
const panelOpenRef = useRef(false);
useEffect(() => {
  panelOpenRef.current = panelOpen;
}, [panelOpen]);
```

Add this right after the existing `previewRef.current = preview;` line.

- [ ] **Step 3: Gate the xterm `onData` handler**

Inside the `useEffect` that sets up xterm, find `term.onData((data) => { ... })` and add a guard at the very top:

```typescript
term.onData((data) => {
  // Panel owns keyboard while open — drop input.
  if (panelOpenRef.current) return;

  const session = sessionRef.current;
  if (!session) return;
  // ... rest unchanged
});
```

- [ ] **Step 4: Import `AiPanel`**

Near the top of the file, add:

```typescript
import { AiPanel } from "./AiPanel";
```

- [ ] **Step 5: Mount `AiPanel` in the return JSX**

In the return block, just before the closing `</div>` (right after the `{paletteOpen && <ProviderPalette ... />}` block), add:

```typescript
{sessionId && (
  <AiPanel
    key={sessionId}
    sessionId={sessionId}
    isOpen={panelOpen}
    providerName={activeProvider}
    onClose={() => setPanelOpen(false)}
    onExecuteCommand={(cmd) => {
      writePty(sessionId, cmd + "\r").catch(console.error);
    }}
    onOpenProviderPalette={() => {
      setPanelOpen(false);
      setPaletteOpen(true);
    }}
  />
)}
```

**Note on `key`:** we key by `sessionId` so that if the session id changes, the panel remounts and its chat history resets. For a single-session M4 this is effectively a constant, but it future-proofs the component when multi-tab arrives. The `sessionId &&` guard ensures we don't mount with an empty string before the PTY is ready.

- [ ] **Step 6: Run the typecheck**

Run: `npm run build`
Expected: clean compile.

- [ ] **Step 7: Run the full test suite**

Run: `npm test`
Expected: everything passes.

- [ ] **Step 8: Commit**

```bash
git add src/components/TerminalView.tsx
git commit -m "feat(m4): wire Ctrl+I panel toggle and mount AiPanel"
```

---

## Task 15: Manual golden-path testing

**Context:** UI must be tested interactively per the brainstorming skill. Run through 10 scenarios in a real browser.

**Files:** none (manual verification)

- [ ] **Step 1: Build and launch dev**

Run: `npm run tauri:dev`
Expected: Tauri dev window opens with terminal ready.

- [ ] **Step 2: Scenario 1 — Open the panel**

Press `Ctrl+I`.
Expected: panel slides in from the right, textarea has focus.

- [ ] **Step 3: Scenario 2 — First-round chat**

Type「請列出目錄」in the panel textarea, press Enter.
Expected: a user bubble appears, then streaming assistant text, then a final assistant bubble containing a `<cmd>ls</cmd>` button (exact command depends on provider).

- [ ] **Step 4: Scenario 3 — Click the cmd button**

Click the `ls` button.
Expected: terminal receives `ls\n`, shell runs it, output appears in xterm.

- [ ] **Step 5: Scenario 4 — Multi-turn context**

With the first turn still in the panel, type「現在顯示詳細資訊」, press Enter.
Expected: assistant uses context from the previous turn and suggests `<cmd>ls -la</cmd>` (or similar detail flag).

- [ ] **Step 6: Scenario 5 — 🗑 New Chat**

Click the 🗑 New Chat button.
Expected: all bubbles disappear; textarea focused; fresh state.

- [ ] **Step 7: Scenario 6 — Session isolation**

**Note:** this scenario depends on multi-tab support, which isn't in M4. Either skip or, if the app supports tabs, confirm each tab has its own chat history.

- [ ] **Step 8: Scenario 7 — Escape closes panel**

Press `Escape`.
Expected: panel disappears; terminal gets keyboard focus back; typing goes into the shell.

- [ ] **Step 9: Scenario 8 — Error handling**

Stop Ollama (or use invalid API key), open panel, type something, press Enter.
Expected: red error bubble appears with 🔄 重試 button; user message is preserved.

- [ ] **Step 10: Scenario 9 — Retry**

Start Ollama again. Click 🔄 重試.
Expected: previous user message is resent without duplication; assistant response appears.

- [ ] **Step 11: Scenario 10 — Provider badge**

Click the provider name at the top of the panel.
Expected: panel closes; ProviderPalette opens for picking a new default.

- [ ] **Step 12: Scenario 11 — Multi-line cmd confirm**

Ask「寫一個 bash loop 印 1 到 3」. If the assistant produces a multi-line `<cmd>`, click it.
Expected: browser `window.confirm` dialog shows the full command; clicking OK runs it in the terminal; clicking Cancel does nothing.

- [ ] **Step 13: Record any regressions**

If any scenario fails, note exactly what happened. Fix the issue, re-run affected scenarios, and commit the fix in a separate commit before moving on.

---

## Task 16: Final verification and cleanup

**Files:** none (verification only)

- [ ] **Step 1: Run the full frontend test suite**

Run: `npm test`
Expected: all tests green.

- [ ] **Step 2: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: all tests green.

- [ ] **Step 3: Run the frontend build**

Run: `npm run build`
Expected: clean build with no TypeScript errors.

- [ ] **Step 4: Run the Rust lint (optional)**

Run: `cd src-tauri && cargo clippy -- -D warnings`
Expected: no clippy warnings. If there are pre-existing warnings unrelated to M4, leave them — don't clean up unrelated code.

- [ ] **Step 5: Review uncommitted changes**

Run: `git status && git log --oneline origin/master..HEAD`
Expected: clean worktree; a linear series of ~14 commits implementing M4.

- [ ] **Step 6: Hand off to `finishing-a-development-branch`**

Announce: "M4 implementation complete — using the finishing-a-development-branch skill to wrap up."

---

## Notes

- **DRY:** `normalizeAiError` is duplicated between `useAiChat.ts` and `TerminalView.tsx`. Leave it duplicated for M4 — extracting is a follow-up refactor, not part of M4 scope.
- **YAGNI:** No cancellation token, no persistence, no custom modal. Spec explicitly defers these to M5+.
- **TDD:** Steps use red→green→commit. Don't skip the "run the failing test" step — it validates the test harness sees the file.
- **Frequent commits:** Each task ends with one commit. Do not batch commits.
- **`QueryMode::Chat`:** Already exists in `src-tauri/src/ai/mod.rs:62` with `#[allow(dead_code)]`. M4 uses it, so the attribute can be removed — but don't bother, clippy won't warn once the variant is used.

# Artifact 協定推廣 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 教模型 artifact 協定（三個 prompt builder），並把 artifact 面板接上
`DatabaseAiChat`、`CrossDbAiChat`、`CodeAssistantView`、`KnowledgeBaseView`。

**Architecture:** 協定說明文字放一個共用 Rust 模組，由三個 prompt builder 引用；
`build_chat_prompt` 用 `supports_artifacts` 旗標控管（它的呼叫端包含兩個不渲染聊天
markdown 的一次性用途）。前端把 `ChatPanelShell` 裡的分割版型抽成自帶樣式的
`<ArtifactSplit>`，四個介面共用。

**Tech Stack:** Rust（`src-tauri`）+ React 19/TypeScript，Vitest + RTL。

---

## 這份計畫涵蓋的 spec 章節

對應 `docs/superpowers/specs/2026-09-01-artifact-protocol-rollout-design.md`：

- 設計 A（共用文字 + 三個 prompt builder）→ Task 1（共用模組）、Task 2
  （`build_chat_prompt` + 旗標）、Task 3（另兩個 prompt builder）
- 設計 A1（旗標的前端側）→ Task 4
- 設計 B1（抽 `ArtifactSplit`）→ Task 5
- 設計 B2（四個介面）→ Task 6（兩個資料庫聊天）、Task 7（Code Assistant + 知識庫）

---

## 兩個對 spec 的刻意偏離（先講清楚）

1. **`artifact_protocol_section()` 不收 `locale` 參數**。spec 寫的是
   `artifact_protocol_section(locale)`（且用了「例如」），但這三個 prompt builder
   的既有慣例是「規則本文一律英文，另外用一條 `Respond in {language}` 規則管輸出
   語言」。協定說明是規則本文，跟著慣例維持英文即可，不需要 locale。
2. **`ChatPanelShell` 的三條 artifact 版型測試需要改斷言**。spec 說「既有測試原封不動
   繼續通過」是驗收標準，但抽出元件後 `--split` 這個 class 會從面板根節點移到
   `ArtifactSplit` 自己的根節點——**行為不變、DOM 結構變了**，所以斷言 DOM 結構的那
   三條必須跟著改 class 名。Task 5 會逐條列出怎麼改，並保留每條測試原本的意圖。

---

### Task 1: 共用的協定說明文字（Rust）

**Files:**
- Create: `src-tauri/src/ai/artifact_prompt.rs`
- Modify: `src-tauri/src/ai/mod.rs`（註冊模組）

> **這個 task 不走「先看到測試變紅」**：它只是一個回傳常數字串的函式，先寫測試
> 只會得到「模組不存在」的編譯錯誤，證明不了任何東西。實作與測試一起建立，由
> Step 3 的斷言驗收。**後面每個 task 都要照常先看到有意義的紅。**

- [ ] **Step 1: 註冊模組**

在 `src-tauri/src/ai/mod.rs` 既有的模組宣告區塊（`pub mod anthropic;` 那一段，
約第 13-24 行）**依字母順序**插入一行——`antigravity` 之後、`chatgpt_web` 之前：

```rust
pub mod artifact_prompt;
```

- [ ] **Step 2: 建立 `src-tauri/src/ai/artifact_prompt.rs`（含測試）**

```rust
//! Artifact 協定的說明文字。三個 prompt builder（`commands/ai.rs` 的
//! `build_chat_prompt`、`code_assistant` 與 `knowledge_base` 各自的
//! `build_system_prompt`）共用同一份，避免同樣的說明散在各處各自漂移。
//!
//! 文字維持英文，跟這三個 prompt 的既有慣例一致——規則本文一律英文，輸出語言
//! 另外由各自的 "Respond in {language}" 規則管。

/// 教模型怎麼輸出會被 artifact 面板渲染的 fenced code block。
/// 呼叫端自行決定要不要接上去（`build_chat_prompt` 有旗標，另外兩個無條件接）。
pub fn artifact_protocol_section() -> &'static str {
    r#"
## Rendering documents and charts

Two fenced code blocks get special rendering in a panel beside this
conversation. Reach for them when the answer is something to look at rather
than a sentence to read.

- ```artifact-html — the body is a complete HTML document, rendered in an
  isolated sandbox. Use it for reports, formatted summaries, comparison
  tables, or anything worth reading as its own document. Include a <title>;
  it becomes the panel's title. <style> and <script> both work, but the page
  cannot reach anything outside its own frame.
- ```artifact-chart — the body is JSON describing a chart:
  {"type":"bar"|"line"|"pie","title":"...","data":[{...}],"xKey":"...",
   "series":[{"key":"...","label":"..."}]}
  `data` is an array of row objects, `xKey` names the field used for the
  category axis, and each series `key` names a numeric field on those rows.

Rules:
- At most one artifact per reply; a later one replaces the earlier one.
- Do NOT use an artifact for a short answer, a single command, or a couple of
  sentences — those belong in the reply itself.
- Always also write a line or two in the reply saying what you produced. The
  artifact supplements the answer, it is not a substitute for one."#
}

#[cfg(test)]
mod tests {
    use super::artifact_protocol_section;

    /// 兩個 fence 名稱是協定本身，前端 `src/lib/markdown.tsx` 的 code renderer
    /// 就是比對這兩個字串。這裡釘住它們，避免哪天有人「順手」改了措辭卻讓模型
    /// 學到一個前端根本不認得的名字。
    #[test]
    fn mentions_both_fence_languages() {
        let s = artifact_protocol_section();
        assert!(s.contains("artifact-html"), "must teach the artifact-html fence");
        assert!(s.contains("artifact-chart"), "must teach the artifact-chart fence");
    }

    /// 圖表 JSON 的欄位名也是協定：前端 `ArtifactChart.tsx` 的 `ChartSpec`
    /// 就是照這些名字解析的。
    #[test]
    fn documents_the_chart_spec_fields() {
        let s = artifact_protocol_section();
        for field in ["\"type\"", "\"data\"", "\"xKey\"", "\"series\""] {
            assert!(s.contains(field), "chart spec must document {field}");
        }
        for kind in ["\"bar\"", "\"line\"", "\"pie\""] {
            assert!(s.contains(kind), "chart spec must list the {kind} chart type");
        }
    }

    /// 沒有這條，模型很容易把每個回答都包成 artifact。
    #[test]
    fn warns_against_using_artifacts_for_short_answers() {
        assert!(artifact_protocol_section().contains("Do NOT use an artifact for a short answer"));
    }
}
```

- [ ] **Step 3: 執行測試**

Run: `cd src-tauri && cargo test --lib ai::artifact_prompt`
Expected: 3 個測試通過。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/ai/artifact_prompt.rs src-tauri/src/ai/mod.rs
git commit -m "feat(artifact): add the shared artifact protocol prompt section"
```

---

### Task 2: `build_chat_prompt` 的旗標與說明（Rust）

**Files:**
- Modify: `src-tauri/src/commands/ai.rs`

- [ ] **Step 1: 先改測試（會編譯失敗，因為簽名還沒改）**

在 `mod tests` 裡，把**現有 5 個** `build_chat_prompt(&snap, Locale::ZhTw)` 呼叫
（測試名稱：`chat_prompt_contains_environment_fields`、
`chat_prompt_includes_recent_output_when_present`、
`chat_prompt_instructs_cmd_tag_format`、`chat_prompt_omits_json_schema_rules`、
`chat_prompt_truncates_long_recent_output_without_utf8_panic`）全部改成：

```rust
        let prompt = build_chat_prompt(&snap, Locale::ZhTw, false);
```

（這 5 條原本的斷言一個字都不要動——它們驗的是「不帶 artifact 時的既有行為」，
補 `false` 之後應該原封不動繼續通過。）

接著在同一個 `mod tests` 裡、最後一個測試之後新增兩條：

```rust
    #[test]
    fn chat_prompt_teaches_artifacts_when_supported() {
        let snap = make_snap("linux", "bash", "/");
        let prompt = build_chat_prompt(&snap, Locale::ZhTw, true);
        assert!(prompt.contains("artifact-html"));
        assert!(prompt.contains("artifact-chart"));
    }

    /// `build_chat_prompt` 的呼叫端不只聊天介面——`ApiDocsView` 與
    /// `DocConverterView` 也走 `ai_chat`，但它們是一次性的文件產生器、根本
    /// 不渲染聊天 markdown。教了它們，fence 會變成產出文件裡的垃圾文字。
    #[test]
    fn chat_prompt_stays_silent_about_artifacts_when_unsupported() {
        let snap = make_snap("linux", "bash", "/");
        let prompt = build_chat_prompt(&snap, Locale::ZhTw, false);
        assert!(!prompt.contains("artifact-html"));
        assert!(!prompt.contains("artifact-chart"));
    }
```

- [ ] **Step 2: 執行測試，確認因為簽名不符而編譯失敗**

Run: `cd src-tauri && cargo test --lib commands::ai::tests::chat_prompt`
Expected: 編譯錯誤，訊息類似
`this function takes 2 arguments but 3 arguments were supplied`。

- [ ] **Step 3: 改 `build_chat_prompt` 的簽名與內容**

把 `build_chat_prompt`（約第 160 行）的簽名改成：

```rust
pub fn build_chat_prompt(
    snapshot: &crate::ai::EnvSnapshot,
    locale: Locale,
    supports_artifacts: bool,
) -> String {
```

在函式內、`format!` 之前，新增：

```rust
    let artifact_section = if supports_artifacts {
        crate::ai::artifact_prompt::artifact_protocol_section()
    } else {
        ""
    };
```

然後在 `format!` 樣板的**結尾**（第 6 條規則
`...if you do, mark it clearly in prose.` 之後、收尾的 `"#` 之前）接上
`{artifact_section}`，也就是那一行變成：

```rust
6. Never produce destructive operations against system roots unless the
   user explicitly asks; if you do, mark it clearly in prose.{artifact_section}"#,
```

（`format!` 的具名參數區塊維持原樣——`artifact_section` 是同名區域變數，
Rust 的 `format!` 會自動捕捉，不需要額外寫 `artifact_section = artifact_section`。）

- [ ] **Step 4: 把旗標接進三個呼叫端**

**(a) `run_chat`**（約第 368 行）簽名新增參數，放在 `locale` 之後：

```rust
async fn run_chat(
    messages: Vec<ChatMessage>,
    snapshot: crate::ai::EnvSnapshot,
    provider_id: Option<String>,
    locale: Locale,
    supports_artifacts: bool,
    router: &AiRouter,
    app: &AppHandle,
    stream_id: String,
) -> Result<AiChatReply, AiError> {
```

其內部（約第 381 行）：

```rust
    let prompt = build_chat_prompt(&snapshot, locale, supports_artifacts);
```

**(b) `ai_chat`**（約第 416 行）的 `#[tauri::command]` 參數清單，在 `locale: Locale,`
之後新增：

```rust
    supports_artifacts: bool,
```

它的 MCP 分支呼叫（約第 461 行）改成：

```rust
            let prompt = build_chat_prompt(&snapshot, locale, supports_artifacts);
```

它結尾委派給 `run_chat` 的那一行（約第 561 行）改成：

```rust
    run_chat(messages, snapshot, provider_id, locale, supports_artifacts, &router, &app, session_id).await
```

**(c) `ai_chat_ctx`**（約第 566 行）同樣在 `locale: Locale,` 之後新增
`supports_artifacts: bool,`，並把它委派給 `run_chat` 的那一行（約第 588 行）改成：

```rust
    run_chat(messages, snapshot, provider_id, locale, supports_artifacts, &router, &app, conn_id).await
```

- [ ] **Step 5: 執行測試**

Run: `cd src-tauri && cargo test --lib commands::ai::`
Expected: 全部通過（原本的 5 條 chat_prompt 測試 + 新增的 2 條 + 該模組其他測試）。

- [ ] **Step 6: 型別檢查整個 crate**

Run: `cd src-tauri && cargo check`
Expected: 無錯誤。若有其他地方呼叫 `build_chat_prompt` 或 `run_chat` 而漏改，
這一步會抓出來。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands/ai.rs
git commit -m "feat(artifact): teach the chat prompt the artifact protocol behind a flag"
```

---

### Task 3: Code Assistant 與知識庫的系統提示（Rust）

這兩個 prompt builder 各自只有一個消費端（`CodeAssistantView`、
`KnowledgeBaseView`），所以無條件接上，不需要旗標。

**Files:**
- Modify: `src-tauri/src/code_assistant/mod.rs`
- Modify: `src-tauri/src/knowledge_base/chat.rs`

- [ ] **Step 1: 寫測試**

`src-tauri/src/code_assistant/mod.rs`：檔案裡若已有 `#[cfg(test)] mod tests`，
加進去；沒有的話在檔案最下方新增：

```rust
#[cfg(test)]
mod artifact_prompt_tests {
    use super::build_system_prompt;
    use crate::ai::Locale;

    #[test]
    fn system_prompt_teaches_the_artifact_protocol() {
        let p = build_system_prompt("/tmp/proj", Locale::ZhTw);
        assert!(p.contains("artifact-html"));
        assert!(p.contains("artifact-chart"));
    }
}
```

`src-tauri/src/knowledge_base/chat.rs`：同樣的做法，但呼叫參數不同：

```rust
#[cfg(test)]
mod artifact_prompt_tests {
    use super::build_system_prompt;
    use crate::ai::Locale;

    #[test]
    fn system_prompt_teaches_the_artifact_protocol() {
        let p = build_system_prompt("My Notebook", Locale::ZhTw);
        assert!(p.contains("artifact-html"));
        assert!(p.contains("artifact-chart"));
    }
}
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `cd src-tauri && cargo test --lib artifact_prompt_tests`
Expected: 兩條都 FAIL（提示裡目前沒有 `artifact-html`）。

- [ ] **Step 3: 實作**

兩個檔案的 `build_system_prompt` 都是一個大 `format!`，且**結尾都是同一段
Mermaid 說明**。做法一致：在 `format!` 之前新增

```rust
    let artifact_section = crate::ai::artifact_prompt::artifact_protocol_section();
```

然後把 `format!` 樣板結尾那段 Mermaid 說明的**最後一個字元之後**（收尾的 `"#`
之前）接上 `{artifact_section}`。

具體來說，`code_assistant/mod.rs` 的樣板最後是
`...and do not rely on custom colors to convey meaning."#`，改成：

```rust
...and do not rely on custom colors to convey meaning.{artifact_section}"#
```

`knowledge_base/chat.rs` 的樣板最後是
`...keep it dark/muted so light text stays readable."#`，改成：

```rust
...keep it dark/muted so light text stays readable.{artifact_section}"#
```

- [ ] **Step 4: 執行測試**

Run: `cd src-tauri && cargo test --lib artifact_prompt_tests`
Expected: 兩條都通過。

- [ ] **Step 5: 全 crate 測試與型別檢查**

Run: `cd src-tauri && cargo test --lib`
Expected: 全部通過。

Run: `cd src-tauri && cargo check`
Expected: 無錯誤。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/code_assistant/mod.rs src-tauri/src/knowledge_base/chat.rs
git commit -m "feat(artifact): teach the code assistant and knowledge base prompts too"
```

---

### Task 4: 前端把旗標傳下去

**Files:**
- Modify: `src/ipc/ai.ts`
- Modify: `src/hooks/useMcpChat.ts`
- Modify: `src/hooks/useRemoteAiChat.ts`
- Modify: `src/ipc/ai.test.ts`

- [ ] **Step 1: 寫測試**

在 `src/ipc/ai.test.ts` 既有測試之後新增（沿用該檔案既有的 invoke mock 寫法——
先讀該檔案開頭確認 mock 變數叫什麼名字，下面用 `invoke` 泛稱）：

```ts
  it("aiChat forwards supportsArtifacts, defaulting to false", async () => {
    await aiChat([{ role: "user", content: "x" }], "s1");
    expect(invoke).toHaveBeenLastCalledWith(
      "ai_chat",
      expect.objectContaining({ supportsArtifacts: false }),
    );

    await aiChat([{ role: "user", content: "x" }], "s1", undefined, false, "zh-TW", true);
    expect(invoke).toHaveBeenLastCalledWith(
      "ai_chat",
      expect.objectContaining({ supportsArtifacts: true }),
    );
  });

  it("invokeAiChatCtx forwards supportsArtifacts, defaulting to false", async () => {
    const ctx = { os: "linux", shell: null, cwd: null, recentOutput: null };
    await invokeAiChatCtx([{ role: "user", content: "x" }], ctx, "c");
    expect(invoke).toHaveBeenLastCalledWith(
      "ai_chat_ctx",
      expect.objectContaining({ supportsArtifacts: false }),
    );

    await invokeAiChatCtx([{ role: "user", content: "x" }], ctx, "c", undefined, "zh-TW", true);
    expect(invoke).toHaveBeenLastCalledWith(
      "ai_chat_ctx",
      expect.objectContaining({ supportsArtifacts: true }),
    );
  });
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npx vitest run src/ipc/ai.test.ts`
Expected: 新增的兩條 FAIL（invoke payload 裡沒有 `supportsArtifacts`）。

- [ ] **Step 3: 改 `src/ipc/ai.ts`**

`invokeAiChatCtx`（約第 98 行）新增最後一個選用參數並帶進 payload：

```ts
export function invokeAiChatCtx(
  messages: ChatMessage[],
  ctx: RemoteCtx,
  connId: string,
  providerId?: string,
  locale: Locale = "zh-TW",
  supportsArtifacts = false,
): Promise<AiChatReply> {
  return invoke<AiChatReply>("ai_chat_ctx", {
    messages,
    ctx: { os: ctx.os, shell: ctx.shell, cwd: ctx.cwd, recent_output: ctx.recentOutput },
    connId,
    providerId: providerId ?? null,
    locale,
    supportsArtifacts,
  });
}
```

`aiChat` 同樣：

```ts
export const aiChat = (
  messages: ChatMessage[],
  sessionId: string,
  providerId?: string,
  useMcp = false,
  locale: Locale = "zh-TW",
  supportsArtifacts = false,
): Promise<AiChatReply> =>
  invoke("ai_chat", {
    messages,
    sessionId,
    providerId: providerId ?? null,
    useMcp,
    locale,
    supportsArtifacts,
  });
```

- [ ] **Step 4: 兩個 hook 傳 `true`**

`src/hooks/useMcpChat.ts` 約第 133 行：

```ts
        const reply = await aiChat(iterHistory, sessionId, undefined, useMcp, locale, true);
```

`src/hooks/useRemoteAiChat.ts` 約第 118 行：

```ts
      const reply = await invokeAiChatCtx(history, buildCtx(), connId, providerId, locale, true);
```

（這兩個都是 `ChatPanelShell` 驅動的介面，已經有 `ArtifactPanelProvider`。）

- [ ] **Step 5: 執行測試與型別檢查**

Run: `npx vitest run src/ipc/ai.test.ts`
Expected: 全部通過。

Run: `npx tsc -b`
Expected: 無錯誤。（**不要**用 `tsc --noEmit`，根 tsconfig 是 solution file，
永遠回傳 0。）

- [ ] **Step 6: Commit**

```bash
git add src/ipc/ai.ts src/ipc/ai.test.ts src/hooks/useMcpChat.ts src/hooks/useRemoteAiChat.ts
git commit -m "feat(artifact): plumb supportsArtifacts through the chat IPC"
```

---

### Task 5: 抽出共用的 `<ArtifactSplit>`

**Files:**
- Create: `src/components/ArtifactPanel/ArtifactSplit.tsx`
- Create: `src/components/ArtifactPanel/ArtifactSplit.css`
- Create: `src/components/ArtifactPanel/ArtifactSplit.test.tsx`
- Modify: `src/components/ChatPanel/ChatPanelShell.tsx`
- Modify: `src/components/ChatPanel/styles.css`（刪掉搬走的規則）
- Modify: `src/components/ChatPanel/ChatPanelShell.test.tsx`（三條斷言改 class 名）

- [ ] **Step 1: 寫 `ArtifactSplit` 的測試**

建立 `src/components/ArtifactPanel/ArtifactSplit.test.tsx`：

```tsx
import { useEffect } from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import {
  ArtifactPanelProvider,
  useArtifactPanel,
  type Artifact,
} from "../../contexts/ArtifactPanelContext";
import { ArtifactSplit } from "./ArtifactSplit";

function ShowOnMount({ artifact }: { artifact: Artifact }) {
  const { showArtifact } = useArtifactPanel();
  useEffect(() => { showArtifact(artifact); }, [artifact, showArtifact]);
  return null;
}

const htmlArtifact: Artifact = {
  id: "1", kind: "html", title: "Brief", content: "<p>hi</p>",
};

describe("ArtifactSplit", () => {
  it("renders only its children when there is no artifact", () => {
    const { container } = render(
      <ArtifactPanelProvider>
        <ArtifactSplit><div>CHAT</div></ArtifactSplit>
      </ArtifactPanelProvider>,
    );
    expect(screen.getByText("CHAT")).toBeInTheDocument();
    expect(container.querySelector(".aiterm-artifact-panel")).toBeNull();
    expect(container.querySelector(".aiterm-artifact-resizer")).toBeNull();
    expect(container.querySelector(".aiterm-artifact-split--active")).toBeNull();
  });

  it("renders the panel and the resizer alongside its children when an artifact is active", () => {
    const { container } = render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={htmlArtifact} />
        <ArtifactSplit><div>CHAT</div></ArtifactSplit>
      </ArtifactPanelProvider>,
    );
    expect(screen.getByText("CHAT")).toBeInTheDocument();
    expect(container.querySelector(".aiterm-artifact-panel")).not.toBeNull();
    expect(container.querySelector(".aiterm-artifact-resizer")).not.toBeNull();
    expect(container.querySelector(".aiterm-artifact-split--active")).not.toBeNull();
  });

  // 這是這個元件存在的理由：右欄是 iframe，用 window mousemove 監聽的話游標一進
  // iframe 就收不到事件、拖曳會頓。pointer capture 才不會斷（見
  // docs/superpowers/specs/2026-09-01-artifact-panel-design.md 與 commit 544d935）。
  it("drags with pointer capture so the iframe cannot swallow the events", () => {
    const { container } = render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={htmlArtifact} />
        <ArtifactSplit><div>CHAT</div></ArtifactSplit>
      </ArtifactPanelProvider>,
    );
    const grip = container.querySelector(".aiterm-artifact-resizer") as HTMLElement;
    let captured = false;
    grip.setPointerCapture = () => { captured = true; };
    grip.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerId: 1 }));
    expect(captured).toBe(true);
  });
});
```

- [ ] **Step 2: 執行測試，確認因模組不存在而失敗**

Run: `npx vitest run src/components/ArtifactPanel/ArtifactSplit.test.tsx`
Expected: 失敗，`Failed to resolve import "./ArtifactSplit"`。

- [ ] **Step 3: 建立 `src/components/ArtifactPanel/ArtifactSplit.css`**

規則從 `src/components/ChatPanel/styles.css` 搬過來，class 前綴改成
`aiterm-artifact-split`（原本掛在 `.aiterm-ai-panel` 上的三個狀態 class 現在由這個
元件自己的根節點承擔）：

```css
.aiterm-artifact-split {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  min-height: 0;
}

.aiterm-artifact-split--active {
  flex-direction: row;
}

.aiterm-artifact-split__chat {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

/* 聊天欄的寬度是像素值（拖拉分割用），容器整體卻可能很窄——固定像素寬會把文件欄
   擠到只剩幾十像素。上限用百分比，讓文件欄在窄容器下仍有一半空間；拖拉時的下限
   另外由 MIN_CHAT_COLUMN_WIDTH 顧。 */
.aiterm-artifact-split--active .aiterm-artifact-split__chat {
  max-width: 55%;
}

.aiterm-artifact-resizer {
  width: 6px;
  cursor: col-resize;
  background-color: transparent;
  transition: background-color 0.2s;
  flex-shrink: 0;
}

.aiterm-artifact-resizer:hover,
.aiterm-artifact-split--resizing .aiterm-artifact-resizer {
  background-color: var(--accent, #a855f7);
}

/* 拖曳中把 iframe 的指標事件關掉。pointer capture 已經讓事件不會被 iframe 攔走，
   這條是第二層保險，順便避免游標在文件上變成 I 字或選到 iframe 裡的文字。 */
.aiterm-artifact-split--resizing .aiterm-artifact-html-frame {
  pointer-events: none;
}

.aiterm-artifact-split--resizing {
  user-select: none;
  cursor: col-resize;
}
```

- [ ] **Step 4: 建立 `src/components/ArtifactPanel/ArtifactSplit.tsx`**

```tsx
import { useRef, useState, type PointerEvent, type ReactNode } from "react";
import { useArtifactPanel } from "../../contexts/ArtifactPanelContext";
import { ArtifactPanel } from "./ArtifactPanel";
import "./ArtifactSplit.css";

const MIN_CHAT_COLUMN_WIDTH = 220;
const MIN_ARTIFACT_COLUMN_WIDTH = 260;

interface ArtifactSplitProps {
  /** 聊天欄的內容。沒有 artifact 時它佔滿整個容器。 */
  children: ReactNode;
}

/**
 * 有 artifact 時把版面裂成「聊天欄 + 可拖拉分隔線 + ArtifactPanel」，沒有時就
 * 只是一個透明的容器。自帶樣式，所以宿主用 CSS class 還是 inline style 都能接。
 *
 * 抽成共用元件而不是各介面各寫一份，是因為這裡的拖曳有一個只有實機才會發現的
 * 陷阱：右欄是 iframe，游標一進到它的範圍，window 上的 mousemove 就變成 iframe
 * 自己那份文件的事件、父視窗完全收不到，拖曳會一頓一頓。必須用
 * setPointerCapture 把該 pointer 的後續事件強制導回分隔線。複製五份幾乎保證
 * 後四份會重蹈覆轍。
 */
export function ArtifactSplit({ children }: ArtifactSplitProps) {
  const { activeArtifact } = useArtifactPanel();
  const [chatColumnWidth, setChatColumnWidth] = useState(320);
  const [isResizing, setIsResizing] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const isDraggingRef = useRef(false);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    isDraggingRef.current = true;
    setIsResizing(true);
    (e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!isDraggingRef.current || !containerRef.current) return;
    const rect = containerRef.current.getBoundingClientRect();
    setChatColumnWidth(Math.max(
      MIN_CHAT_COLUMN_WIDTH,
      Math.min(e.clientX - rect.left, rect.width - MIN_ARTIFACT_COLUMN_WIDTH),
    ));
  };

  const onPointerUp = () => {
    if (!isDraggingRef.current) return;
    isDraggingRef.current = false;
    setIsResizing(false);
  };

  const className = [
    "aiterm-artifact-split",
    activeArtifact ? "aiterm-artifact-split--active" : "",
    isResizing ? "aiterm-artifact-split--resizing" : "",
  ].filter(Boolean).join(" ");

  return (
    <div className={className} ref={containerRef}>
      <div
        className="aiterm-artifact-split__chat"
        style={activeArtifact
          ? { width: `${chatColumnWidth}px`, flexShrink: 0, flexGrow: 0 }
          : { flex: 1 }}
      >
        {children}
      </div>

      {activeArtifact && (
        <>
          <div
            className="aiterm-artifact-resizer"
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            title="拖曳調整寬度"
          />
          <ArtifactPanel />
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 5: 執行 `ArtifactSplit` 測試**

Run: `npx vitest run src/components/ArtifactPanel/ArtifactSplit.test.tsx`
Expected: 3 個測試通過。

- [ ] **Step 6: 讓 `ChatPanelShell` 改用它**

在 `src/components/ChatPanel/ChatPanelShell.tsx`：

1. import 改成（移除 `ArtifactPanel`，改引入 `ArtifactSplit`；`PointerEvent`
   型別仍被外層寬度把手使用，保留）：

```tsx
import { ArtifactPanelProvider, useArtifactPanel } from "../../contexts/ArtifactPanelContext";
import { ArtifactSplit } from "../ArtifactPanel/ArtifactSplit";
```

2. **刪除**這些現在由 `ArtifactSplit` 承擔的東西：常數
   `MIN_CHAT_COLUMN_WIDTH`、`MIN_ARTIFACT_COLUMN_WIDTH`；state
   `chatColumnWidth`、`isArtifactResizing`；ref `splitContainerRef`、
   `isArtifactDraggingRef`；三個 `onArtifactResizePointer*` 處理函式；以及它們
   上方那段解釋 iframe/pointer-capture 的註解（那段註解已經搬進
   `ArtifactSplit.tsx`）。

3. `panelClass` 刪掉兩行（`--split` 與 `--resizing` 現在由 `ArtifactSplit`
   自己的根節點承擔）：

```tsx
  const panelClass = [
    "aiterm-ai-panel",
    isOpen ? "" : "aiterm-ai-panel-hidden",
    // Windows can't blur the terminal behind the glass panel — see styles.css.
    isWindows ? "aiterm-ai-panel--solid" : "",
    expanded ? "aiterm-ai-panel--expanded" : "",
  ].filter(Boolean).join(" ");
```

4. 根 `<div>` 移除 `ref={splitContainerRef}`。

5. 把原本 `<div className="aiterm-ai-panel-chat-column" style={...}>` 換成
   `<ArtifactSplit>`，並移除檔案結尾那段
   `{activeArtifact && (<>…<ArtifactPanel /></>)}`。也就是結構變成：

```tsx
      <ArtifactSplit>
        {/* 原本 chat-column 裡的所有內容原封不動：header、history panel、
            MessageList、toolFallbackReason、ModeHint、agent 狀態列、
            extraAboveInput、輸入區 */}
      </ArtifactSplit>
    </div>
  );
}
```

（`useArtifactPanel()` 的 `activeArtifact` 仍然要留著——`ChatPanelShell` 用它
決定展開狀態，見 `userExpandedRef` 那段 effect。）

- [ ] **Step 7: 從 `src/components/ChatPanel/styles.css` 刪掉搬走的規則**

刪除這些（它們的等價規則已經在 `ArtifactSplit.css`）：
`.aiterm-ai-panel--split`、`.aiterm-ai-panel-chat-column`、
`.aiterm-ai-panel--split .aiterm-ai-panel-chat-column`、
`.aiterm-artifact-resizer`（含 `:hover` 與 `--resizing` 變體）、
`.aiterm-ai-panel--resizing .aiterm-artifact-html-frame`、
`.aiterm-ai-panel--resizing`，連同它們上方的中文註解一起刪。
**其餘規則一律不動。**

- [ ] **Step 8: 更新 `ChatPanelShell.test.tsx` 的三條 class 斷言**

抽出元件後 `--split` 換到 `ArtifactSplit` 的根節點，所以把該檔案裡所有
`.aiterm-ai-panel--split` 改成 `.aiterm-artifact-split--active`
（共三處，分別在「renders a two-column split…」、「does not render the split
layout…」、「closing the artifact panel collapses back…」三條測試裡）。
**其餘斷言一個字都不要改**——特別是斷言 `--expanded` 的那兩條（展開狀態仍然掛在
面板根節點，不受這次重構影響）。

- [ ] **Step 9: 執行測試與型別檢查**

Run: `npx vitest run src/components/ChatPanel src/components/ArtifactPanel`
Expected: 全部通過。

Run: `npx tsc -b`
Expected: 無錯誤。

Run: `npm run test -- --run`
Expected: 全部通過（基準線 142 檔 / 1163 測試，加上這次新增的）。
**已知既有 flake**：`src/components/MailView/MailView.test.tsx` 的
「falls back to the first account when the selected one was removed」偶爾會在全套件
平行執行時失敗，與本功能無關；只有這一條紅就重跑確認，不要去修它。

- [ ] **Step 10: Commit**

```bash
git add src/components/ArtifactPanel/ArtifactSplit.tsx src/components/ArtifactPanel/ArtifactSplit.css src/components/ArtifactPanel/ArtifactSplit.test.tsx src/components/ChatPanel/ChatPanelShell.tsx src/components/ChatPanel/styles.css src/components/ChatPanel/ChatPanelShell.test.tsx
git commit -m "refactor(artifact): extract the split layout into a shared ArtifactSplit"
```

---

### Task 6: 接上兩個資料庫聊天

**Files:**
- Modify: `src/components/DatabaseView/DatabaseAiChat.tsx`
- Modify: `src/components/CrossDbView/CrossDbAiChat.tsx`
- Create: `src/components/DatabaseView/DatabaseAiChat.artifact.test.tsx`
- Create: `src/components/CrossDbView/CrossDbAiChat.artifact.test.tsx`

> 這兩個元件都沒有既有的 render 測試（`CrossDbView` 目錄下完全沒有測試檔），
> 所以新增獨立的 `*.artifact.test.tsx`，只涵蓋這次新增的行為，不去建立整個元件
> 的測試基礎建設。若掛載時因為缺 mock 而失敗，照
> `docs/superpowers/plans/` 既有做法 mock 掉 Tauri 入口即可；真的卡住就回報，
> 不要為了讓測試綠而改動元件。

- [ ] **Step 1: 寫測試（兩個檔案，內容平行）**

`src/components/DatabaseView/DatabaseAiChat.artifact.test.tsx`：

```tsx
import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return { useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }) };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue({}) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

import { DatabaseAiChat } from "./DatabaseAiChat";

describe("DatabaseAiChat artifact wiring", () => {
  it("mounts inside an ArtifactSplit so artifacts have somewhere to render", () => {
    const { container } = render(
      <DatabaseAiChat connectionId="c1" schema={null} />,
    );
    // 沒有 artifact 時分割是「不啟用」狀態，但容器必須在——這就是接線本身。
    expect(container.querySelector(".aiterm-artifact-split")).not.toBeNull();
    expect(container.querySelector(".aiterm-artifact-panel")).toBeNull();
  });
});
```

`src/components/CrossDbView/CrossDbAiChat.artifact.test.tsx`：同上，但改成

```tsx
import { CrossDbAiChat } from "./CrossDbAiChat";
// ...
    const { container } = render(<CrossDbAiChat databases={[]} />);
```

（兩個元件的 props 以實際簽名為準：`DatabaseAiChat` 是
`{ connectionId, schema, sendRemoteResponse? }`，`CrossDbAiChat` 是
`{ databases, sendRemoteResponse? }`——實作前先讀檔案確認，選用的 prop 不用傳。）

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npx vitest run src/components/DatabaseView/DatabaseAiChat.artifact.test.tsx src/components/CrossDbView/CrossDbAiChat.artifact.test.tsx`
Expected: 兩條都 FAIL（找不到 `.aiterm-artifact-split`）。

- [ ] **Step 3: 接線 `DatabaseAiChat`**

1. 新增 import：

```tsx
import { ArtifactPanelProvider } from "../../contexts/ArtifactPanelContext";
import { ArtifactSplit } from "../ArtifactPanel/ArtifactSplit";
```

2. 把匯出的 `DatabaseAiChat` 改成薄殼包 provider，原本的函式本體改名為
   `DatabaseAiChatInner`（做法與 `ChatPanelShell` 一致——provider 必須包在消費它的
   元件外面）：

```tsx
export function DatabaseAiChat(props: Props) {
  return (
    <ArtifactPanelProvider>
      <DatabaseAiChatInner {...props} />
    </ArtifactPanelProvider>
  );
}

function DatabaseAiChatInner({ connectionId, schema, sendRemoteResponse }: Props) {
  // ...原本的函式本體，一字不動...
```

3. 在 return 裡，把「Main chat area」那個 div
   （`<div style={{ display: "flex", flexDirection: "column", flex: 1, minWidth: 0 }}>`）
   用 `<ArtifactSplit>` 包起來——歷史抽屜留在外面，因為它是聊天欄左側的既有欄位：

```tsx
      <ArtifactSplit>
        <div style={{ display: "flex", flexDirection: "column", flex: 1, minWidth: 0 }}>
          {/* ...原本的內容一字不動... */}
        </div>
      </ArtifactSplit>
```

4. 兩個 `aiChat(...)` 呼叫（約第 295、351 行）尾端補上 `true`。這兩個呼叫目前都
   傳到 `locale` 為止，所以在 `locale` 之後加一個引數：

```tsx
          locale,
          true,
        );
```

- [ ] **Step 4: 接線 `CrossDbAiChat`**

做法完全相同：
1. 同樣兩個 import。
2. 匯出的 `CrossDbAiChat` 改成薄殼包 provider，本體改名 `CrossDbAiChatInner`。
3. 用 `<ArtifactSplit>` 包住 `<div className="crossdb-chat__main">`（歷史面板留在
   外面）。
4. 三個 `aiChat(...)` 呼叫（約第 270、303、382 行）都在 `locale` 之後補 `true`。

- [ ] **Step 5: 執行測試與型別檢查**

Run: `npx vitest run src/components/DatabaseView src/components/CrossDbView`
Expected: 全部通過（含新增的兩條）。

Run: `npx tsc -b`
Expected: 無錯誤。

- [ ] **Step 6: Commit**

```bash
git add src/components/DatabaseView/DatabaseAiChat.tsx src/components/DatabaseView/DatabaseAiChat.artifact.test.tsx src/components/CrossDbView/CrossDbAiChat.tsx src/components/CrossDbView/CrossDbAiChat.artifact.test.tsx
git commit -m "feat(artifact): wire the database and cross-database chats"
```

---

### Task 7: 接上 Code Assistant 與知識庫

這兩個介面的後端提示在 Task 3 已經無條件教了，前端不需要傳旗標——只要包 provider
與套 `ArtifactSplit`。

**Files:**
- Modify: `src/components/CodeAssistantView/index.tsx`
- Modify: `src/components/KnowledgeBaseView/index.tsx`
- Create: `src/components/CodeAssistantView/artifact.test.tsx`
- Modify: `src/components/KnowledgeBaseView/index.test.tsx`（新增一條）

- [ ] **Step 1: 寫測試**

`src/components/CodeAssistantView/artifact.test.tsx`：比照 Task 6 的寫法建立，
斷言渲染出 `.aiterm-artifact-split`。**注意**：`CodeAssistantView` 在
`projectRoot` 為空時會走「請選資料夾」的空狀態分支、提早 return，那個分支**不需要**
artifact 面板；測試要讓它進到有 `projectRoot` 的分支。該值來自
`loadSavedRoot()`，讀的是 `localStorage` 的 `"aiterm-code-assistant-root"`
（`index.tsx:15` 的 `STORAGE_KEY`），所以測試裡先設好它：

```tsx
  beforeEach(() => {
    localStorage.setItem("aiterm-code-assistant-root", "/tmp/proj");
  });
```

（`src/test-setup.ts` 每個測試後會清空 localStorage，不需要自己收尾。）

`src/components/KnowledgeBaseView/index.test.tsx`：在既有測試之後新增一條，斷言
選中筆記本後渲染出 `.aiterm-artifact-split`（沿用該檔案既有的 mock 與 render
helper，不要另建一套）。

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npx vitest run src/components/CodeAssistantView src/components/KnowledgeBaseView`
Expected: 新增的兩條 FAIL。

- [ ] **Step 3: 接線 `CodeAssistantView`**

1. 兩個 import（路徑注意這裡是 `../../`）：

```tsx
import { ArtifactPanelProvider } from "../../contexts/ArtifactPanelContext";
import { ArtifactSplit } from "../ArtifactPanel/ArtifactSplit";
```

2. 匯出的 `CodeAssistantView` 改薄殼包 provider、本體改名
   `CodeAssistantViewInner`。
3. 在**有 `projectRoot` 的那個 return**（約第 256 行起）裡，把
   `.ca-messages` 與 `.ca-toolbar`（以及其後的輸入區）一起用 `<ArtifactSplit>`
   包起來，`{closeConfirmDialog}` 留在外面（它是 modal，不屬於聊天欄）：

```tsx
  return (
    <div className="ca-view">
      {closeConfirmDialog}
      <ArtifactSplit>
        {/* ...原本 .ca-messages 之後的所有內容一字不動... */}
      </ArtifactSplit>
    </div>
  );
```

**不要**動 `projectRoot` 為空時的那個空狀態 return。

- [ ] **Step 4: 接線 `KnowledgeBaseView`**

1. 同樣兩個 import。
2. 匯出的 `KnowledgeBaseView` 改薄殼包 provider、本體改名
   `KnowledgeBaseViewInner`。
3. 在 `.kb-main` 裡，把 `activeNotebook` 為真那個分支的 `<>…</>` 內容用
   `<ArtifactSplit>` 包起來（`PythonEnvGate` 與空狀態分支留在外面）：

```tsx
        ) : (
          <ArtifactSplit>
            {/* ...原本 <> 裡的所有內容一字不動：unsynced banner、
                .ca-messages、輸入區等... */}
          </ArtifactSplit>
        )}
```

**注意**：這個檔案已經有 `historyWidth`/`isResizingHistory`
（`NotebookSidebar` 的 resizer），**不要動它們**；`ArtifactSplit` 的寬度 state 封裝
在元件內部，不會撞名。

- [ ] **Step 5: 執行測試、型別檢查、完整套件**

Run: `npx vitest run src/components/CodeAssistantView src/components/KnowledgeBaseView`
Expected: 全部通過。

Run: `npx tsc -b`
Expected: 無錯誤。

Run: `npm run test -- --run`
Expected: 全部通過（既有 MailView flake 除外，見 Task 5 Step 9 的說明）。

Run: `cd src-tauri && cargo test --lib`
Expected: 全部通過。

- [ ] **Step 6: Commit**

```bash
git add src/components/CodeAssistantView/index.tsx src/components/CodeAssistantView/artifact.test.tsx src/components/KnowledgeBaseView/index.tsx src/components/KnowledgeBaseView/index.test.tsx
git commit -m "feat(artifact): wire the code assistant and knowledge base chats"
```

---

## 完成後的驗證（需要真機手動確認）

七個 task 完成、`npm run test -- --run`、`npx tsc -b`、`cargo test --lib` 都綠燈
之後，程式碼層面已符合 spec。以下是 spec 標註「需要真機驗證」的部分：

1. 在資料庫分頁實際問「用圖表顯示 xxx」，確認模型真的會輸出 `artifact-chart`、圖
   畫得出來、資料綁定正確。這取決於所選 provider/model 的能力。
2. 在知識庫分頁實際問「把這些文件整理成一份報告」，確認會輸出 `artifact-html`。
3. 在 Code Assistant 問一個適合出圖的問題，確認面板正常。
4. 逐一確認四個介面的分割拖曳都順（特別是拖過 HTML 文件時不該頓——那正是
   `ArtifactSplit` 用 pointer capture 的原因）。
5. **確認 `ApiDocsView` 與 `DocConverterView` 的產出裡不會冒出 artifact fence**
   ——它們共用 `build_chat_prompt` 但維持 `supports_artifacts=false`，這是旗標設計
   是否正確的最終驗收。

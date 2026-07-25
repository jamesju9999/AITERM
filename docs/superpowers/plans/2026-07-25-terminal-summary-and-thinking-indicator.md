# Terminal Command-History Tab Summary + Thinking Indicator Fix — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** (Part 1) Change the title-bar tab summary to derive from the terminal's executed shell-command history, generated exactly once per terminal tab as a stable identifier — replacing the just-merged AI-chat-based summary. (Part 2) Fix the `StreamingIndicator` overlapping the input box by having it replace the input pill's position (mutually exclusive render) while the AI is thinking.

**Architecture:** The summarization helper (`summarizeTab.ts`) is rewritten to take `TerminalBlock[]` instead of `McpChatMessage[]`. The trigger moves out of `AiPanel` (where it watched chat messages) and into `TerminalView` (which already holds `blocks` from `useTerminalBlocks` and already receives the `onSummaryUpdate` prop threaded up to `TerminalApp`). All the AI-chat-summary wiring added to `AiPanel` is removed and its test file reverted. `TerminalApp`'s summary storage + title composition is unchanged. Part 2 is a self-contained render + CSS change in `TerminalView` / `StreamingIndicator`.

**Tech Stack:** React 19 + TypeScript, Vitest, no backend changes.

**Spec:** `docs/superpowers/specs/2026-07-25-terminal-summary-and-thinking-indicator-design.md`

---

## Task 1: Rewrite `summarizeTab.ts` to summarize terminal commands + rewrite its tests

**Files:**
- Modify: `src/lib/summarizeTab.ts`
- Modify: `src/lib/summarizeTab.test.ts`

- [ ] **Step 1: Rewrite the test file (TDD — new behavior first)**

Replace the ENTIRE contents of `src/lib/summarizeTab.test.ts` with:

```ts
import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { summarizeCommands } from "./summarizeTab";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";

beforeEach(() => {
  invokeMock.mockReset();
});

function block(command: string, overrides: Partial<TerminalBlock> = {}): TerminalBlock {
  return {
    id: Math.random().toString(36).slice(2),
    command,
    status: "completed",
    startTime: Date.now(),
    rawOutput: "",
    ...overrides,
  };
}

describe("summarizeCommands", () => {
  it("returns null and makes no AI call when there are no commands", async () => {
    const result = await summarizeCommands([], "sess-1", "zh-TW");
    expect(result).toBeNull();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("returns null and makes no AI call when all commands are blank/whitespace", async () => {
    const result = await summarizeCommands([block("   "), block("")], "sess-1", "zh-TW");
    expect(result).toBeNull();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("calls ai_chat with a summary-suffixed session id and returns the trimmed reply", async () => {
    invokeMock.mockResolvedValue({ content: "  查詢本機 IP  ", tool_calls: [], tool_calling_unsupported: false });

    const result = await summarizeCommands(
      [block("ifconfig en0", { cwd: "/Users/jamesju/Downloads" })],
      "sess-1",
      "zh-TW",
    );

    expect(result).toBe("查詢本機 IP");
    expect(invokeMock).toHaveBeenCalledWith(
      "ai_chat",
      expect.objectContaining({ sessionId: "sess-1-summary" }),
    );
  });

  it("returns null when invokeAiChat rejects", async () => {
    invokeMock.mockRejectedValue(new Error("network error"));
    const result = await summarizeCommands([block("ls")], "sess-1", "zh-TW");
    expect(result).toBeNull();
  });

  it("returns null when the reply content is empty/whitespace", async () => {
    invokeMock.mockResolvedValue({ content: "   ", tool_calls: [], tool_calling_unsupported: false });
    const result = await summarizeCommands([block("ls")], "sess-1", "zh-TW");
    expect(result).toBeNull();
  });

  it("only includes the last 10 commands in the prompt sent to invokeAiChat", async () => {
    invokeMock.mockResolvedValue({ content: "summary", tool_calls: [], tool_calling_unsupported: false });

    const many: TerminalBlock[] = [];
    for (let i = 0; i < 14; i++) many.push(block(`command-${i}`));
    await summarizeCommands(many, "sess-1", "zh-TW");

    const promptText = (invokeMock.mock.calls[0][1] as { messages: { content: string }[] }).messages[0].content;
    expect(promptText).not.toContain("command-0");
    expect(promptText).not.toContain("command-3");
    expect(promptText).toContain("command-4");
    expect(promptText).toContain("command-13");
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx vitest run src/lib/summarizeTab.test.ts`
Expected: fails — `summarizeCommands` is not exported (the module still exports `summarizeConversation`).

- [ ] **Step 3: Rewrite `summarizeTab.ts`**

Replace the ENTIRE contents of `src/lib/summarizeTab.ts` with:

```ts
import { invokeAiChat } from "../ipc/ai";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";
import type { Locale } from "./i18n";

const MAX_CONTEXT_COMMANDS = 10;

function buildSummaryPrompt(commands: string[], cwd: string | undefined, locale: Locale): string {
  const list = commands.join("\n");
  const cwdLine = cwd ? (locale === "zh-TW" ? `工作目錄：${cwd}\n` : `Working directory: ${cwd}\n`) : "";

  return locale === "zh-TW"
    ? `以下是使用者在一個終端機工作階段執行的指令。請用不超過 20 個字生成一句精簡的中文摘要，描述這個終端機在做什麼（作為分頁標題）。不要標點符號結尾、不要加引號、只輸出摘要本身。\n\n${cwdLine}${list}`
    : `Below are the shell commands a user ran in one terminal session. Write a concise summary (40 characters or fewer) in English describing what this terminal is for (used as a tab title). No trailing punctuation, no quotes, output only the summary itself.\n\n${cwdLine}${list}`;
}

/**
 * One-shot AI call that summarizes a terminal tab's recent executed shell
 * commands into a short title-bar-friendly identifier. Returns null on any
 * failure (no commands, network error, provider not configured, empty reply)
 * — callers should treat null as "leave the tab title as it was", never
 * surface an error.
 */
export async function summarizeCommands(
  blocks: TerminalBlock[],
  sessionId: string,
  locale: Locale,
): Promise<string | null> {
  const commands = blocks
    .map((b) => b.command.trim())
    .filter((c) => c.length > 0)
    .slice(-MAX_CONTEXT_COMMANDS);
  if (commands.length === 0) return null;

  const cwd = blocks[blocks.length - 1]?.cwd;

  try {
    const prompt = buildSummaryPrompt(commands, cwd, locale);
    const reply = await invokeAiChat(
      [{ role: "user", content: prompt }],
      `${sessionId}-summary`,
      undefined,
      false,
      locale,
    );
    const summary = reply.content?.trim();
    return summary || null;
  } catch {
    return null;
  }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npx vitest run src/lib/summarizeTab.test.ts`
Expected: 6 passed.

- [ ] **Step 5: Type-check**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: this file is fine, BUT expect errors in `src/components/AiPanel/index.tsx` — it still imports `summarizeConversation`, which no longer exists. This is expected at this step; Task 2 removes that import. If the ONLY errors are in `AiPanel/index.tsx` about `summarizeConversation`, proceed. If there are errors anywhere else, stop and investigate.

- [ ] **Step 6: Commit**

```bash
git add src/lib/summarizeTab.ts src/lib/summarizeTab.test.ts
git commit -m "feat(titlebar): summarize terminal command history instead of AI chat"
```

## Context

This is Task 1 of a 5-task plan. `TerminalBlock` is exported from `src/hooks/useTerminalBlocks.ts` and has fields `command: string`, `cwd?: string`, `status`, `rawOutput`, etc. `invokeAiChat` (from `src/ipc/ai.ts`) is called exactly as the old code called it — `invokeAiChat(messages, sessionId, undefined, false, locale)` — passing `undefined` for `providerId` so the backend uses the tab's default provider. Note the test file's `block()` helper builds a minimal valid `TerminalBlock`; if `TerminalBlock` has additional *required* fields beyond what the helper sets (`id`, `command`, `status`, `startTime`, `rawOutput`), TypeScript will flag it — add those required fields to the helper's defaults. Per the spec, the summary uses command text + cwd only, never command output (`rawOutput`), to avoid noise and oversized prompts.

The crate intentionally leaves `AiPanel/index.tsx` broken (missing `summarizeConversation`) at the end of this task — it's fixed in Task 2 of the same plan/branch. Committing Task 1 alone leaves the tree not-type-clean; that's expected within this plan.

## Before You Begin

If `TerminalBlock`'s shape differs from what's described, read `src/hooks/useTerminalBlocks.ts` and adapt the test helper's required fields — but keep the function's behavior (command text + cwd, last 10, null on empty/error) exactly as specified.

## Your Job

Follow TDD: rewrite tests, watch them fail, rewrite the helper, watch them pass. Commit.

Work from: [worktree path — see the controller's dispatch message]

## Code Organization

Only touch the two `summarizeTab` files.

## When You're in Over Your Head

If `TerminalBlock` or `invokeAiChat` don't exist at the stated paths or have incompatible shapes, STOP and report BLOCKED/NEEDS_CONTEXT.

## Before Reporting Back: Self-Review

Check: all 6 tests pass? Does the empty/blank-commands guard short-circuit before calling `invokeAiChat`? Does the last-10 slice work for 14 commands? Is `rawOutput` genuinely NOT included in the prompt?

## Report Format

- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Test results
- Files changed
- Self-review findings
- Concerns

---

## Task 2: Remove the AI-chat summary wiring from `AiPanel` + revert its test

**Files:**
- Modify: `src/components/AiPanel/index.tsx`
- Modify: `src/components/AiPanel/AiPanel.test.tsx`

- [ ] **Step 1: Remove the `summarizeConversation` import**

In `src/components/AiPanel/index.tsx`, find and DELETE this line (currently line 8):

```tsx
import { summarizeConversation } from "../../lib/summarizeTab";
```

- [ ] **Step 2: Remove the `onSummaryUpdate` prop from the interface**

Find (currently lines 59-61):

```tsx
  sendRemoteResponse?: (text: string) => void;
  /** Called with a freshly generated one-line summary of this tab's /ai
   *  conversation, after each response settles. See summarizeTab.ts. */
  onSummaryUpdate?: (summary: string) => void;
}
```

Replace with:

```tsx
  sendRemoteResponse?: (text: string) => void;
}
```

- [ ] **Step 3: Remove `onSummaryUpdate` from the destructured params**

Find (currently lines 69-78):

```tsx
export function AiPanel({
  sessionId,
  isOpen,
  providerName,
  onClose,
  onExecuteCommand,
  onOpenProviderPalette,
  sendRemoteResponse,
  onSummaryUpdate,
}: AiPanelProps) {
```

Replace with:

```tsx
export function AiPanel({
  sessionId,
  isOpen,
  providerName,
  onClose,
  onExecuteCommand,
  onOpenProviderPalette,
  sendRemoteResponse,
}: AiPanelProps) {
```

- [ ] **Step 4: Remove the summarization `useEffect` + ref**

Find (currently lines 193-212, the block between the `useMcp` localStorage effect and the `/** Build system prompt ... */` comment):

```tsx
  // Regenerate the title-bar summary once an AI turn fully settles (not
  // mid-stream, not mid-agent-loop) and the message count actually grew —
  // the ref guard stops this from re-firing on unrelated re-renders.
  const lastSummarizedCountRef = useRef(0);
  useEffect(() => {
    if (chat.isStreaming || agentRunning) return;
    if (chat.messages.length === 0) return;
    if (chat.messages.length === lastSummarizedCountRef.current) return;
    const requestCount = chat.messages.length;
    lastSummarizedCountRef.current = requestCount;
    summarizeConversation(chat.messages, sessionId, locale)
      .then((summary) => {
        // A newer request may have started while this one was in flight;
        // only apply the result if we're still the latest.
        if (summary && lastSummarizedCountRef.current === requestCount) {
          onSummaryUpdate?.(summary);
        }
      })
      .catch(() => {});
  }, [chat.messages, chat.isStreaming, agentRunning, sessionId, locale, onSummaryUpdate]);

```

DELETE this whole block (including the trailing blank line), so the `useMcp` effect is immediately followed by the `/** Build system prompt with live CWD + dir listing. */` comment.

- [ ] **Step 5: Remove the ref reset from the "New Chat" handler**

Find (currently line 463):

```tsx
            onClick={() => { chat.clear(); lastSummarizedCountRef.current = 0; /* reset so a new conversation isn't skipped if it settles at the same message count as the cleared one */ setHistoryOpen(false); }}
```

Replace with:

```tsx
            onClick={() => { chat.clear(); setHistoryOpen(false); }}
```

- [ ] **Step 6: Revert `AiPanel.test.tsx` to its pre-summary state**

The ONLY changes to `src/components/AiPanel/AiPanel.test.tsx` since commit `8f33675` were the summary-feature additions (a scriptable `-summary` mock branch, a `realChatCalls()` helper, a `summaryResponseContent` var, and one new test). Revert the entire file to that commit's version:

Run: `git checkout 8f33675 -- src/components/AiPanel/AiPanel.test.tsx`

(This is a clean full-file revert — that file received no other, unrelated changes after `8f33675`, so restoring it wholesale is correct and simplest. Do NOT hand-edit; use the git checkout.)

- [ ] **Step 7: Type-check**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors now (Task 1's `summarizeCommands` exists; `AiPanel` no longer references the removed `summarizeConversation`).

- [ ] **Step 8: Run the AiPanel tests**

Run: `npx vitest run src/components/AiPanel/AiPanel.test.tsx`
Expected: 8 passed (the reverted file's original 8 tests — no `-summary`-aware test anymore).

- [ ] **Step 9: Lint**

Run: `npx eslint src/components/AiPanel/index.tsx`
Expected: 0 errors, 0 warnings (this file's baseline was 0/0 before the summary feature; removing the feature returns it to 0/0). If `useRef` is now unused elsewhere, note that — but `useRef` is used by many other refs in this file (`agentAbortRef`, `textareaRef`, etc.), so its import stays.

- [ ] **Step 10: Commit**

```bash
git add src/components/AiPanel/index.tsx src/components/AiPanel/AiPanel.test.tsx
git commit -m "refactor(titlebar): remove AI-chat summary wiring from AiPanel"
```

## Context

This is Task 2 of a 5-task plan. Task 1 (already committed) rewrote `summarizeTab.ts` to export `summarizeCommands` (terminal-command-based) and removed `summarizeConversation`, which left `AiPanel/index.tsx` referencing a now-missing export. This task removes all the AI-chat-summary wiring from `AiPanel` (the import, the prop, the effect, the ref, the ref-reset) and reverts the `AiPanel.test.tsx` changes that the previous feature added. After this task, the summary feature no longer lives in `AiPanel` at all — Task 3 re-homes the trigger into `TerminalView`. Note `TerminalView` still passes `onSummaryUpdate` to `<AiPanel>` at this point (that passthrough is removed in Task 3); since `onSummaryUpdate` is no longer a declared prop on `AiPanel`, TypeScript may flag that passthrough as an unknown-prop error at Step 7 — if so, that specific error is expected and gets fixed in Task 3. Re-read: actually, extra unknown props on a JSX element ARE a TS error. To keep Task 2 type-clean on its own, ALSO remove the `onSummaryUpdate={onSummaryUpdate}` line from the `<AiPanel>` render in `TerminalView.tsx` as part of this task — see the note below.

**IMPORTANT addendum to keep the tree type-clean:** After Step 6, before Step 7, also edit `src/components/TerminalView.tsx`: find the `<AiPanel>` render and DELETE the line `onSummaryUpdate={onSummaryUpdate}` from its props (it's near the bottom of the file, in the `{sessionId && (<AiPanel ... />)}` block). Leave the `onSummaryUpdate` *prop on TerminalViewProps and its destructuring* intact — Task 3 will start using it directly. Add `src/components/TerminalView.tsx` to the Step 10 commit. (Without this, passing an undeclared `onSummaryUpdate` prop to `AiPanel` is a TS error.)

## Before You Begin

If the exact lines differ from what's quoted (line numbers may drift), search by content. If unsure, report BLOCKED/NEEDS_CONTEXT.

## Your Job

Apply the removals, the TerminalView passthrough deletion (addendum), and the test revert; verify type-clean + 8 tests pass + lint 0/0; commit.

Work from: [worktree path — see the controller's dispatch message]

## Before Reporting Back: Self-Review

Check: is `summarizeConversation` gone from the whole file? Is `onSummaryUpdate` gone from `AiPanel`'s interface, params, and the `TerminalView` `<AiPanel>` passthrough — but STILL present on `TerminalViewProps` + its destructuring? Did the test file get reverted cleanly (8 tests, no `realChatCalls`/`summaryResponseContent`)?

## Report Format

- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Test/lint results (exact numbers)
- Files changed
- Self-review findings
- Concerns

---

## Task 3: Add the command-summary trigger to `TerminalView`

**Files:**
- Modify: `src/components/TerminalView.tsx`

- [ ] **Step 1: Add the import**

Near the other imports at the top of `src/components/TerminalView.tsx`, add:

```tsx
import { summarizeCommands } from "../lib/summarizeTab";
```

(Place it wherever fits the existing import grouping — e.g. alongside other `../lib/...` imports. If there are no other `../lib` imports, put it near the hook/component imports.)

- [ ] **Step 2: Add the trigger `useEffect`**

`TerminalView` already destructures `blocks` from `useTerminalBlocks(...)` (search for `const { blocks,` — it's around line 230) and already receives `onSummaryUpdate` as a prop (from Task 2, still declared on `TerminalViewProps` and destructured in the function signature) and `locale` from `useLocale()`, and has `sessionId` in scope.

Add this effect. Place it near the other top-level `useEffect`s in the component body (a natural spot is right after the block-list / blocks-related effects, but anywhere in the component body among the other effects is fine — it just must be inside the `TerminalView` function, at hook-level, not nested):

```tsx
  // Generate a one-time identifying tab title from the first executed
  // command(s). Debounced so rapid successive commands are captured together;
  // the ref guard makes it fire at most once per terminal session (the view
  // stays mounted across tab switches). Summarizes command text only — see
  // summarizeCommands. Silent on failure; a failed first attempt just leaves
  // the tab showing its plain name until the app restarts.
  const summaryGeneratedRef = useRef(false);
  useEffect(() => {
    if (summaryGeneratedRef.current) return;
    const hasFinalized = blocks.some((b) => b.status === "completed" || b.status === "failed");
    if (!hasFinalized) return;
    const timer = setTimeout(() => {
      summaryGeneratedRef.current = true;
      summarizeCommands(blocks, sessionId, locale)
        .then((summary) => {
          if (summary) onSummaryUpdate?.(summary);
        })
        .catch(() => {});
    }, 1500);
    return () => clearTimeout(timer);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [blocks, sessionId, locale, onSummaryUpdate]);
```

Notes for placement:
- `useRef` and `useEffect` are already imported in this file — don't re-add.
- If `sessionId` is possibly empty/undefined at first render, that's fine: `summarizeCommands` would build a `-summary` session id and the call is harmless; but the `hasFinalized` guard means it only runs once a real command has completed, by which point `sessionId` is set. No extra guard needed.

- [ ] **Step 3: Type-check**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors.

- [ ] **Step 4: Lint**

Run: `npx eslint src/components/TerminalView.tsx`
Expected: no NEW findings versus this file's pre-existing baseline. Before editing, run the lint once and note the count; after, confirm it didn't increase (the new effect uses the same `eslint-disable-next-line react-hooks/exhaustive-deps` pattern as this file's other debounced/blocks effects, so it should add none). If you can't easily get a before-count, use `git show HEAD:src/components/TerminalView.tsx > /tmp/tv-before.tsx` then compare `npx eslint` on a temp copy — do NOT use `git stash`.

- [ ] **Step 5: Commit**

```bash
git add src/components/TerminalView.tsx
git commit -m "feat(titlebar): trigger one-time tab summary from terminal command history"
```

## Context

This is Task 3 of a 5-task plan. Task 2 removed the summary trigger from `AiPanel` and the `onSummaryUpdate` passthrough to `<AiPanel>`, but kept `onSummaryUpdate` as a `TerminalViewProps` prop + destructured var. This task uses that prop directly: a debounced, once-per-session effect that watches `blocks` and calls `summarizeCommands` when the first command finalizes. `blocks` is `TerminalBlock[]` from `useTerminalBlocks`, already in scope. The whole data flow after this task: first command finalizes → (1.5s debounce) → `summarizeCommands(blocks, ...)` → `onSummaryUpdate(summary)` → `TerminalApp` stores it on `tab.aiSummary` → title bar shows "終端機 - summary". `TerminalApp` and `Tab.aiSummary` are unchanged from the previous feature.

**Why the ref guard + debounce:** `blocks` is a new array reference on many renders. Without `summaryGeneratedRef`, the effect could fire repeatedly; the guard ensures at most one AI call per terminal tab (the stated design goal — a stable identifier, not a live-updating summary). The debounce (`setTimeout` + `clearTimeout` cleanup) means several quickly-typed commands are captured together in the single generation.

## Before You Begin

If `blocks` isn't destructured from `useTerminalBlocks` in this file, or `onSummaryUpdate`/`locale`/`sessionId` aren't in scope, STOP and report BLOCKED/NEEDS_CONTEXT.

## Your Job

Add the import + effect, verify type-clean and no new lint, commit.

Work from: [worktree path — see the controller's dispatch message]

## Before Reporting Back: Self-Review

Check: does the effect's dep array include `blocks`, `sessionId`, `locale`, `onSummaryUpdate`? Does `summaryGeneratedRef` correctly make it fire at most once? Does the `hasFinalized` guard prevent firing before any command completes? Is the debounce cleanup (`return () => clearTimeout(timer)`) present so rapid commands reset the timer?

## Report Format

- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Test/lint results
- Files changed
- Self-review findings
- Concerns

---

## Task 4: Fix the thinking indicator overlapping the input box (Part 2)

**Files:**
- Modify: `src/components/TerminalView.tsx`
- Modify: `src/components/StreamingIndicator.css`

- [ ] **Step 1: Make the input / indicator render mutually exclusive**

In `src/components/TerminalView.tsx`, find (near the bottom, around lines 1295-1349):

```tsx
      {!isAlternateBuffer && (
        <WarpInput
          sessionId={sessionId}
          onSubmit={(cmd) => {
```

...(the full `<WarpInput ... />` element, which ends with `shortcut={submitShortcut}` and `/>`)...

```tsx
          shortcut={submitShortcut}
        />
      )}
      {preview.loading && (
        <StreamingIndicator visible text={streamText} />
      )}
```

The current structure is: `{!isAlternateBuffer && (<WarpInput .../>)}` followed by a separate `{preview.loading && (<StreamingIndicator .../>)}`. Change it so that when `preview.loading` is true, the `StreamingIndicator` renders IN PLACE OF `WarpInput` (both still gated on `!isAlternateBuffer`):

Replace the two blocks above with:

```tsx
      {!isAlternateBuffer && (
        preview.loading ? (
          <StreamingIndicator visible text={streamText} />
        ) : (
          <WarpInput
            sessionId={sessionId}
            onSubmit={(cmd) => {
```

...(keep the ENTIRE existing `onSubmit` handler body and all other `<WarpInput>` props exactly as they are — do not modify any of the WarpInput logic)...

```tsx
            shortcut={submitShortcut}
          />
        )
      )}
```

In other words: wrap the existing `<WarpInput .../>` (unchanged internally) as the `else` branch of `preview.loading ? <StreamingIndicator .../> : <WarpInput .../>`, all inside the existing `{!isAlternateBuffer && (...)}` gate, and DELETE the now-redundant standalone `{preview.loading && (<StreamingIndicator visible text={streamText} />)}` block that followed it.

Be careful with JSX nesting/indentation — the `<WarpInput>` element and its long `onSubmit` arrow function must remain syntactically intact, just relocated into the ternary's else branch. After editing, the file must compile.

- [ ] **Step 2: Verify it compiles**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors.

- [ ] **Step 3: Restyle `StreamingIndicator.css` as an in-flow input-pill replacement**

In `src/components/StreamingIndicator.css`, find the `.aiterm-streaming` rule:

```css
.aiterm-streaming {
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  background: rgba(12, 12, 12, 0.95);
  border-top: 1px solid #333;
  padding: 6px 10px;
  font-family: "Cascadia Mono", Consolas, monospace;
  font-size: 12px;
  z-index: 10;
}
```

Replace it with (matching WarpInput's pill frame — same margin, border, radius, background):

```css
.aiterm-streaming {
  margin: 10px 14px;
  padding: 8px 14px;
  background-color: #161616;
  border: 1.5px solid #2f9e7f;
  border-radius: 24px;
  font-family: "Cascadia Mono", Consolas, monospace;
  font-size: 12px;
}
```

Leave the `.aiterm-streaming__label`, `.aiterm-streaming__text`, `.aiterm-streaming__text::-webkit-scrollbar`, `.aiterm-streaming__cursor`, and `@keyframes aiterm-blink` rules UNCHANGED — only the container `.aiterm-streaming` rule changes (drops the absolute positioning / z-index / top-border / translucent background; gains the input-pill frame).

- [ ] **Step 4: Type-check again (CSS change can't break TS, but confirm nothing else drifted)**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors.

- [ ] **Step 5: Lint**

Run: `npx eslint src/components/TerminalView.tsx`
Expected: no new findings vs. baseline (the ternary is a pure structural change; adds no new lint issues).

- [ ] **Step 6: Commit**

```bash
git add src/components/TerminalView.tsx src/components/StreamingIndicator.css
git commit -m "fix(terminal): show thinking indicator in place of the input box instead of overlapping it"
```

## Context

This is Task 4 of a 5-task plan — the self-contained Part 2 fix, independent of the Part 1 summary rework (Tasks 1-3). Root cause (from investigation): `.aiterm-streaming` was `position:absolute; bottom:0; z-index:10`, so it painted on top of `WarpInput` (a normal-flow flex child pinned at the bottom). The fix makes them mutually exclusive in render (thinking → show indicator instead of input) and restyles the indicator to look like the input pill (same rounded green-bordered frame), so it reads as "the input box is thinking." The `StreamingIndicator.tsx` component logic (JSON-explanation extraction, `visible` gate, streamed-text rendering) is NOT touched — only where/how it's positioned.

## Before You Begin

If the `<WarpInput>` / `<StreamingIndicator>` render blocks differ significantly from what's quoted, read the surrounding JSX carefully and adapt while preserving the exact intent (mutually-exclusive render inside the `!isAlternateBuffer` gate). If the JSX is complex enough that you're unsure you can relocate `<WarpInput>` without breaking it, report BLOCKED/NEEDS_CONTEXT rather than committing broken JSX.

## Your Job

Apply the render restructure + CSS restyle, verify type-clean and no new lint, commit.

Work from: [worktree path — see the controller's dispatch message]

## Before Reporting Back: Self-Review

Check: is the standalone `{preview.loading && <StreamingIndicator/>}` block gone (folded into the ternary)? Is `<WarpInput>`'s internal logic (the whole `onSubmit` handler) byte-identical, just relocated? Did the CSS lose `position:absolute`/`bottom`/`z-index` and gain the pill frame? Are the `__label`/`__text`/`__cursor` styles untouched?

## Report Format

- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Test/lint results
- Files changed
- Self-review findings
- Concerns

---

## Task 5: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Full type check**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors.

- [ ] **Step 2: Full lint**

Run: `npm run lint`
Expected: no new errors/warnings vs. the repo's known pre-existing baseline (this repo has pre-existing lint findings unrelated to this work — the bar is "no new ones," not "zero total"; compare the total count against a `git show`/temp-copy baseline if unsure, NOT `git stash`).

- [ ] **Step 3: Full frontend test suite**

Run: `npm run test -- --run`
Expected: all pass. Note the total count: it should equal the pre-feature baseline (279) plus Task 1's 6 `summarizeCommands` tests, MINUS the tests removed by reverting `AiPanel.test.tsx` to `8f33675`. Concretely: the previous (now-reverted) feature had left the suite at 285 (279 + 5 old `summarizeConversation` tests + 1 new AiPanel test − 0... actually the previous feature added `summarizeTab.test.ts` with 5 tests and 1 AiPanel test = 285). After this plan: `summarizeTab.test.ts` now has 6 tests (was 5), and the extra AiPanel test is gone. Net expected total: 285 − 5 (old summarizeTab) + 6 (new summarizeTab) − 1 (removed AiPanel test) = 285. So expect **285** again — but do not treat the exact number as a hard gate; the real check is: 0 failures, `summarizeTab.test.ts` shows 6 passing, and `AiPanel.test.tsx` shows 8 passing. Verify those two specifically.

- [ ] **Step 4: Manual verification checklist**

Run `npm run tauri:dev` (or otherwise launch the app) and confirm:
1. Open a new terminal tab — title bar shows just "終端機" (no summary yet).
2. Run one shell command (e.g. `ls`), wait for it to finish — ~1.5s later, the title bar updates to "終端機 - <summary>" where the summary reflects the command(s) run.
3. Run several more commands — the title bar summary does NOT change (generated once only).
4. Open a second terminal tab, run a different command there — it gets its own independent summary; switching between the two tabs shows each one's own title.
5. Trigger the AI (type `/ai <something>`): during "thinking" (生成指令中…), the input box position shows the rounded-pill thinking indicator INSTEAD of the input box — no overlap, no text bleeding below the box. When thinking ends, the input box returns.
6. Restart the app — summaries are gone (not persisted); tabs show plain names until the next command runs.

Report explicitly which items were verified against a live running app vs. only by reading code, if a live GUI session isn't available in your environment.

- [ ] **Step 5: Report unresolved concerns, if any**

If Step 4 surfaces any issue, report it for a follow-up fix rather than improvising outside this plan's scope.

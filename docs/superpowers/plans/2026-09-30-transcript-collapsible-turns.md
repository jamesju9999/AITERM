# Transcript collapsible turns Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans. Steps use checkbox syntax.

**Goal:** In the task-board transcript dialog, fold the execution record into one collapsible entry per user prompt (`❯ text`).

**Architecture:** Pure parser `parseTranscriptTurns` in `transcriptUtils.ts`; `TranscriptDialog` renders turns (collapsed by default) with expand/collapse-all and raw toggle; falls back to raw `<pre>` when no prompts parsed. Spec: `docs/superpowers/specs/2026-09-30-transcript-collapsible-turns-design.md`.

**Tech Stack:** React 19, Vitest + RTL, i18n in `src/lib/i18n.ts` (zh-TW + en both written by hand).

---

### Task 1: Parser (TDD)
Files: modify `src/components/TaskBoard/transcriptUtils.ts`, `transcriptUtils.test.ts`.
- [ ] Add failing tests: multi-prompt split; bare `❯` -> no turns; `❯ text` between two `─` rule lines is not a prompt; text before first prompt -> preamble; no prompts -> empty turns.
- [ ] Run `npx vitest run src/components/TaskBoard/transcriptUtils.test.ts` — expect FAIL (import missing).
- [ ] Implement `parseTranscriptTurns` (line scan; PROMPT `^\s*❯ (\S.*)$`; RULE `^\s*─{10,}\s*$`; input-box check on neighbours; trim blank edges of each output).
- [ ] Re-run — expect PASS. Commit.

### Task 2: i18n
Files: modify `src/lib/i18n.ts` (zh-TW near line 91, en near line 1739).
- [ ] Add `board_transcript_expand_all`, `_collapse_all`, `_show_raw`, `_show_turns`, `_preamble` in both locales.
- [ ] `npx tsc -b`. Commit with Task 3.

### Task 3: Dialog (TDD)
Files: modify `TranscriptDialog.tsx`, `index.css`; create `TranscriptDialog.test.tsx` (mock `../../ipc/tasks` readTranscript, wrap in `LocaleProvider`).
- [ ] Failing tests: default collapsed (output text absent), click header shows output; expand-all/collapse-all; raw toggle shows `task-transcript-raw` with original text; no prompts -> raw shown, no toggle controls.
- [ ] Run — expect FAIL.
- [ ] Implement dialog + CSS (`.task-transcript-turn*`).
- [ ] Run tests, `npx tsc -b`, `npm run lint`, full `npm run test`. Commit.

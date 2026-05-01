# AITerm – Agent Instructions

AITerm is a **Tauri 2** desktop terminal app: a **React 19** frontend communicates with a **Rust** backend via Tauri IPC (invoke + events). Full architecture is documented in [CLAUDE.md](../CLAUDE.md).

## Commands

```bash
npm run tauri:dev          # Full-stack dev (Rust + frontend); first build is slow
npm run build              # Frontend only (tsc + vite)
npm run lint               # ESLint
npm run test               # Frontend tests (Vitest)
cd src-tauri && cargo test # Rust tests (wiremock for HTTP mocking)
npx tsc --noEmit           # Type check only
```

> Always run `npm run test` + `npx tsc --noEmit` before declaring a feature done.

## Architecture Quick Reference

| Layer | Path | Purpose |
|-------|------|---------|
| Frontend shell | [src/components/TerminalApp.tsx](../src/components/TerminalApp.tsx) | Multi-tab orchestrator, keyboard shortcuts |
| Terminal tab | [src/components/TerminalView.tsx](../src/components/TerminalView.tsx) | xterm.js + PTY wiring + `/ai` trigger |
| IPC wrappers | [src/ipc/](../src/ipc/) | Typed `invoke` + event listeners |
| Hooks | [src/hooks/](../src/hooks/) | `useAiChat`, `useAgentMission`, etc. |
| Tauri commands | [src-tauri/src/commands/](../src-tauri/src/commands/) | All `#[tauri::command]` entry points |
| PTY | [src-tauri/src/pty/](../src-tauri/src/pty/) | `PtyManager` + session lifecycle |
| AI router | [src-tauri/src/ai/](../src-tauri/src/ai/) | Multi-provider routing (OpenAI / Anthropic / Ollama / compat) |
| DB adapters | [src-tauri/src/db/](../src-tauri/src/db/) | SQLx (PG/MySQL/SQLite), tiberius (MSSQL), ODBC (DB2, Windows-only) |
| Config | [src-tauri/src/config/](../src-tauri/src/config/) | JSON config + OS keyring for API keys |
| Guard | [src-tauri/src/guard/](../src-tauri/src/guard/) | Command risk classification: safe / needs_confirm / dangerous / blocked |
| i18n | [src/lib/i18n.ts](../src/lib/i18n.ts) + [src/contexts/LocaleContext.tsx](../src/contexts/LocaleContext.tsx) | `zh-TW` / `en`, via `useLocale()` hook |

## Key Conventions

### IPC Pattern
When adding a new Tauri command: define backend in `src-tauri/src/commands/<module>.rs`, then add the typed wrapper in `src/ipc/<module>.ts`. Update both files together.

```typescript
// Frontend (src/ipc/foo.ts)
export function fooAction(param: string): Promise<Result> {
  return invoke<Result>("foo_action", { param });
}
```

```rust
// Backend
#[tauri::command]
pub async fn foo_action(param: String) -> Result<ResultType, String> { ... }
```

### State Management
- Component-local `useState` for UI state
- `localStorage` for persistent preferences (locale, tab type, sidebar width)
- `useRef` to capture latest values in Tauri event listeners (prevents stale closures)

### AI Streaming Events
Streaming tokens emit on `ai-stream` Tauri events as `{ session_id, kind, delta, done }`. Always use a `mountedRef` guard in hooks that listen to events to prevent `setState` after unmount.

### PTY Output
Backend emits base64-encoded chunks on dynamic event `pty://data/{sessionId}`. Frontend decodes with `atob()` + `Uint8Array`.

## Critical Pitfalls

1. **Never unmount terminal tabs** — keep inactive xterm.js views with `visibility: hidden; pointer-events: none`. Unmounting causes resize crashes.
2. **AI providers are on-demand** — instantiated per request in `AiRouter::resolve()`, not cached. Config changes apply immediately.
3. **DB2 is Windows-only** — no macOS DB2 ODBC support. Do not add DB2 code paths that assume cross-platform availability.
4. **TLS** — `reqwest` uses `rustls-tls`; `tiberius` uses `native-tls`. OpenSSL is not required.
5. **Command risk gate** — commands pass through `CommandGuard::evaluate_risk()` before execution. Respect the `execution_mode` (always-confirm / graded / full-auto) setting.
6. **Chat history is capped at 20 messages** — older messages are dropped before sending to AI.
7. **Agent loops are capped at `max_agent_steps`** (default 5) — prevents runaway loops.

## Testing Conventions

- **Frontend**: Vitest + React Testing Library + jsdom. Files: `src/**/*.test.ts(x)`.
- **Rust**: `#[tokio::test]` for async tests. HTTP mocked with wiremock. Fixtures via tempfile.
- Tests mirror the module they test; co-located (`src/components/AiPanel/AiPanel.test.tsx`).

## Platform Differences

- Platform-specific Tauri config in `src-tauri/tauri.macos.conf.json` and `src-tauri/tauri.windows.conf.json` — these override the base `tauri.conf.json`.
- DB2 sidecar (`db2-sidecar/`) is a C# project compiled and downloaded as a binary on Windows CI only.
- `src-tauri/binaries/` is gitignored.

## Development Workflow

This project uses a Superpowers skill-based workflow. See [CLAUDE.md §Development Workflow](../CLAUDE.md) for the full skill invocation table (brainstorming → writing-plans → executing-plans → verification). Design specs go in `docs/superpowers/specs/`, plans in `docs/superpowers/plans/`.

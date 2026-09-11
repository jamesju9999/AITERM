# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Run the app in development
npm run tauri:dev          # Tauri dev server (first build is slow — Rust)

# Build
npm run build              # Frontend only (tsc + vite)
npm run tauri:build        # Full production build
npm run tauri:build -- --no-sign   # Build without the updater signing key (contributors)

# Lint
npm run lint               # ESLint

# Tests
npm run test               # Frontend Vitest suite
cd src-tauri && cargo test --workspace   # Rust unit + integration tests
                           # 一定要加 --workspace：src-tauri/Cargo.toml 同時是
                           # package 與 workspace root，bare `cargo test` 只會跑
                           # `app`，aiterm-core 與 aiterm-host 會被整批跳過而且
                           # 不會有任何徵兆（1347 條 vs 1582 條）。
npx tsc -b                 # Type check (what `npm run build` runs)
                           # NOT `tsc --noEmit`: the root tsconfig.json is a
                           # solution file ("files": []), so it checks nothing
                           # and always exits 0.
```

## Architecture

AITerm is a Tauri 2 desktop app: a React 19 frontend communicates with a Rust backend via **Tauri IPC (invoke + events)**. The typed IPC wrappers live in `src/ipc/`.

### Backend (`src-tauri/src/`)

| Module | Responsibility |
|--------|---------------|
| `pty/` | Pseudo-terminal lifecycle via `portable-pty`. `PtyManager` holds sessions by ID; PTY output streams as base64 chunks on `pty://data/{sessionId}` events. |
| `ai/` | Multi-provider AI router (OpenAI, Anthropic, Ollama, OpenAI-compat). Providers are instantiated on demand so config changes take effect without restart. Streaming AI tokens emit on `ai-stream` events. |
| `config/` | JSON config store + OS keyring for API keys. |
| `db/` | SQLx-backed database manager with adapters for PostgreSQL, MySQL, SQLite, ODBC. |
| `commands.rs` | All `#[tauri::command]` entry points. |

### Frontend (`src/`)

The shell of the app is `TerminalApp.tsx` (multi-tab state, keyboard shortcuts: `Ctrl+T` new tab, `Ctrl+W` close, `Ctrl+B` sidebar). `App.tsx` just wraps it.

**Terminal tab:** `TerminalView.tsx` mounts `xterm.js`, wires PTY events, and handles the `/ai`+`/agent` prefix trigger. Inactive terminal tabs are hidden via `visibility: hidden` + `pointer-events: none` instead of unmounting — xterm.js crashes on resize when the element has no dimensions.

**AI command flow:** User types `/ai <query>` → `parseAiPrefix.ts` parses it → `ai_query` invoke → `AiCommandReady { command, explanation, risk_level }` → `CommandPreview` shows it → execution gated by `risk_level` × user `execution_mode` (always-confirm / graded / full-auto).

**Agent loop:** `useAgentMission.ts` orchestrates autonomous multi-step execution with `max_agent_steps` limit; dangerous commands fall back to manual confirmation.

**Multi-turn chat:** `useAiChat.ts` manages message history + streaming with a `mountedRef` guard to prevent `setState` after unmount (Tauri listen race condition).

### IPC Patterns

- **PTY:** `pty_create(size) → sessionId` → write via `pty_write` → receive chunks on `pty://data/{sessionId}`
- **AI query:** `ai_query(query, sessionId)` + streaming via `ai-stream` events `{ session_id, kind, delta, done }`
- **AI errors:** `AiError` is a discriminated union on `kind`: `not_configured | network | auth_failed | rate_limit | model_error | invalid_input`

### State Management

- Component-local `useState` + `localStorage` for persistent UI state (tabs, settings)
- `LocaleContext` for i18n (en / zh-TW), strings in `src/lib/i18n.ts`
- Refs used throughout to avoid stale closures in Tauri event listeners

### Tests

- Frontend tests (`src/**/*.test.ts(x)`): Vitest + React Testing Library + jsdom
- Rust tests (`src-tauri/tests/`): wiremock for HTTP mocking, tempfile for fixtures

### Platform Support

- **Cross-platform requirement**: All features and bug fixes must work on **macOS, Windows, and Linux** unless the feature is explicitly platform-specific (e.g. DB2). Before finalizing any implementation, consider whether it uses platform-specific APIs, paths, shell behavior, or dependencies.
- **DB2 uses a Java (JDBC) sidecar** (`db2-sidecar-java/`) — cross-platform (macOS, Windows, Linux). The sidecar runs `java -jar db2sidecar.jar` and must be built locally (see `scripts/setup-db2-*.sh`). `src-tauri/binaries/` is gitignored; build the JAR before running `tauri:dev`.
- **uv is a bundled sidecar too** (`binaries/uv`), registered in `externalBin` on all three platform confs. `tauri-build`'s `build.rs` validates every `externalBin` entry exists on disk *at compile time* — so without first running the matching `scripts/setup-uv-{mac,linux,win}.{sh,ps1}`, not just `tauri:dev`/`tauri:build` but even `cd src-tauri && cargo test` or `cargo check` will fail with a missing resource-path error. Run the setup script for your platform first.
- **Platform-specific Tauri config**: use `src-tauri/tauri.{macos|windows|linux}.conf.json` to override `tauri.conf.json` per platform
- `src-tauri/binaries/` is gitignored — platform binaries are never committed; handle via CI steps or platform config overrides
- **TLS**: `reqwest` uses `rustls-tls`; `tiberius` uses `native-tls` (Schannel on Windows, SecureTransport on macOS) — OpenSSL is not required on any CI platform

## Development Workflow (Superpowers Skills)

This project uses a structured milestone-driven workflow. Always apply skills in this order:

| Situation | Skill to invoke |
|-----------|----------------|
| Starting any new feature / milestone | `superpowers:brainstorming` → explore intent + produce design spec |
| Have a spec, about to write code | `superpowers:writing-plans` → produce implementation plan in `docs/superpowers/plans/` |
| Have a plan, ready to execute | `superpowers:executing-plans` → step-by-step execution with review checkpoints |
| Encountering a bug or test failure | `superpowers:systematic-debugging` → root-cause first, no guessing |
| Implementation complete | `superpowers:verification-before-completion` → run tests + type check before claiming done |
| Before merging / wrapping up branch | `superpowers:requesting-code-review` → structured self-review pass |
| Writing tests | `superpowers:test-driven-development` → failing test before implementation |
| 2+ independent tasks in parallel | `superpowers:dispatching-parallel-agents` |

### File Conventions

- **Design specs** → `docs/superpowers/specs/YYYY-MM-DD-<feature>-design.md`
- **Implementation plans** → `docs/superpowers/plans/YYYY-MM-DD-<feature>.md`
- **Brainstorm artifacts** (HTML prototypes) → `.superpowers/brainstorm/`

## Coding Discipline

These are maintained as a skill so they don't sit in context every session:
see `.claude/skills/karpathy-guidelines/SKILL.md` (think before coding,
simplicity first, surgical changes, goal-driven execution). Invoke
`karpathy-guidelines` when writing, reviewing, or refactoring code.

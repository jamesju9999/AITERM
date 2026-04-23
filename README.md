# AITerm

Cross-platform AI terminal (Tauri 2 + React + Rust). **M0 — Skeleton.**

This milestone gives you a working Tauri window hosting `xterm.js` attached to
a live shell via `portable-pty`. No AI features yet.

## Prerequisites

- Rust 1.78+ (`rustup show`)
- Node 20+ (`node -v`)
- Windows 11 with WebView2 runtime (bundled) and MSVC build tools

On Windows, the shell priority is `pwsh.exe` → `powershell.exe` → `cmd.exe`.

## Develop

```bash
npm install
npm run tauri:dev
```

First build takes a few minutes (Rust). Subsequent runs are faster.

## Test

```bash
# Rust unit + integration tests
cd src-tauri
cargo test

# Frontend type check
cd ..
npx tsc --noEmit
```

## Project Layout

```
src-tauri/       Rust backend (Tauri + PTY)
  src/pty/       PTY module (session, manager, commands, events)
  tests/         Integration tests
src/             React frontend
  components/    TerminalView (xterm.js wrapper)
  ipc/           Typed Tauri invoke + event wrappers
docs/            Specs and implementation plans
```

# AITerm

**[🌐 Official Website](https://jamesju9999.github.io/aiterm-site/)** | [English](#english) | [繁體中文](#繁體中文)

---

<a id="english"></a>
# AITerm (English)

A powerful, cross-platform AI-enhanced terminal built with **Tauri 2**, **React 19**, and **Rust**. AITerm seamlessly integrates the traditional command-line experience with advanced AI capabilities, autonomous agents, database connectivity, and project requirement management.

## ✨ Key Features

- **Integrated AI Providers**: Out-of-the-box support for multiple LLM providers including OpenAI, Anthropic, and Ollama (for local inference).
- **AI Command Flow (`/ai`)**: Type `/ai <query>` directly in the terminal to generate and preview commands with risk-level assessment before execution.
- **Autonomous Agent Loop (`/agent`)**: Multi-step, goal-driven agentic execution for complex tasks, with built-in guardrails and fallback to manual confirmation for dangerous operations.
- **Multi-turn Chat Sidebar**: Persistent, context-aware AI chat directly beside your terminal for troubleshooting, code generation, and brainstorming.
- **Requirement Management System**: AI-assisted discussion for feature planning, automatically generating and saving specifications (SDD) into structured, project-managed directories.
- **Robust Terminal Engine**: Powered by `xterm.js` and `portable-pty` for a fast, native-feeling shell experience across Windows, macOS, and Linux, with OSC 133 shell integration markers support.
- **File Explorer**: Built-in sidebar file browser that automatically tracks and syncs with the current working directory of the active terminal session.
- **Multi-Database Connectivity**: Native support for PostgreSQL, MySQL, SQLite, and ODBC data sources via SQLx. DB2 connectivity is available on both **macOS and Windows** via a bundled Java JDBC sidecar (IBM `db2jcc4.jar`, no native drivers required).
- **i18n Support**: Full interface localization for English and Traditional Chinese (繁體中文), switchable at runtime.
- **Auto Update Check**: Built-in version checker in the About page that queries GitHub Releases and links directly to the specific release tag when an update is available.

## 🏗️ Architecture

AITerm uses a modern Desktop architecture communicating via Tauri IPC (invoke + events):
- **Frontend (`src/`)**: React 19 shell, `xterm.js` terminal views, component-local state management, and an intelligent frontend router for handling AI commands and streams.
- **Backend (`src-tauri/src/`)**: Rust-powered high-performance core handling PTY lifecycles, AI provider routing, SQLx-backed database connections, and secure local configuration (OS keyring).
- **DB2 Sidecar (`db2-sidecar-java/`)**: A self-contained Java process (Maven fat jar) that bridges Rust ↔ IBM JDBC via newline-delimited JSON on stdin/stdout. Bundled with Eclipse Temurin 21 JRE — no separate JDK or IBM client library installation required.

## 🚀 Getting Started

### Installing on macOS

> **"AITerm is damaged and can't be opened"** — This message appears because the app is not yet notarized with Apple. Run the following command in Terminal after dragging AITerm to Applications, then try opening it again:
> ```bash
> xattr -cr /Applications/AITerm.app
> ```
> Alternatively, right-click AITerm in Finder → **Open** → **Open** to bypass Gatekeeper once.

### Prerequisites (Development)
- Rust 1.78+ (`rustup show`)
- Node.js 20+ (`node -v`)
- _Windows only_: Windows 11 with WebView2 runtime and MSVC build tools.
- _DB2 support_: Run the platform setup script once to build the sidecar and download the bundled JRE:
  - macOS: `bash scripts/setup-db2-mac.sh`
  - Windows: `pwsh scripts/setup-db2-win.ps1`

### Development
```bash
npm install
npm run tauri:dev
```
> *Note: The first build will take a few minutes as Rust dependencies are compiled. Subsequent runs are much faster.*

### Testing
```bash
# Frontend tests (Vitest + React Testing Library)
npm run test

# Rust unit & integration tests
cd src-tauri && cargo test

# Type checking
npx tsc --noEmit
```

## 📂 Project Layout
```
src-tauri/          # Rust backend (Tauri + PTY + AI Router + DB)
  src/pty/          # PTY lifecycle and streaming
  src/ai/           # Multi-provider AI router
  src/db/           # Database adapters (PG, MySQL, SQLite, ODBC, DB2)
  tests/            # Rust integration tests
src/                # React 19 frontend
  components/       # TerminalView, ChatSidebar, FileExplorer, Settings, etc.
  ipc/              # Typed Tauri invoke + event wrappers
  lib/i18n.ts       # Localization strings (en / zh-TW)
db2-sidecar-java/   # Java JDBC bridge sidecar (Maven project)
scripts/            # Platform setup and cross-platform build launchers
docs/               # Specs and implementation plans
```

## 📄 License

This project is licensed under the [Apache License 2.0](LICENSE).

---

<a id="繁體中文"></a>
# AITerm (繁體中文)

一個強大且跨平台的 AI 智慧終端機，基於 **Tauri 2**、**React 19** 和 **Rust** 打造。AITerm 將傳統的命令列體驗與先進的 AI 能力、自主代理（Agent）、資料庫連線能力以及專案需求管理完美結合。

## ✨ 核心功能

- **多重 AI 供應商整合**：開箱即支援 OpenAI、Anthropic 以及 Ollama（本地端推論）等多種語言模型。
- **AI 指令工作流 (`/ai`)**：直接在終端機輸入 `/ai <查詢>` 即可生成指令，並在執行前提供風險等級評估與預覽。
- **自主代理循環 (`/agent`)**：針對複雜任務提供多步驟、目標導向的代理執行能力，內建安全防護機制，並在遇到危險操作時自動退回手動確認模式。
- **多輪對話側邊欄**：終端機旁內建具備上下文記憶的 AI 對話視窗，方便進行問題排解、程式碼生成與靈感發想。
- **需求管理系統**：提供 AI 輔助的系統功能討論，可自動生成規格文件 (SDD) 並將其儲存到專案管理的結構化目錄中。
- **強大的終端機引擎**：基於 `xterm.js` 與 `portable-pty`，在 Windows、macOS 與 Linux 上提供快速、原生的 Shell 體驗，並支援 OSC 133 Shell 整合標記。
- **檔案總管**：內建側邊欄檔案瀏覽器，可自動追蹤並同步目前活躍終端機的工作目錄。
- **多資料庫連線**：透過 SQLx 原生支援 PostgreSQL、MySQL、SQLite 與 ODBC。DB2 連線在 **macOS 與 Windows** 均可使用，透過內建 Java JDBC Sidecar（IBM `db2jcc4.jar`）實現，無需安裝任何原生驅動程式。
- **多語言介面**：完整支援英文與繁體中文介面，可於執行時即時切換。
- **自動更新檢查**：「關於」頁面內建版本檢查器，查詢 GitHub Releases 並在有新版本時直接連結至對應的 Release Tag 頁面。

## 🏗️ 系統架構

AITerm 採用現代化桌面應用架構，透過 Tauri IPC（invoke + events）進行通訊：
- **前端 (`src/`)**：使用 React 19，包含 `xterm.js` 終端機視圖、元件狀態管理，以及處理 AI 指令與資料流的智慧前端邏輯。
- **後端 (`src-tauri/src/`)**：由 Rust 驅動的高效能核心，負責處理 PTY 生命週期、AI 路由、基於 SQLx 的資料庫連線，以及安全的本地配置（OS Keyring）。
- **DB2 Sidecar (`db2-sidecar-java/`)**：獨立的 Java 行程（Maven fat jar），透過 stdin/stdout 的換行分隔 JSON 協議橋接 Rust ↔ IBM JDBC。內附 Eclipse Temurin 21 JRE，**無需另行安裝 JDK 或 IBM 用戶端函式庫**。

## 🚀 快速開始

### macOS 安裝說明

> **「AITerm 已損毀，無法打開」** — 這是因為 App 尚未通過 Apple 公證（notarization）。將 AITerm 拖入「應用程式」後，在「終端機」執行以下指令，再重新開啟即可：
> ```bash
> xattr -cr /Applications/AITerm.app
> ```
> 或在 Finder 中對 AITerm 按右鍵 → **打開** → **打開**，即可略過 Gatekeeper 一次。

### 環境要求（開發）
- Rust 1.78+ (`rustup show`)
- Node.js 20+ (`node -v`)
- _僅限 Windows_：Windows 11（包含 WebView2 執行階段與 MSVC 建置工具）。
- _DB2 支援_：執行一次平台設定腳本，以建置 Sidecar 並下載內附 JRE：
  - macOS：`bash scripts/setup-db2-mac.sh`
  - Windows：`pwsh scripts/setup-db2-win.ps1`

### 開發測試
```bash
npm install
npm run tauri:dev
```
> *提示：首次建置需要編譯 Rust 依賴套件，會花費幾分鐘的時間，之後的執行速度將大幅提升。*

### 執行測試
```bash
# 前端測試 (Vitest + React Testing Library)
npm run test

# Rust 單元測試與整合測試
cd src-tauri && cargo test

# 型別檢查
npx tsc --noEmit
```

## 📂 專案結構
```
src-tauri/          # Rust 後端 (Tauri + PTY + AI Router + DB)
  src/pty/          # PTY 生命週期與資料流處理
  src/ai/           # 多重 AI 供應商路由
  src/db/           # 資料庫介面卡 (PG、MySQL、SQLite、ODBC、DB2)
  tests/            # Rust 整合測試
src/                # React 19 前端
  components/       # TerminalView、對話側邊欄、檔案總管、設定等元件
  ipc/              # 型別安全的 Tauri IPC 封裝
  lib/i18n.ts       # 多語言字串 (en / zh-TW)
db2-sidecar-java/   # Java JDBC 橋接 Sidecar（Maven 專案）
scripts/            # 平台設定與跨平台建置腳本
docs/               # 規格文件與實作計畫
```

## 📄 授權

本專案採用 [Apache License 2.0](LICENSE) 授權。

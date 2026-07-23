<img width="1536" height="1024" alt="ChatGPT Image 2026年5月14日 下午08_28_16" src="https://github.com/user-attachments/assets/aa7f6739-ecfa-4cd4-9683-aafa2e4607e9" />

# AITerm

**[🌐 Official Website](https://jamesju9999.github.io/aiterm-site/)** | [English](#english) | [繁體中文](#繁體中文)

---

<a id="english"></a>
# AITerm (English)

A powerful, cross-platform AI-enhanced terminal built with **Tauri 2**, **React 19**, and **Rust**. AITerm seamlessly integrates the traditional command-line experience with advanced AI capabilities, autonomous agents, and project requirement management.

## ✨ Key Features

- **Integrated AI Providers**: Out-of-the-box support for multiple LLM providers including OpenAI, Anthropic, Ollama (local inference), and OpenAI-compatible endpoints.
- **AI Command Flow (`/ai`)**: Type `/ai <query>` directly in the terminal to generate and preview commands with risk-level assessment before execution.
- **Structured Command Blocks**: Every command and its output render as a distinct card instead of raw scrollback. The live terminal pane auto-expands only while a command is running and shrinks back down when idle, keeping the screen uncluttered.
- **Autonomous Agent Loop (`/agent`)**: Multi-step, goal-driven agentic execution for complex tasks, with built-in guardrails and fallback to manual confirmation for dangerous operations.
- **LoopStudio (Loop Engineering)**: A visual multi-agent orchestration workbench — an orchestrator breaks a goal down and dispatches sub-agents to work in parallel, with safety gates and a live execution trace, running autonomously until the goal is verified done instead of requiring back-and-forth confirmation.
- **Code Assistant**: A dedicated tab where the AI answers questions about any project directory — it scans the file tree, searches and reads source files on its own, can render Mermaid diagrams, and lets you export the conversation to Markdown.
- **Multi-turn Chat Sidebar**: Persistent, context-aware AI chat directly beside your terminal for troubleshooting, code generation, and brainstorming.
- **Knowledge Base**: Create a notebook pointed at a folder and AITerm chunks, embeds, and indexes every document inside it, so you can ask natural-language questions with answers that cite the source documents. Works with Ollama or any OpenAI-compatible embedding provider.
- **Multi-Database Connections**: Native support for PostgreSQL, MySQL, SQLite, and ODBC. DB2 works on both macOS and Windows via a built-in Java JDBC sidecar — no driver installation required.
- **File Explorer**: A built-in sidebar file browser kept in two-way sync with the terminal's working directory — navigate in the terminal and the explorer follows, or click the "cd" button (or the folder picker built into the command input) to jump the terminal to any folder instantly.
- **Command Search & Bookmarks**: `Ctrl+F` searches across the entire block history with live scroll-to-match; `Ctrl+Shift+R` opens a bookmarks picker to save and re-fill frequently used commands.
- **Multilingual UI**: Full support for English and Traditional Chinese, switchable at runtime with no restart required.
- **Telegram Remote Control**: Send commands to your terminal from your phone via a Telegram bot, with results streamed back to the same chat.
- **Requirement Management System**: AI-assisted discussion for feature planning, automatically generating and saving specifications (SDD) into structured, project-managed directories.
- **Robust Terminal Engine**: Powered by `xterm.js` and `portable-pty` for a fast, native-feeling shell experience across Windows, macOS, and Linux.

## 🏗️ Architecture

AITerm uses a modern Desktop architecture communicating via Tauri IPC (invoke + events):
- **Frontend (`src/`)**: React 19 shell, `xterm.js` terminal views, component-local state management, and an intelligent frontend router for handling AI commands and streams.
- **Backend (`src-tauri/src/`)**: Rust-powered high-performance core handling PTY lifecycles, AI provider routing, SQLx-backed database connections (including a Java JDBC sidecar for DB2), Telegram bot integration, and secure local configuration (OS keyring).

## 🚀 Getting Started

### Prerequisites
- Rust 1.78+ (`rustup show`)
- Node.js 20+ (`node -v`)
- _Windows only_: Windows 11 with WebView2 runtime and MSVC build tools.

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
src-tauri/       # Rust backend (Tauri + PTY + AI Router + DB)
  src/pty/       # PTY lifecycle and streaming
  src/ai/        # Multi-provider AI router
  tests/         # Rust integration tests
src/             # React 19 frontend
  components/    # TerminalView, ChatSidebar, etc.
  ipc/           # Typed Tauri invoke + event wrappers
docs/            # Specs and implementation plans
```

---

<a id="繁體中文"></a>
# AITerm (繁體中文)

一個強大且跨平台的 AI 智慧終端機，基於 **Tauri 2**、**React 19** 和 **Rust** 打造。AITerm 將傳統的命令列體驗與先進的 AI 能力、自主代理（Agent）以及專案需求管理完美結合。

## ✨ 核心功能

- **多重 AI 供應商整合**：開箱即支援 OpenAI、Anthropic、Ollama（本地端推論）以及 OpenAI 相容端點等多種語言模型。
- **AI 指令工作流 (`/ai`)**：直接在終端機輸入 `/ai <查詢>` 即可生成指令，並在執行前提供風險等級評估與預覽。
- **結構化指令區塊**：每個指令與其輸出都會渲染成獨立卡片，而非傳統終端機的原始捲動畫面；即時終端機視窗只在指令執行中才自動展開，閒置時自動收合，畫面保持乾淨。
- **自主代理循環 (`/agent`)**：針對複雜任務提供多步驟、目標導向的代理執行能力，內建安全防護機制，並在遇到危險操作時自動退回手動確認模式。
- **LoopStudio（循環工程）**：可視化的多代理協同工作台——主控代理拆解任務並分派子代理平行執行，搭配安全閘門與即時執行追蹤，自主循環直到目標驗證完成，不需要一問一答反覆確認。
- **程式庫協助**：獨立分頁，讓 AI 針對任意專案目錄回答問題——自動掃描檔案樹、搜尋並讀取原始碼，可繪製 Mermaid 圖表，對話還能匯出成 Markdown。
- **多輪對話側邊欄**：終端機旁內建具備上下文記憶的 AI 對話視窗，方便進行問題排解、程式碼生成與靈感發想。
- **知識庫**：建立筆記本並指定一個資料夾，AITerm 會自動切割、嵌入（Embedding）並索引其中的所有文件，之後即可用自然語言提問，回答附上原始文件引用來源；支援 Ollama 與任何 OpenAI 相容的 Embedding 供應商。
- **多資料庫連線**：原生支援 PostgreSQL、MySQL、SQLite、ODBC。DB2 透過內建 Java JDBC Sidecar 在 macOS 與 Windows 均可使用，無需安裝驅動程式。
- **檔案總管**：內建側邊欄檔案瀏覽器，與終端機工作目錄雙向同步——終端機切換路徑會即時反映在檔案總管，點擊「cd」按鈕或輸入框內建的資料夾選單也能立即把終端機切換到該目錄。
- **指令搜尋與書籤**：`Ctrl+F` 可跨所有區塊歷史搜尋指令並即時捲動至符合項目；`Ctrl+Shift+R` 開啟書籤選單，收藏常用指令並一鍵帶入輸入框。
- **多語言介面**：完整支援英文與繁體中文，可於執行時即時切換，不需重啟。
- **Telegram 遠端控制**：透過 Telegram Bot 從手機傳送指令至終端機，執行結果即時回傳至同一對話。
- **需求管理系統**：提供 AI 輔助的系統功能討論，可自動生成規格文件 (SDD) 並將其儲存到專案管理的結構化目錄中。
- **強大的終端機引擎**：基於 `xterm.js` 與 `portable-pty`，在 Windows、macOS 與 Linux 上提供快速、原生的 Shell 體驗。

## 🏗️ 系統架構

AITerm 採用現代化桌面應用架構，透過 Tauri IPC（invoke + events）進行通訊：
- **前端 (`src/`)**：使用 React 19，包含 `xterm.js` 終端機視圖、元件狀態管理，以及處理 AI 指令與資料流的智慧前端邏輯。
- **後端 (`src-tauri/src/`)**：由 Rust 驅動的高效能核心，負責處理 PTY 生命週期、AI 路由、基於 SQLx 的資料庫連線（含 DB2 專用的 Java JDBC Sidecar）、Telegram Bot 整合，以及安全的本地配置（OS Keyring）。

## 🚀 快速開始

### 環境要求
- Rust 1.78+ (`rustup show`)
- Node.js 20+ (`node -v`)
- _僅限 Windows_：Windows 11（包含 WebView2 執行階段與 MSVC 建置工具）。

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
src-tauri/       # Rust 後端 (Tauri + PTY + AI Router + DB)
  src/pty/       # PTY 生命週期與資料流處理
  src/ai/        # 多重 AI 供應商路由
  tests/         # Rust 整合測試
src/             # React 19 前端
  components/    # TerminalView、對話側邊欄等元件
  ipc/           # 型別安全的 Tauri IPC 封裝
docs/            # 規格文件與實作計畫
```

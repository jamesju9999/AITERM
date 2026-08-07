# MCP Marketplace Browser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "市集" Tab to the MCP settings page that lets users search Smithery Registry and one-click install MCP servers (run npx/pip + auto-add config).

**Architecture:** Frontend JS fetches Smithery REST API directly; user clicks Install → Rust `install_mcp_package` command streams install log via Tauri events → on success calls existing `add_mcp_server` IPC.

**Tech Stack:** React 19, TypeScript, Tauri 2, Rust tokio async, Smithery Registry API (`https://registry.smithery.ai`)

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `src/lib/smithery.ts` | Create | Smithery API types + fetch functions |
| `src/lib/smithery.test.ts` | Create | Unit tests for smithery.ts |
| `src/lib/i18n.ts` | Modify | Add marketplace i18n strings |
| `src-tauri/src/commands/mcp.rs` | Modify | Add `install_mcp_package` command |
| `src-tauri/src/lib.rs` | Modify | Register new command |
| `src/components/Settings/McpInstallTerminal.tsx` | Create | Terminal log display component |
| `src/components/Settings/McpMarketplaceTab.tsx` | Create | Marketplace search + install UI |
| `src/components/Settings/McpMarketplaceTab.test.tsx` | Create | Tests for marketplace tab |
| `src/components/Settings/McpServersPage.tsx` | Modify | Add tab switching |
| `src/components/Settings/McpServersPage.css` | Modify | Tab button styles |

---

### Task 1: Smithery API Client

**Files:**
- Create: `src/lib/smithery.ts`
- Create: `src/lib/smithery.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
// src/lib/smithery.test.ts
import { describe, it, expect, vi, beforeEach } from "vitest";
import { searchSmithery, getSmitheryServer } from "./smithery";

const mockFetch = vi.fn();
vi.stubGlobal("fetch", mockFetch);

beforeEach(() => {
  mockFetch.mockReset();
});

describe("searchSmithery", () => {
  it("returns servers from API response", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        servers: [
          {
            qualifiedName: "@mcp/server-filesystem",
            displayName: "Filesystem",
            description: "Read/write local filesystem",
            homepage: "https://example.com",
          },
        ],
        pagination: { currentPage: 1, pageSize: 20, totalCount: 1 },
      }),
    });

    const results = await searchSmithery("filesystem");
    expect(results).toHaveLength(1);
    expect(results[0].qualifiedName).toBe("@mcp/server-filesystem");
    expect(mockFetch).toHaveBeenCalledWith(
      "https://registry.smithery.ai/servers?q=filesystem&pageSize=20"
    );
  });

  it("throws on network error", async () => {
    mockFetch.mockResolvedValueOnce({ ok: false, status: 500 });
    await expect(searchSmithery("test")).rejects.toThrow("Smithery API error: 500");
  });
});

describe("getSmitheryServer", () => {
  it("returns server detail with connections", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        qualifiedName: "@mcp/server-filesystem",
        displayName: "Filesystem",
        description: "Read/write local filesystem",
        connections: [
          {
            type: "stdio",
            stdioFunction: {
              command: "npx",
              args: ["-y", "@modelcontextprotocol/server-filesystem"],
              env: {},
            },
          },
        ],
      }),
    });

    const detail = await getSmitheryServer("@mcp/server-filesystem");
    expect(detail.connections).toHaveLength(1);
    expect(detail.connections[0].type).toBe("stdio");
    expect(detail.connections[0].stdioFunction?.command).toBe("npx");
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
npx vitest run src/lib/smithery.test.ts
```
Expected: FAIL — module not found

- [ ] **Step 3: Implement smithery.ts**

```typescript
// src/lib/smithery.ts

export interface SmitheryServer {
  qualifiedName: string;
  displayName: string;
  description: string;
  homepage?: string;
}

export interface SmitheryConnection {
  type: "stdio" | "http" | "sse";
  stdioFunction?: {
    command: string;
    args: string[];
    env: Record<string, string>;
  };
  url?: string;
}

export interface SmitheryServerDetail {
  qualifiedName: string;
  displayName: string;
  description: string;
  connections: SmitheryConnection[];
}

const BASE = "https://registry.smithery.ai";

export async function searchSmithery(query: string): Promise<SmitheryServer[]> {
  const url = `${BASE}/servers?q=${encodeURIComponent(query)}&pageSize=20`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Smithery API error: ${res.status}`);
  const data = await res.json();
  return data.servers as SmitheryServer[];
}

export async function getSmitheryServer(
  qualifiedName: string
): Promise<SmitheryServerDetail> {
  const url = `${BASE}/servers/${encodeURIComponent(qualifiedName)}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Smithery API error: ${res.status}`);
  return res.json() as Promise<SmitheryServerDetail>;
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
npx vitest run src/lib/smithery.test.ts
```
Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add src/lib/smithery.ts src/lib/smithery.test.ts
git commit -m "feat(mcp-marketplace): add Smithery API client"
```

---

### Task 2: i18n Strings

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: Add strings to both zh-TW and en locales**

In `src/lib/i18n.ts`, inside the `zh` locale object (find the `mcp_toggle` key area and add after it):

```typescript
// after existing mcp_* keys in zh locale:
mcp_marketplace: "市集",
mcp_marketplace_installed: "已安裝",
mcp_marketplace_search_placeholder: "搜尋 MCP server...",
mcp_marketplace_install: "安裝",
mcp_marketplace_installing: "⟳ 安裝中...",
mcp_marketplace_installed_done: "✓ 已安裝",
mcp_marketplace_failed: "✗ 失敗，重試",
mcp_marketplace_copy_cmd: "複製指令",
mcp_marketplace_no_connections: "此 server 不支援自動安裝",
mcp_marketplace_network_error: "無法連線到市集，請檢查網路",
mcp_marketplace_node_missing: "請先安裝 Node.js：https://nodejs.org",
mcp_marketplace_python_missing: "請先安裝 Python：https://python.org",
mcp_marketplace_timeout: "安裝逾時，請手動執行指令",
mcp_marketplace_done_msg: "✓ 安裝完成，已新增到已安裝清單",
mcp_marketplace_terminal_title: "安裝記錄",
```

Add the same keys to the `en` locale:

```typescript
// after existing mcp_* keys in en locale:
mcp_marketplace: "Marketplace",
mcp_marketplace_installed: "Installed",
mcp_marketplace_search_placeholder: "Search MCP servers...",
mcp_marketplace_install: "Install",
mcp_marketplace_installing: "⟳ Installing...",
mcp_marketplace_installed_done: "✓ Installed",
mcp_marketplace_failed: "✗ Failed, retry",
mcp_marketplace_copy_cmd: "Copy Command",
mcp_marketplace_no_connections: "Auto-install not supported",
mcp_marketplace_network_error: "Cannot reach marketplace, check network",
mcp_marketplace_node_missing: "Please install Node.js: https://nodejs.org",
mcp_marketplace_python_missing: "Please install Python: https://python.org",
mcp_marketplace_timeout: "Install timed out, run command manually",
mcp_marketplace_done_msg: "✓ Install complete, added to installed list",
mcp_marketplace_terminal_title: "Install Log",
```

- [ ] **Step 2: Type-check**

```bash
npx tsc --noEmit
```
Expected: no errors

- [ ] **Step 3: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(mcp-marketplace): add i18n strings"
```

---

### Task 3: Rust install_mcp_package Command

**Files:**
- Modify: `src-tauri/src/commands/mcp.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the command to mcp.rs**

At the end of `src-tauri/src/commands/mcp.rs`, before the closing of the file, add:

```rust
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::{timeout, Duration};

#[derive(Clone, serde::Serialize)]
pub struct McpInstallLogEvent {
    pub session_id: String,
    pub line: String,
    pub is_error: bool,
    pub done: bool,
    pub success: bool,
}

#[tauri::command]
pub async fn install_mcp_package(
    app: tauri::AppHandle,
    command: String,
    args: Vec<String>,
    session_id: String,
) -> Result<(), String> {
    let emit = |line: String, is_error: bool, done: bool, success: bool| {
        let _ = app.emit(
            "mcp-install-log",
            McpInstallLogEvent {
                session_id: session_id.clone(),
                line,
                is_error,
                done,
                success,
            },
        );
    };

    let mut child = tokio::process::Command::new(&command)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            let msg = if e.kind() == std::io::ErrorKind::NotFound {
                format!("command not found: {command}")
            } else {
                e.to_string()
            };
            emit(msg.clone(), true, true, false);
            msg
        })?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let session_id_out = session_id.clone();
    let app_out = app.clone();
    let out_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app_out.emit(
                "mcp-install-log",
                McpInstallLogEvent {
                    session_id: session_id_out.clone(),
                    line,
                    is_error: false,
                    done: false,
                    success: false,
                },
            );
        }
    });

    let session_id_err = session_id.clone();
    let app_err = app.clone();
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app_err.emit(
                "mcp-install-log",
                McpInstallLogEvent {
                    session_id: session_id_err.clone(),
                    line,
                    is_error: true,
                    done: false,
                    success: false,
                },
            );
        }
    });

    let result = timeout(Duration::from_secs(60), child.wait()).await;

    let _ = out_task.await;
    let _ = err_task.await;

    match result {
        Ok(Ok(status)) => {
            if status.success() {
                emit(String::new(), false, true, true);
                Ok(())
            } else {
                let msg = format!("process exited with code {}", status.code().unwrap_or(-1));
                emit(msg.clone(), true, true, false);
                Err(msg)
            }
        }
        Ok(Err(e)) => {
            emit(e.to_string(), true, true, false);
            Err(e.to_string())
        }
        Err(_) => {
            let msg = "install timed out after 60 seconds".to_string();
            emit(msg.clone(), true, true, false);
            Err(msg)
        }
    }
}
```

- [ ] **Step 2: Register command in lib.rs**

In `src-tauri/src/lib.rs`, find the `invoke_handler` call and add `install_mcp_package` to the list:

```rust
// Find the existing pattern like:
tauri::generate_handler![
    // ... existing commands ...
    commands::mcp::set_mcp_enabled,
    commands::mcp::update_mcp_server,
    // add:
    commands::mcp::install_mcp_package,
]
```

- [ ] **Step 3: Build to verify Rust compiles**

```bash
cd src-tauri && cargo build 2>&1 | tail -20
```
Expected: `Finished` with no errors

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/mcp.rs src-tauri/src/lib.rs
git commit -m "feat(mcp-marketplace): add install_mcp_package Rust command"
```

---

### Task 4: McpInstallTerminal Component

**Files:**
- Create: `src/components/Settings/McpInstallTerminal.tsx`

- [ ] **Step 1: Create the component**

```tsx
// src/components/Settings/McpInstallTerminal.tsx
import { useEffect, useRef } from "react";

export interface InstallLogLine {
  text: string;
  isError: boolean;
}

interface Props {
  command: string;
  args: string[];
  lines: InstallLogLine[];
  onClose: () => void;
}

export function McpInstallTerminal({ command, args, lines, onClose }: Props) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [lines]);

  return (
    <div
      style={{
        position: "fixed",
        bottom: 0,
        left: 0,
        right: 0,
        height: 160,
        background: "#0a0a0a",
        borderTop: "1px solid #2a2a2a",
        display: "flex",
        flexDirection: "column",
        animation: "slideUp 0.2s ease-out",
        zIndex: 100,
      }}
    >
      <style>{`@keyframes slideUp { from { transform: translateY(100%); } to { transform: translateY(0); } }`}</style>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          padding: "4px 10px",
          borderBottom: "1px solid #1e1e1e",
          flexShrink: 0,
        }}
      >
        <span style={{ fontSize: 11, color: "#555" }}>安裝記錄</span>
        <button
          onClick={onClose}
          style={{
            background: "none",
            border: "none",
            color: "#555",
            cursor: "pointer",
            fontSize: 14,
            lineHeight: 1,
            padding: "0 2px",
          }}
        >
          ×
        </button>
      </div>
      <div
        style={{
          flex: 1,
          overflowY: "auto",
          padding: "6px 10px",
          fontFamily: "monospace",
          fontSize: 11,
        }}
      >
        <div style={{ color: "#555" }}>
          $ {command} {args.join(" ")}
        </div>
        {lines.map((line, i) => (
          <div
            key={i}
            style={{ color: line.isError ? "#f87171" : "#4ade80", wordBreak: "break-all" }}
          >
            {line.text}
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Type-check**

```bash
npx tsc --noEmit
```
Expected: no errors

- [ ] **Step 3: Commit**

```bash
git add src/components/Settings/McpInstallTerminal.tsx
git commit -m "feat(mcp-marketplace): add McpInstallTerminal component"
```

---

### Task 5: McpMarketplaceTab Component

**Files:**
- Create: `src/components/Settings/McpMarketplaceTab.tsx`
- Create: `src/components/Settings/McpMarketplaceTab.test.tsx`

- [ ] **Step 1: Write the failing test**

```tsx
// src/components/Settings/McpMarketplaceTab.test.tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { McpMarketplaceTab } from "./McpMarketplaceTab";

vi.mock("../../lib/smithery", () => ({
  searchSmithery: vi.fn(),
  getSmitheryServer: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("../../contexts/LocaleContext", () => ({
  useLocale: () => ({
    t: (k: string) => k,
  }),
}));

import { searchSmithery, getSmitheryServer } from "../../lib/smithery";
import { invoke } from "@tauri-apps/api/core";

const mockSearch = searchSmithery as ReturnType<typeof vi.fn>;
const mockGetDetail = getSmitheryServer as ReturnType<typeof vi.fn>;
const mockInvoke = invoke as ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.clearAllMocks();
  mockInvoke.mockResolvedValue(undefined);
});

describe("McpMarketplaceTab", () => {
  it("renders search input", () => {
    render(<McpMarketplaceTab onInstalled={() => {}} />);
    expect(screen.getByPlaceholderText("mcp_marketplace_search_placeholder")).toBeTruthy();
  });

  it("shows results after search", async () => {
    mockSearch.mockResolvedValueOnce([
      {
        qualifiedName: "@mcp/server-filesystem",
        displayName: "Filesystem",
        description: "Read/write files",
      },
    ]);

    render(<McpMarketplaceTab onInstalled={() => {}} />);
    const input = screen.getByPlaceholderText("mcp_marketplace_search_placeholder");

    await act(async () => {
      fireEvent.change(input, { target: { value: "filesystem" } });
      await new Promise((r) => setTimeout(r, 600)); // debounce
    });

    await waitFor(() => {
      expect(screen.getByText("Filesystem")).toBeTruthy();
    });
  });

  it("shows network error when search throws", async () => {
    mockSearch.mockRejectedValueOnce(new Error("network error"));

    render(<McpMarketplaceTab onInstalled={() => {}} />);
    const input = screen.getByPlaceholderText("mcp_marketplace_search_placeholder");

    await act(async () => {
      fireEvent.change(input, { target: { value: "test" } });
      await new Promise((r) => setTimeout(r, 600));
    });

    await waitFor(() => {
      expect(screen.getByText("mcp_marketplace_network_error")).toBeTruthy();
    });
  });

  it("shows copy button when server has no connections", async () => {
    mockSearch.mockResolvedValueOnce([
      { qualifiedName: "@mcp/no-conn", displayName: "No Conn", description: "" },
    ]);
    mockGetDetail.mockResolvedValueOnce({
      qualifiedName: "@mcp/no-conn",
      displayName: "No Conn",
      description: "",
      connections: [],
    });

    render(<McpMarketplaceTab onInstalled={() => {}} />);
    const input = screen.getByPlaceholderText("mcp_marketplace_search_placeholder");

    await act(async () => {
      fireEvent.change(input, { target: { value: "no-conn" } });
      await new Promise((r) => setTimeout(r, 600));
    });

    await waitFor(() => screen.getByText("No Conn"));

    const installBtn = screen.getByText("mcp_marketplace_install");
    await act(async () => {
      fireEvent.click(installBtn);
    });

    await waitFor(() => {
      expect(screen.getByText("mcp_marketplace_copy_cmd")).toBeTruthy();
    });
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
npx vitest run src/components/Settings/McpMarketplaceTab.test.tsx
```
Expected: FAIL — module not found

- [ ] **Step 3: Implement McpMarketplaceTab.tsx**

```tsx
// src/components/Settings/McpMarketplaceTab.tsx
import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { searchSmithery, getSmitheryServer, SmitheryServer, SmitheryServerDetail } from "../../lib/smithery";
import { McpInstallTerminal, InstallLogLine } from "./McpInstallTerminal";
import { useLocale } from "../../contexts/LocaleContext";
import type { McpServerInput } from "../../ipc/mcp";

interface Props {
  onInstalled: () => void;
}

type InstallStatus = "idle" | "fetching" | "running" | "success" | "error" | "no-connections";

interface ServerInstallState {
  status: InstallStatus;
  detail?: SmitheryServerDetail;
  logs: InstallLogLine[];
}

export function McpMarketplaceTab({ onInstalled }: Props) {
  const { t } = useLocale();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SmitheryServer[]>([]);
  const [isSearching, setIsSearching] = useState(false);
  const [networkError, setNetworkError] = useState(false);
  const [installStates, setInstallStates] = useState<Record<string, ServerInstallState>>({});
  const [activeTerminal, setActiveTerminal] = useState<string | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout>>();

  useEffect(() => {
    clearTimeout(debounceRef.current);
    if (!query.trim()) {
      setResults([]);
      setNetworkError(false);
      return;
    }
    debounceRef.current = setTimeout(async () => {
      setIsSearching(true);
      setNetworkError(false);
      try {
        const servers = await searchSmithery(query);
        setResults(servers);
      } catch {
        setNetworkError(true);
        setResults([]);
      } finally {
        setIsSearching(false);
      }
    }, 500);
    return () => clearTimeout(debounceRef.current);
  }, [query]);

  const handleInstall = async (server: SmitheryServer) => {
    const qn = server.qualifiedName;

    // Step 1: fetch detail to get connections
    setInstallStates((prev) => ({
      ...prev,
      [qn]: { status: "fetching", logs: [] },
    }));

    let detail: SmitheryServerDetail;
    try {
      detail = await getSmitheryServer(qn);
    } catch {
      setInstallStates((prev) => ({
        ...prev,
        [qn]: { status: "error", logs: [{ text: "Failed to fetch server details", isError: true }] },
      }));
      return;
    }

    if (!detail.connections || detail.connections.length === 0) {
      setInstallStates((prev) => ({
        ...prev,
        [qn]: { status: "no-connections", detail, logs: [] },
      }));
      return;
    }

    const conn = detail.connections[0];
    if (conn.type !== "stdio" || !conn.stdioFunction) {
      setInstallStates((prev) => ({
        ...prev,
        [qn]: { status: "no-connections", detail, logs: [] },
      }));
      return;
    }

    const { command, args } = conn.stdioFunction;
    const sessionId = `mcp-install-${qn}`;

    setInstallStates((prev) => ({
      ...prev,
      [qn]: { status: "running", detail, logs: [] },
    }));
    setActiveTerminal(qn);

    // Step 2: listen for log events
    const unlisten = await listen<{ session_id: string; line: string; is_error: boolean; done: boolean; success: boolean }>(
      "mcp-install-log",
      (event) => {
        if (event.payload.session_id !== sessionId) return;

        if (event.payload.done) {
          if (event.payload.success) {
            // Step 3: add to installed list
            const input: McpServerInput = {
              name: detail.displayName || qn,
              enabled: true,
              transport: "stdio",
              command,
              args,
              env: conn.stdioFunction!.env ?? {},
            };
            invoke("add_mcp_server", { input }).then(() => {
              setInstallStates((prev) => ({
                ...prev,
                [qn]: {
                  ...prev[qn],
                  status: "success",
                  logs: [
                    ...(prev[qn]?.logs ?? []),
                    { text: t("mcp_marketplace_done_msg"), isError: false },
                  ],
                },
              }));
              setTimeout(() => {
                setActiveTerminal(null);
                onInstalled();
              }, 3000);
            });
          } else {
            setInstallStates((prev) => ({
              ...prev,
              [qn]: { ...prev[qn], status: "error" },
            }));
          }
          unlisten();
        } else if (event.payload.line) {
          setInstallStates((prev) => ({
            ...prev,
            [qn]: {
              ...prev[qn],
              logs: [
                ...(prev[qn]?.logs ?? []),
                { text: event.payload.line, isError: event.payload.is_error },
              ],
            },
          }));
        }
      }
    );

    // Step 4: invoke install command
    try {
      await invoke("install_mcp_package", { command, args, sessionId });
    } catch (e) {
      // error already emitted via event
    }
  };

  const getInstallButtonLabel = (qn: string) => {
    const state = installStates[qn];
    if (!state) return t("mcp_marketplace_install");
    switch (state.status) {
      case "fetching":
      case "running": return t("mcp_marketplace_installing");
      case "success": return t("mcp_marketplace_installed_done");
      case "error": return t("mcp_marketplace_failed");
      case "no-connections": return t("mcp_marketplace_copy_cmd");
      default: return t("mcp_marketplace_install");
    }
  };

  const isInstallDisabled = (qn: string) => {
    const s = installStates[qn]?.status;
    return s === "fetching" || s === "running" || s === "success";
  };

  const activeState = activeTerminal ? installStates[activeTerminal] : null;
  const activeDetail = activeState?.detail;
  const activeConn = activeDetail?.connections?.[0];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", paddingBottom: activeTerminal ? 160 : 0 }}>
      <div style={{ padding: "10px 16px 8px" }}>
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("mcp_marketplace_search_placeholder")}
          style={{
            width: "100%",
            background: "#111",
            border: "1px solid #2a2a2a",
            borderRadius: 6,
            padding: "7px 10px",
            color: "#ccc",
            fontSize: 13,
            outline: "none",
            boxSizing: "border-box",
          }}
        />
        {networkError && (
          <div style={{ marginTop: 6, fontSize: 12, color: "#f87171" }}>
            {t("mcp_marketplace_network_error")}
          </div>
        )}
      </div>

      <div style={{ flex: 1, overflowY: "auto", padding: "0 16px 16px" }}>
        {isSearching && (
          <div style={{ color: "#555", fontSize: 12, padding: "8px 0" }}>搜尋中...</div>
        )}
        {results.map((server) => (
          <div
            key={server.qualifiedName}
            style={{
              background: "#111",
              border: "1px solid #2a2a2a",
              borderRadius: 6,
              padding: "10px 12px",
              marginBottom: 8,
              display: "flex",
              justifyContent: "space-between",
              alignItems: "flex-start",
              gap: 10,
            }}
          >
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ color: "#ccc", fontSize: 13, fontWeight: 500 }}>
                {server.displayName || server.qualifiedName}
              </div>
              <div style={{ color: "#555", fontSize: 11, marginTop: 2, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                {server.description}
              </div>
              <div style={{ color: "#444", fontSize: 10, marginTop: 2 }}>
                {server.qualifiedName}
              </div>
            </div>
            <button
              onClick={() => handleInstall(server)}
              disabled={isInstallDisabled(server.qualifiedName)}
              style={{
                flexShrink: 0,
                background: installStates[server.qualifiedName]?.status === "success" ? "#1a3a1e" : "#1a2a1e",
                border: `1px solid ${installStates[server.qualifiedName]?.status === "error" ? "#7c3a3a" : "#34d399"}`,
                color: installStates[server.qualifiedName]?.status === "error" ? "#f87171" : "#34d399",
                borderRadius: 4,
                padding: "4px 12px",
                fontSize: 12,
                cursor: isInstallDisabled(server.qualifiedName) ? "default" : "pointer",
                opacity: isInstallDisabled(server.qualifiedName) ? 0.7 : 1,
                whiteSpace: "nowrap",
              }}
            >
              {getInstallButtonLabel(server.qualifiedName)}
            </button>
          </div>
        ))}
      </div>

      {activeTerminal && activeDetail && activeConn?.stdioFunction && (
        <McpInstallTerminal
          command={activeConn.stdioFunction.command}
          args={activeConn.stdioFunction.args}
          lines={activeState?.logs ?? []}
          onClose={() => setActiveTerminal(null)}
        />
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run tests**

```bash
npx vitest run src/components/Settings/McpMarketplaceTab.test.tsx
```
Expected: PASS

- [ ] **Step 5: Type-check**

```bash
npx tsc --noEmit
```
Expected: no errors

- [ ] **Step 6: Commit**

```bash
git add src/components/Settings/McpMarketplaceTab.tsx src/components/Settings/McpMarketplaceTab.test.tsx
git commit -m "feat(mcp-marketplace): add McpMarketplaceTab component"
```

---

### Task 6: McpServersPage Tab Switching

**Files:**
- Modify: `src/components/Settings/McpServersPage.tsx`
- Modify: `src/components/Settings/McpServersPage.css`

- [ ] **Step 1: Read current McpServersPage.tsx to understand structure**

Read `src/components/Settings/McpServersPage.tsx` before editing. The tab state and import need to be added near the top; the tab buttons and `<McpMarketplaceTab>` render at the appropriate place in the JSX.

- [ ] **Step 2: Add activeTab state and import**

At the top of `McpServersPage.tsx`, add the import:

```typescript
import { McpMarketplaceTab } from "./McpMarketplaceTab";
```

Inside the component function, add the state:

```typescript
const [activeTab, setActiveTab] = useState<"installed" | "marketplace">("installed");
```

- [ ] **Step 3: Add tab buttons to JSX**

In the JSX return, just before the existing server list content (after the page header/title `<div>`), add the tab row:

```tsx
<div className="mcp-tab-row">
  <button
    className={`mcp-tab-btn${activeTab === "installed" ? " active" : ""}`}
    onClick={() => setActiveTab("installed")}
  >
    {t("mcp_marketplace_installed")}
  </button>
  <button
    className={`mcp-tab-btn${activeTab === "marketplace" ? " active" : ""}`}
    onClick={() => setActiveTab("marketplace")}
  >
    🌐 {t("mcp_marketplace")}
  </button>
</div>
```

- [ ] **Step 4: Conditionally render tab content**

Wrap the existing server list in `{activeTab === "installed" && ...}`, and add marketplace tab content after:

```tsx
{activeTab === "installed" && (
  /* existing server list JSX — no changes inside */
  <> ... </>
)}
{activeTab === "marketplace" && (
  <McpMarketplaceTab
    onInstalled={() => setActiveTab("installed")}
  />
)}
```

- [ ] **Step 5: Add CSS to McpServersPage.css**

```css
.mcp-tab-row {
  display: flex;
  gap: 2px;
  border-bottom: 1px solid #2a2a2a;
  margin-bottom: 12px;
  padding: 0 4px;
}

.mcp-tab-btn {
  background: none;
  border: none;
  border-bottom: 2px solid transparent;
  color: #555;
  cursor: pointer;
  font-size: 13px;
  padding: 6px 14px;
  margin-bottom: -1px;
  transition: color 0.15s;
}

.mcp-tab-btn:hover {
  color: #888;
}

.mcp-tab-btn.active {
  color: #34d399;
  border-bottom-color: #34d399;
}
```

- [ ] **Step 6: Run all frontend tests**

```bash
npm run test
```
Expected: all tests pass

- [ ] **Step 7: Type-check**

```bash
npx tsc --noEmit
```
Expected: no errors

- [ ] **Step 8: Commit**

```bash
git add src/components/Settings/McpServersPage.tsx src/components/Settings/McpServersPage.css
git commit -m "feat(mcp-marketplace): add tab switching to McpServersPage"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Task |
|-----------------|------|
| Tab 切換（已安裝 / 市集） | Task 6 |
| 搜尋 debounce 500ms | Task 5 (`McpMarketplaceTab`) |
| Smithery API 搜尋 + 詳細 | Task 1 (`smithery.ts`) |
| install_mcp_package Rust 命令 | Task 3 |
| mcp-install-log 事件 stream | Task 3 |
| 底部 terminal 面板 160px slide-in | Task 4 (`McpInstallTerminal`) |
| 按鈕狀態機 (安裝/安裝中/已安裝/失敗) | Task 5 |
| 無 connections → 複製指令 | Task 5 |
| 安裝成功 → add_mcp_server → 切回已安裝 | Task 5 |
| 60 秒逾時 | Task 3 (tokio timeout) |
| Node.js/Python not found 錯誤 | Task 3 (`ErrorKind::NotFound`) |
| Smithery API 網路錯誤提示 | Task 5 |
| i18n strings | Task 2 |
| CSP 設定 | N/A — tauri.conf.json CSP is already `null` (no restriction) |

All requirements covered. No TBD placeholders. Types are consistent across tasks.

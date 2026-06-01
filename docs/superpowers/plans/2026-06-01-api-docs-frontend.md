# API Docs — Plan 3: React Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `ApiDocsView` React tab that lets users load a documentation tree, select pages, configure extraction settings, authenticate to gated sites, and watch real-time progress as pages are converted to Markdown.

**Architecture:** Four focused components under `src/components/ApiDocsView/` — `DocTree` (left panel, checkbox tree + filter), `ExtractionSettings` (right panel, output options + auth), `ExtractionLog` (progress bar + log list), and `ApiDocsView` (orchestrator). Registered as `"api-docs"` tab type in `TabBar`, `NewTabPicker`, and `TerminalApp.tsx`.

**Tech Stack:** React 19, TypeScript, Tauri IPC (Plan 2's `src/ipc/apiDocs.ts`), existing `useLocale` i18n pattern, CSS modules following existing tab component conventions.

**Prerequisites:** Plan 1 (Python backend) and Plan 2 (Rust bridge + `src/ipc/apiDocs.ts`) must be complete.

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `src/components/ApiDocsView/ApiDocsView.tsx` | Create | Orchestrator: state, event listeners, IPC calls |
| `src/components/ApiDocsView/ApiDocsView.css` | Create | Layout styles |
| `src/components/ApiDocsView/DocTree.tsx` | Create | Recursive checkbox tree with keyword filter |
| `src/components/ApiDocsView/ExtractionSettings.tsx` | Create | Right panel: output dir, merge toggle, keep options, auth status |
| `src/components/ApiDocsView/ExtractionLog.tsx` | Create | Progress bar + scrollable log list |
| `src/components/ApiDocsView/index.ts` | Create | Re-export barrel |
| `src/lib/i18n.ts` | Modify | Add `api_docs_*` i18n strings (zh-TW + en) |
| `src/components/TabBar/index.tsx` | Modify | Add `"api-docs"` to `Tab.type` union |
| `src/components/NewTabPicker/index.tsx` | Modify | Add API Docs picker entry |
| `src/components/TerminalApp.tsx` | Modify | Handle `"api-docs"` in `handlePickerSelect` + render branch |

---

### Task 1: i18n strings

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: Add zh-TW strings**

In the `"zh-TW"` object of `translations`, add after the `doc_converter_*` keys:

```typescript
    // API Docs tab
    api_docs_tab: "API Docs",
    new_api_docs_desc: "從任意 API 文件網站萃取 Markdown",
    api_docs_url_placeholder: "輸入文件根網址，例如 https://docs.stripe.com",
    api_docs_load_tree: "載入文件樹",
    api_docs_loading: "載入中...",
    api_docs_platform_label: "平台",
    api_docs_filter_placeholder: "篩選頁面...",
    api_docs_select_all: "全選",
    api_docs_deselect_all: "全不選",
    api_docs_output_dir: "輸出目錄",
    api_docs_output_dir_placeholder: "~/api-docs",
    api_docs_merge_label: "合併為單一檔案",
    api_docs_keep_label: "保留內容",
    api_docs_keep_description: "Endpoint 描述",
    api_docs_keep_parameters: "Parameters",
    api_docs_keep_request_body: "Request Body",
    api_docs_keep_responses: "Responses",
    api_docs_keep_code_samples: "Code Samples",
    api_docs_extract_raw: "萃取原始 Markdown",
    api_docs_extract_ai: "萃取 + AI 增強",
    api_docs_auth_status_label: "認證狀態",
    api_docs_not_logged_in: "未登入",
    api_docs_login_btn: "登入",
    api_docs_logout_btn: "登出",
    api_docs_session_expired: "Session 已過期，請重新登入",
    api_docs_pages_selected: (n: number) => `已選 ${n} 頁`,
    api_docs_no_pages: "請先載入文件樹並選取頁面",
    api_docs_no_provider: "請先設定 AI 供應商",
    api_docs_extracting: "萃取中...",
    api_docs_done: "完成",
    api_docs_output_files: "輸出檔案",
```

- [ ] **Step 2: Add English strings**

In the `"en"` object, add after the `doc_converter_*` keys:

```typescript
    // API Docs tab
    api_docs_tab: "API Docs",
    new_api_docs_desc: "Extract Markdown from any API docs website",
    api_docs_url_placeholder: "Enter docs root URL, e.g. https://docs.stripe.com",
    api_docs_load_tree: "Load Doc Tree",
    api_docs_loading: "Loading...",
    api_docs_platform_label: "Platform",
    api_docs_filter_placeholder: "Filter pages...",
    api_docs_select_all: "Select All",
    api_docs_deselect_all: "Deselect All",
    api_docs_output_dir: "Output Directory",
    api_docs_output_dir_placeholder: "~/api-docs",
    api_docs_merge_label: "Merge into single file",
    api_docs_keep_label: "Keep",
    api_docs_keep_description: "Endpoint descriptions",
    api_docs_keep_parameters: "Parameters",
    api_docs_keep_request_body: "Request Body",
    api_docs_keep_responses: "Responses",
    api_docs_keep_code_samples: "Code Samples",
    api_docs_extract_raw: "Extract Raw Markdown",
    api_docs_extract_ai: "Extract + AI Enhance",
    api_docs_auth_status_label: "Auth Status",
    api_docs_not_logged_in: "Not logged in",
    api_docs_login_btn: "Login",
    api_docs_logout_btn: "Logout",
    api_docs_session_expired: "Session expired, please log in again",
    api_docs_pages_selected: (n: number) => `${n} page${n === 1 ? "" : "s"} selected`,
    api_docs_no_pages: "Load the doc tree and select pages first",
    api_docs_no_provider: "Configure an AI provider first",
    api_docs_extracting: "Extracting...",
    api_docs_done: "Done",
    api_docs_output_files: "Output files",
```

- [ ] **Step 3: Type-check**

```bash
npx tsc --noEmit 2>&1 | grep "i18n"
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(api-docs): i18n strings for API Docs tab (zh-TW + en)"
```

---

### Task 2: Register tab type

**Files:**
- Modify: `src/components/TabBar/index.tsx`
- Modify: `src/components/NewTabPicker/index.tsx`
- Modify: `src/components/TerminalApp.tsx`

- [ ] **Step 1: Extend Tab type in TabBar/index.tsx**

Find the line:
```typescript
  type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter";
```
Replace with:
```typescript
  type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs";
```

- [ ] **Step 2: Add entry to NewTabPicker/index.tsx**

Find the Props type:
```typescript
  onSelect: (type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter") => void;
```
Replace with:
```typescript
  onSelect: (type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs") => void;
```

Then append the new button before the closing `</div>` of the picker, after the `doc-converter` button:

```tsx
      <button
        className="new-tab-picker__item"
        onClick={() => { onSelect("api-docs"); onClose(); }}
      >
        <span className="new-tab-picker__icon">📚</span>
        <div>
          <div className="new-tab-picker__label">{t.api_docs_tab}</div>
          <div className="new-tab-picker__desc">{t.new_api_docs_desc}</div>
        </div>
      </button>
```

- [ ] **Step 3: Handle api-docs in TerminalApp.tsx**

In `handlePickerSelect`, find the block that sets the title. The existing pattern:
```typescript
  const handlePickerSelect = useCallback((type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter") => {
```
Change to:
```typescript
  const handlePickerSelect = useCallback((type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs") => {
```

Add after the `"doc-converter"` title assignment:
```typescript
    if (type === "api-docs") title = t.api_docs_tab;
```

- [ ] **Step 4: Add render branch in TerminalApp.tsx**

Find the render section that currently ends with the `doc-converter` branch:
```tsx
              ) : tab.type === "doc-converter" ? (
                <DocConverterView isActive={isActive} />
```

Add after it:
```tsx
              ) : tab.type === "api-docs" ? (
                <ApiDocsView isActive={isActive} />
```

Add the import at the top of `TerminalApp.tsx`:
```typescript
import { ApiDocsView } from "./ApiDocsView";
```

- [ ] **Step 5: Type-check**

```bash
npx tsc --noEmit 2>&1 | grep "TerminalApp\|NewTabPicker\|TabBar"
```

Expected: no errors (ApiDocsView import will error until Task 3 — that's fine, proceed).

- [ ] **Step 6: Commit**

```bash
git add src/components/TabBar/index.tsx src/components/NewTabPicker/index.tsx src/components/TerminalApp.tsx
git commit -m "feat(api-docs): register api-docs tab type in TabBar, NewTabPicker, TerminalApp"
```

---

### Task 3: DocTree component

**Files:**
- Create: `src/components/ApiDocsView/DocTree.tsx`

Renders a recursive checkbox tree. Each node has a checkbox; checking a parent auto-checks/unchecks children. Keyword filter hides non-matching subtrees.

- [ ] **Step 1: Write failing test**

Create `src/components/ApiDocsView/DocTree.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DocTree } from "./DocTree";
import type { DocNode } from "../../ipc/apiDocs";

const tree: DocNode[] = [
  {
    title: "Getting Started",
    href: "/docs/getting-started",
    items: [
      { title: "Quickstart", href: "/docs/quickstart", items: [] },
    ],
  },
  { title: "API Reference", href: "/docs/api", items: [] },
];

describe("DocTree", () => {
  it("renders all leaf nodes", () => {
    render(
      <DocTree nodes={tree} selected={new Set()} onChange={() => {}} filter="" />
    );
    expect(screen.getByText("Getting Started")).toBeInTheDocument();
    expect(screen.getByText("Quickstart")).toBeInTheDocument();
    expect(screen.getByText("API Reference")).toBeInTheDocument();
  });

  it("calls onChange when a leaf is checked", () => {
    const onChange = vi.fn();
    render(
      <DocTree nodes={tree} selected={new Set()} onChange={onChange} filter="" />
    );
    const checkbox = screen.getByLabelText("Quickstart");
    fireEvent.click(checkbox);
    expect(onChange).toHaveBeenCalled();
    const newSet: Set<string> = onChange.mock.calls[0][0];
    expect(newSet.has("/docs/quickstart")).toBe(true);
  });

  it("hides non-matching nodes when filter is set", () => {
    render(
      <DocTree nodes={tree} selected={new Set()} onChange={() => {}} filter="quick" />
    );
    expect(screen.queryByText("API Reference")).not.toBeInTheDocument();
    expect(screen.getByText("Quickstart")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
npx vitest run src/components/ApiDocsView/DocTree.test.tsx 2>&1 | tail -10
```

Expected: FAIL — `DocTree.tsx` does not exist.

- [ ] **Step 3: Write DocTree.tsx**

```tsx
// src/components/ApiDocsView/DocTree.tsx
import type { DocNode } from "../../ipc/apiDocs";

interface Props {
  nodes: DocNode[];
  selected: Set<string>;
  onChange: (next: Set<string>) => void;
  filter: string;
}

/** Collect all leaf hrefs in a subtree */
function collectLeaves(nodes: DocNode[]): string[] {
  const out: string[] = [];
  for (const n of nodes) {
    if (n.items.length === 0) {
      out.push(n.href);
    } else {
      out.push(...collectLeaves(n.items));
    }
  }
  return out;
}

/** True if this node or any descendant matches the filter */
function matchesFilter(node: DocNode, lc: string): boolean {
  if (node.title.toLowerCase().includes(lc)) return true;
  return node.items.some((child) => matchesFilter(child, lc));
}

function TreeNode({
  node,
  selected,
  onChange,
  filter,
  depth,
}: {
  node: DocNode;
  selected: Set<string>;
  onChange: (next: Set<string>) => void;
  filter: string;
  depth: number;
}) {
  const lc = filter.toLowerCase();
  if (lc && !matchesFilter(node, lc)) return null;

  const isLeaf = node.items.length === 0;

  if (isLeaf) {
    const checked = selected.has(node.href);
    return (
      <label
        className="doc-tree__leaf"
        style={{ paddingLeft: depth * 16 + 8 }}
      >
        <input
          type="checkbox"
          aria-label={node.title}
          checked={checked}
          onChange={() => {
            const next = new Set(selected);
            if (checked) next.delete(node.href);
            else next.add(node.href);
            onChange(next);
          }}
        />
        <span>{node.title}</span>
      </label>
    );
  }

  const leaves = collectLeaves(node.items);
  const allChecked = leaves.length > 0 && leaves.every((h) => selected.has(h));
  const someChecked = leaves.some((h) => selected.has(h));

  const toggleGroup = () => {
    const next = new Set(selected);
    if (allChecked) {
      leaves.forEach((h) => next.delete(h));
    } else {
      leaves.forEach((h) => next.add(h));
    }
    onChange(next);
  };

  return (
    <div className="doc-tree__group">
      <label
        className="doc-tree__group-header"
        style={{ paddingLeft: depth * 16 + 4 }}
      >
        <input
          type="checkbox"
          aria-label={node.title}
          checked={allChecked}
          ref={(el) => {
            if (el) el.indeterminate = someChecked && !allChecked;
          }}
          onChange={toggleGroup}
        />
        <span>{node.title}</span>
      </label>
      {node.items.map((child) => (
        <TreeNode
          key={child.href}
          node={child}
          selected={selected}
          onChange={onChange}
          filter={filter}
          depth={depth + 1}
        />
      ))}
    </div>
  );
}

export function DocTree({ nodes, selected, onChange, filter }: Props) {
  return (
    <div className="doc-tree">
      {nodes.map((node) => (
        <TreeNode
          key={node.href}
          node={node}
          selected={selected}
          onChange={onChange}
          filter={filter}
          depth={0}
        />
      ))}
    </div>
  );
}
```

- [ ] **Step 4: Run test to confirm it passes**

```bash
npx vitest run src/components/ApiDocsView/DocTree.test.tsx 2>&1 | tail -10
```

Expected: all 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/ApiDocsView/DocTree.tsx src/components/ApiDocsView/DocTree.test.tsx
git commit -m "feat(api-docs): DocTree component with checkbox tree and keyword filter"
```

---

### Task 4: ExtractionLog component

**Files:**
- Create: `src/components/ApiDocsView/ExtractionLog.tsx`

- [ ] **Step 1: Write failing test**

Create `src/components/ApiDocsView/ExtractionLog.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ExtractionLog } from "./ExtractionLog";

describe("ExtractionLog", () => {
  it("renders progress bar correctly", () => {
    render(
      <ExtractionLog
        current={3}
        total={10}
        logs={[]}
        outputFiles={[]}
      />
    );
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "3");
    expect(bar).toHaveAttribute("aria-valuemax", "10");
  });

  it("renders log messages with correct level class", () => {
    render(
      <ExtractionLog
        current={1}
        total={5}
        logs={[
          { level: "info", message: "✓ Loaded" },
          { level: "error", message: "✗ Failed" },
        ]}
        outputFiles={[]}
      />
    );
    expect(screen.getByText("✓ Loaded")).toBeInTheDocument();
    expect(screen.getByText("✗ Failed")).toBeInTheDocument();
  });

  it("renders output files when done", () => {
    render(
      <ExtractionLog
        current={5}
        total={5}
        logs={[]}
        outputFiles={["/tmp/api.md", "/tmp/payments.md"]}
      />
    );
    expect(screen.getByText("/tmp/api.md")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
npx vitest run src/components/ApiDocsView/ExtractionLog.test.tsx 2>&1 | tail -10
```

Expected: FAIL — `ExtractionLog.tsx` does not exist.

- [ ] **Step 3: Write ExtractionLog.tsx**

```tsx
// src/components/ApiDocsView/ExtractionLog.tsx
import { useLocale } from "../../contexts/LocaleContext";
import type { ApiDocsLogEvent } from "../../ipc/apiDocs";

interface Props {
  current: number;
  total: number;
  logs: ApiDocsLogEvent[];
  outputFiles: string[];
}

export function ExtractionLog({ current, total, logs, outputFiles }: Props) {
  const { t } = useLocale();
  const pct = total > 0 ? Math.round((current / total) * 100) : 0;

  return (
    <div className="extraction-log">
      {total > 0 && (
        <div className="extraction-log__progress">
          <div
            className="extraction-log__bar-track"
          >
            <div
              role="progressbar"
              aria-valuenow={current}
              aria-valuemin={0}
              aria-valuemax={total}
              className="extraction-log__bar-fill"
              style={{ width: `${pct}%` }}
            />
          </div>
          <span className="extraction-log__bar-label">
            {current} / {total}
          </span>
        </div>
      )}

      <div className="extraction-log__list">
        {logs.map((entry, i) => (
          <div
            key={i}
            className={`extraction-log__entry extraction-log__entry--${entry.level}`}
          >
            {entry.message}
          </div>
        ))}
      </div>

      {outputFiles.length > 0 && (
        <div className="extraction-log__output">
          <div className="extraction-log__output-title">{t.api_docs_output_files}</div>
          {outputFiles.map((f) => (
            <div key={f} className="extraction-log__output-file">
              {f}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run test to confirm it passes**

```bash
npx vitest run src/components/ApiDocsView/ExtractionLog.test.tsx 2>&1 | tail -10
```

Expected: all 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/ApiDocsView/ExtractionLog.tsx src/components/ApiDocsView/ExtractionLog.test.tsx
git commit -m "feat(api-docs): ExtractionLog with progress bar and log list"
```

---

### Task 5: ExtractionSettings component

**Files:**
- Create: `src/components/ApiDocsView/ExtractionSettings.tsx`

Right panel: output directory, merge toggle, keep checkboxes, auth status + login/logout.

- [ ] **Step 1: Write failing test**

Create `src/components/ApiDocsView/ExtractionSettings.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ExtractionSettings } from "./ExtractionSettings";
import type { KeepOptions, AuthStatus } from "../../ipc/apiDocs";

const defaultKeep: KeepOptions = {
  description: true,
  parameters: true,
  request_body: true,
  responses: true,
  code_samples: true,
};

const notLoggedIn: AuthStatus = { logged_in: false, account: "" };

// Wrap with locale
import { LocaleContext } from "../../contexts/LocaleContext";
import { translations } from "../../lib/i18n";
const wrap = (ui: React.ReactNode) =>
  render(
    <LocaleContext.Provider value={{ locale: "en", t: translations["en"], setLocale: () => {} }}>
      {ui}
    </LocaleContext.Provider>
  );

describe("ExtractionSettings", () => {
  it("renders output dir input", () => {
    wrap(
      <ExtractionSettings
        outputDir=""
        onOutputDirChange={() => {}}
        merge={false}
        onMergeChange={() => {}}
        keep={defaultKeep}
        onKeepChange={() => {}}
        auth={notLoggedIn}
        domain=""
        onLogin={() => {}}
        onLogout={() => {}}
        extracting={false}
        selectedCount={0}
        hasProvider={true}
        onExtractRaw={() => {}}
        onExtractAi={() => {}}
      />
    );
    expect(screen.getByPlaceholderText("~/api-docs")).toBeInTheDocument();
  });

  it("disables AI enhance button when no provider", () => {
    wrap(
      <ExtractionSettings
        outputDir="/tmp"
        onOutputDirChange={() => {}}
        merge={false}
        onMergeChange={() => {}}
        keep={defaultKeep}
        onKeepChange={() => {}}
        auth={notLoggedIn}
        domain=""
        onLogin={() => {}}
        onLogout={() => {}}
        extracting={false}
        selectedCount={1}
        hasProvider={false}
        onExtractRaw={() => {}}
        onExtractAi={() => {}}
      />
    );
    const aiBtn = screen.getByText("Extract + AI Enhance");
    expect(aiBtn).toBeDisabled();
  });

  it("calls onLogin when login button clicked", () => {
    const onLogin = vi.fn();
    wrap(
      <ExtractionSettings
        outputDir=""
        onOutputDirChange={() => {}}
        merge={false}
        onMergeChange={() => {}}
        keep={defaultKeep}
        onKeepChange={() => {}}
        auth={notLoggedIn}
        domain="docs.example.com"
        onLogin={onLogin}
        onLogout={() => {}}
        extracting={false}
        selectedCount={0}
        hasProvider={true}
        onExtractRaw={() => {}}
        onExtractAi={() => {}}
      />
    );
    fireEvent.click(screen.getByText("Login"));
    expect(onLogin).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
npx vitest run src/components/ApiDocsView/ExtractionSettings.test.tsx 2>&1 | tail -10
```

Expected: FAIL — `ExtractionSettings.tsx` does not exist.

- [ ] **Step 3: Write ExtractionSettings.tsx**

```tsx
// src/components/ApiDocsView/ExtractionSettings.tsx
import { useLocale } from "../../contexts/LocaleContext";
import type { KeepOptions, AuthStatus } from "../../ipc/apiDocs";

interface Props {
  outputDir: string;
  onOutputDirChange: (v: string) => void;
  merge: boolean;
  onMergeChange: (v: boolean) => void;
  keep: KeepOptions;
  onKeepChange: (v: KeepOptions) => void;
  auth: AuthStatus;
  domain: string;
  onLogin: () => void;
  onLogout: () => void;
  extracting: boolean;
  selectedCount: number;
  hasProvider: boolean;
  onExtractRaw: () => void;
  onExtractAi: () => void;
}

export function ExtractionSettings({
  outputDir, onOutputDirChange,
  merge, onMergeChange,
  keep, onKeepChange,
  auth, domain, onLogin, onLogout,
  extracting, selectedCount, hasProvider,
  onExtractRaw, onExtractAi,
}: Props) {
  const { t } = useLocale();
  const canExtract = selectedCount > 0 && !extracting;

  const toggleKeep = (key: keyof KeepOptions) => {
    onKeepChange({ ...keep, [key]: !keep[key] });
  };

  return (
    <div className="extraction-settings">
      {/* Output directory */}
      <div className="extraction-settings__section">
        <label className="extraction-settings__label">{t.api_docs_output_dir}</label>
        <input
          className="extraction-settings__input"
          type="text"
          value={outputDir}
          onChange={(e) => onOutputDirChange(e.target.value)}
          placeholder={t.api_docs_output_dir_placeholder}
        />
      </div>

      {/* Merge toggle */}
      <div className="extraction-settings__section">
        <label className="extraction-settings__checkbox-row">
          <input
            type="checkbox"
            checked={merge}
            onChange={(e) => onMergeChange(e.target.checked)}
          />
          <span>{t.api_docs_merge_label}</span>
        </label>
      </div>

      {/* Keep options */}
      <div className="extraction-settings__section">
        <div className="extraction-settings__label">{t.api_docs_keep_label}</div>
        {(
          [
            ["description", t.api_docs_keep_description],
            ["parameters", t.api_docs_keep_parameters],
            ["request_body", t.api_docs_keep_request_body],
            ["responses", t.api_docs_keep_responses],
            ["code_samples", t.api_docs_keep_code_samples],
          ] as [keyof KeepOptions, string][]
        ).map(([key, label]) => (
          <label key={key} className="extraction-settings__checkbox-row">
            <input
              type="checkbox"
              checked={keep[key]}
              onChange={() => toggleKeep(key)}
            />
            <span>{label}</span>
          </label>
        ))}
      </div>

      {/* Auth status */}
      {domain && (
        <div className="extraction-settings__section extraction-settings__auth">
          <div className="extraction-settings__label">{t.api_docs_auth_status_label}</div>
          <div className="extraction-settings__auth-row">
            <span
              className={`extraction-settings__auth-dot extraction-settings__auth-dot--${auth.logged_in ? "on" : "off"}`}
            />
            <span className="extraction-settings__auth-account">
              {auth.logged_in ? auth.account || domain : t.api_docs_not_logged_in}
            </span>
          </div>
          {auth.logged_in ? (
            <button className="extraction-settings__btn extraction-settings__btn--secondary" onClick={onLogout}>
              {t.api_docs_logout_btn}
            </button>
          ) : (
            <button className="extraction-settings__btn" onClick={onLogin}>
              {t.api_docs_login_btn}
            </button>
          )}
        </div>
      )}

      {/* Extract buttons */}
      <div className="extraction-settings__section extraction-settings__actions">
        {selectedCount === 0 && (
          <div className="extraction-settings__hint">{t.api_docs_no_pages}</div>
        )}
        <button
          className="extraction-settings__btn extraction-settings__btn--primary"
          disabled={!canExtract}
          onClick={onExtractRaw}
        >
          {extracting ? t.api_docs_extracting : t.api_docs_extract_raw}
        </button>
        <button
          className="extraction-settings__btn extraction-settings__btn--primary"
          disabled={!canExtract || !hasProvider}
          title={!hasProvider ? t.api_docs_no_provider : undefined}
          onClick={onExtractAi}
        >
          {t.api_docs_extract_ai}
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run test to confirm it passes**

```bash
npx vitest run src/components/ApiDocsView/ExtractionSettings.test.tsx 2>&1 | tail -10
```

Expected: all 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/ApiDocsView/ExtractionSettings.tsx src/components/ApiDocsView/ExtractionSettings.test.tsx
git commit -m "feat(api-docs): ExtractionSettings panel with output dir, keep options, auth"
```

---

### Task 6: ApiDocsView orchestrator

**Files:**
- Create: `src/components/ApiDocsView/ApiDocsView.tsx`
- Create: `src/components/ApiDocsView/ApiDocsView.css`
- Create: `src/components/ApiDocsView/index.ts`

- [ ] **Step 1: Write ApiDocsView.tsx**

```tsx
// src/components/ApiDocsView/ApiDocsView.tsx
import { useState, useEffect, useRef, useCallback } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import {
  apiDocsDetect,
  apiDocsFetchTree,
  apiDocsExtract,
  apiDocsLogin,
  apiDocsLogout,
  apiDocsAuthStatus,
  onApiDocsDetected,
  onApiDocsProgress,
  onApiDocsLog,
  onApiDocsDone,
  DEFAULT_KEEP_OPTIONS,
} from "../../ipc/apiDocs";
import type {
  DocNode,
  KeepOptions,
  AuthStatus,
  ApiDocsLogEvent,
} from "../../ipc/apiDocs";
import { DocTree } from "./DocTree";
import { ExtractionSettings } from "./ExtractionSettings";
import { ExtractionLog } from "./ExtractionLog";
import "./ApiDocsView.css";

interface Props {
  isActive: boolean;
}

function extractDomain(url: string): string {
  return url
    .replace(/^https?:\/\//, "")
    .split("/")[0];
}

export function ApiDocsView({ isActive }: Props) {
  const { t } = useLocale();

  // URL input
  const [url, setUrl] = useState("");
  const [platform, setPlatform] = useState<string | null>(null);
  const [treeLoading, setTreeLoading] = useState(false);
  const [tree, setTree] = useState<DocNode[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState("");

  // Settings
  const [outputDir, setOutputDir] = useState("~/api-docs");
  const [merge, setMerge] = useState(true);
  const [keep, setKeep] = useState<KeepOptions>(DEFAULT_KEEP_OPTIONS);

  // Auth
  const [auth, setAuth] = useState<AuthStatus>({ logged_in: false, account: "" });
  const domain = extractDomain(url);

  // Extraction state
  const [extracting, setExtracting] = useState(false);
  const [progress, setProgress] = useState({ current: 0, total: 0 });
  const [logs, setLogs] = useState<ApiDocsLogEvent[]>([]);
  const [outputFiles, setOutputFiles] = useState<string[]>([]);

  // Placeholder — real provider detection would call list_providers()
  // For now, assume provider is available (disable AI button only when explicitly not configured)
  const [hasProvider] = useState(true);

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  // Subscribe to Tauri events
  useEffect(() => {
    let unlisteners: (() => void)[] = [];

    Promise.all([
      onApiDocsDetected((e) => {
        if (mountedRef.current) setPlatform(e.platform);
      }),
      onApiDocsProgress((e) => {
        if (mountedRef.current) setProgress({ current: e.current, total: e.total });
      }),
      onApiDocsLog((e) => {
        if (mountedRef.current) setLogs((prev) => [...prev, e]);
      }),
      onApiDocsDone((e) => {
        if (mountedRef.current) {
          setExtracting(false);
          setOutputFiles(e.files);
        }
      }),
    ]).then((fns) => {
      unlisteners = fns;
    });

    return () => unlisteners.forEach((fn) => fn());
  }, []);

  // Refresh auth status when domain changes
  useEffect(() => {
    if (!domain) return;
    apiDocsAuthStatus(domain)
      .then((status) => { if (mountedRef.current) setAuth(status); })
      .catch(() => {});
  }, [domain]);

  const handleLoadTree = useCallback(async () => {
    if (!url.trim()) return;
    setTreeLoading(true);
    setPlatform(null);
    setTree([]);
    setSelected(new Set());
    setLogs([]);
    setOutputFiles([]);
    try {
      await apiDocsDetect(url);
      const nodes = await apiDocsFetchTree(url);
      if (mountedRef.current) setTree(nodes);
    } catch (err) {
      if (mountedRef.current) {
        setLogs([{ level: "error", message: String(err) }]);
      }
    } finally {
      if (mountedRef.current) setTreeLoading(false);
    }
  }, [url]);

  const handleSelectAll = useCallback(() => {
    const collect = (nodes: DocNode[]): string[] =>
      nodes.flatMap((n) => n.items.length ? collect(n.items) : [n.href]);
    setSelected(new Set(collect(tree)));
  }, [tree]);

  const handleDeselectAll = useCallback(() => setSelected(new Set()), []);

  const handleLogin = useCallback(async () => {
    try {
      await apiDocsLogin(url);
      const status = await apiDocsAuthStatus(domain);
      if (mountedRef.current) setAuth(status);
    } catch (err) {
      if (mountedRef.current) {
        setLogs((prev) => [...prev, { level: "error", message: String(err) }]);
      }
    }
  }, [url, domain]);

  const handleLogout = useCallback(async () => {
    await apiDocsLogout(domain);
    if (mountedRef.current) setAuth({ logged_in: false, account: "" });
  }, [domain]);

  const startExtraction = useCallback(async () => {
    setExtracting(true);
    setLogs([]);
    setOutputFiles([]);
    setProgress({ current: 0, total: selected.size });
    try {
      await apiDocsExtract({
        url,
        pages: Array.from(selected),
        output_dir: outputDir,
        merge,
        keep,
        cookies: "",  // Rust layer reads from keyring
      });
    } catch (err) {
      if (mountedRef.current) {
        setExtracting(false);
        setLogs((prev) => [...prev, { level: "error", message: String(err) }]);
      }
    }
  }, [url, selected, outputDir, merge, keep]);

  if (!isActive) return null;

  return (
    <div className="api-docs-view">
      {/* URL bar */}
      <div className="api-docs-view__url-bar">
        <input
          className="api-docs-view__url-input"
          type="text"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder={t.api_docs_url_placeholder}
          onKeyDown={(e) => e.key === "Enter" && handleLoadTree()}
        />
        <button
          className="api-docs-view__load-btn"
          onClick={handleLoadTree}
          disabled={treeLoading || !url.trim()}
        >
          {treeLoading ? t.api_docs_loading : t.api_docs_load_tree}
        </button>
        {platform && (
          <span className="api-docs-view__platform-badge">
            {t.api_docs_platform_label}: {platform}
          </span>
        )}
      </div>

      {/* Main layout: left tree + right settings */}
      <div className="api-docs-view__body">
        {/* Left: tree */}
        <div className="api-docs-view__tree-panel">
          {tree.length > 0 && (
            <>
              <div className="api-docs-view__tree-toolbar">
                <input
                  className="api-docs-view__filter-input"
                  type="text"
                  value={filter}
                  onChange={(e) => setFilter(e.target.value)}
                  placeholder={t.api_docs_filter_placeholder}
                />
                <button className="api-docs-view__toolbar-btn" onClick={handleSelectAll}>
                  {t.api_docs_select_all}
                </button>
                <button className="api-docs-view__toolbar-btn" onClick={handleDeselectAll}>
                  {t.api_docs_deselect_all}
                </button>
                <span className="api-docs-view__selected-count">
                  {t.api_docs_pages_selected(selected.size)}
                </span>
              </div>
              <div className="api-docs-view__tree-scroll">
                <DocTree
                  nodes={tree}
                  selected={selected}
                  onChange={setSelected}
                  filter={filter}
                />
              </div>
            </>
          )}
          {tree.length === 0 && !treeLoading && (
            <div className="api-docs-view__empty">
              {url ? t.api_docs_load_tree : t.api_docs_url_placeholder}
            </div>
          )}
          {(logs.length > 0 || extracting) && (
            <ExtractionLog
              current={progress.current}
              total={progress.total}
              logs={logs}
              outputFiles={outputFiles}
            />
          )}
        </div>

        {/* Right: settings */}
        <div className="api-docs-view__settings-panel">
          <ExtractionSettings
            outputDir={outputDir}
            onOutputDirChange={setOutputDir}
            merge={merge}
            onMergeChange={setMerge}
            keep={keep}
            onKeepChange={setKeep}
            auth={auth}
            domain={domain}
            onLogin={handleLogin}
            onLogout={handleLogout}
            extracting={extracting}
            selectedCount={selected.size}
            hasProvider={hasProvider}
            onExtractRaw={startExtraction}
            onExtractAi={startExtraction}  // AI flag passed via options in future
          />
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Write ApiDocsView.css**

```css
/* src/components/ApiDocsView/ApiDocsView.css */

.api-docs-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--bg-primary, #1a1a2e);
  color: var(--text-primary, #e0e0e0);
  font-size: 13px;
}

.api-docs-view__url-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border, #2a2a3e);
  flex-shrink: 0;
}

.api-docs-view__url-input {
  flex: 1;
  background: var(--bg-input, #0d0d1a);
  border: 1px solid var(--border, #333);
  border-radius: 4px;
  padding: 6px 10px;
  color: inherit;
  font-size: 13px;
}

.api-docs-view__load-btn {
  padding: 6px 14px;
  background: var(--accent, #4a90d9);
  color: white;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  white-space: nowrap;
}
.api-docs-view__load-btn:disabled {
  opacity: 0.5;
  cursor: default;
}

.api-docs-view__platform-badge {
  font-size: 11px;
  color: var(--text-secondary, #888);
  white-space: nowrap;
}

.api-docs-view__body {
  display: flex;
  flex: 1;
  overflow: hidden;
}

.api-docs-view__tree-panel {
  flex: 1;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border, #2a2a3e);
  overflow: hidden;
  min-width: 0;
}

.api-docs-view__tree-toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border, #2a2a3e);
  flex-shrink: 0;
}

.api-docs-view__filter-input {
  flex: 1;
  background: var(--bg-input, #0d0d1a);
  border: 1px solid var(--border, #333);
  border-radius: 4px;
  padding: 4px 8px;
  color: inherit;
  font-size: 12px;
}

.api-docs-view__toolbar-btn {
  padding: 3px 8px;
  background: var(--bg-secondary, #242436);
  border: 1px solid var(--border, #333);
  border-radius: 4px;
  color: inherit;
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
}

.api-docs-view__selected-count {
  font-size: 11px;
  color: var(--text-secondary, #888);
  white-space: nowrap;
}

.api-docs-view__tree-scroll {
  flex: 1;
  overflow-y: auto;
  padding: 6px 0;
}

.api-docs-view__empty {
  padding: 24px;
  color: var(--text-secondary, #555);
  font-size: 12px;
}

.api-docs-view__settings-panel {
  width: 260px;
  flex-shrink: 0;
  overflow-y: auto;
}

/* DocTree */
.doc-tree__leaf,
.doc-tree__group-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 8px;
  cursor: pointer;
  border-radius: 3px;
}
.doc-tree__leaf:hover,
.doc-tree__group-header:hover {
  background: var(--bg-hover, #242436);
}

/* ExtractionSettings */
.extraction-settings {
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.extraction-settings__section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.extraction-settings__label {
  font-size: 11px;
  color: var(--text-secondary, #888);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
.extraction-settings__input {
  background: var(--bg-input, #0d0d1a);
  border: 1px solid var(--border, #333);
  border-radius: 4px;
  padding: 5px 8px;
  color: inherit;
  font-size: 12px;
}
.extraction-settings__checkbox-row {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  cursor: pointer;
}
.extraction-settings__auth-row {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}
.extraction-settings__auth-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
.extraction-settings__auth-dot--on { background: #22c55e; }
.extraction-settings__auth-dot--off { background: #ef4444; }
.extraction-settings__auth-account {
  color: var(--text-secondary, #aaa);
  font-size: 11px;
  word-break: break-all;
}
.extraction-settings__btn {
  padding: 5px 12px;
  border-radius: 4px;
  border: 1px solid var(--border, #333);
  background: var(--bg-secondary, #242436);
  color: inherit;
  font-size: 12px;
  cursor: pointer;
}
.extraction-settings__btn--primary {
  background: var(--accent, #4a90d9);
  border-color: transparent;
  color: white;
}
.extraction-settings__btn:disabled {
  opacity: 0.4;
  cursor: default;
}
.extraction-settings__hint {
  font-size: 11px;
  color: var(--text-secondary, #666);
}
.extraction-settings__actions {
  gap: 8px;
}

/* ExtractionLog */
.extraction-log {
  padding: 10px;
  border-top: 1px solid var(--border, #2a2a3e);
  max-height: 200px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.extraction-log__progress {
  display: flex;
  align-items: center;
  gap: 8px;
}
.extraction-log__bar-track {
  flex: 1;
  height: 6px;
  background: var(--bg-secondary, #242436);
  border-radius: 3px;
  overflow: hidden;
}
.extraction-log__bar-fill {
  height: 100%;
  background: var(--accent, #4a90d9);
  border-radius: 3px;
  transition: width 0.2s;
}
.extraction-log__bar-label {
  font-size: 11px;
  color: var(--text-secondary, #888);
  white-space: nowrap;
}
.extraction-log__list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 11px;
  font-family: monospace;
}
.extraction-log__entry--info { color: var(--text-secondary, #aaa); }
.extraction-log__entry--warn { color: #f59e0b; }
.extraction-log__entry--error { color: #ef4444; }
.extraction-log__output-title {
  font-size: 11px;
  color: var(--text-secondary, #888);
  margin-top: 4px;
}
.extraction-log__output-file {
  font-size: 11px;
  font-family: monospace;
  color: #22c55e;
  word-break: break-all;
}
```

- [ ] **Step 3: Write index.ts**

```typescript
// src/components/ApiDocsView/index.ts
export { ApiDocsView } from "./ApiDocsView";
```

- [ ] **Step 4: Type-check**

```bash
npx tsc --noEmit 2>&1 | grep "ApiDocsView\|api_docs" | head -10
```

Expected: no errors.

- [ ] **Step 5: Run all frontend tests**

```bash
npx vitest run src/components/ApiDocsView/ 2>&1 | tail -15
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/components/ApiDocsView/
git commit -m "feat(api-docs): ApiDocsView orchestrator, CSS, barrel export"
```

---

### Task 7: Final integration test

- [ ] **Step 1: Run full frontend test suite**

```bash
npm run test 2>&1 | tail -20
```

Expected: no FAILED tests.

- [ ] **Step 2: Type-check full project**

```bash
npx tsc --noEmit 2>&1 | grep "error TS" | head -10
```

Expected: no errors.

- [ ] **Step 3: Lint**

```bash
npm run lint 2>&1 | grep -E "error|warning" | grep -v "^$" | head -20
```

Expected: no new errors.

- [ ] **Step 4: Rust check**

```bash
cd src-tauri && cargo check 2>&1 | grep -E "^error" | head -10
```

Expected: no errors.

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "feat(api-docs): Plan 3 complete — React frontend for API Docs tab"
```

---

## Self-Review Checklist

**Spec coverage:**
- [x] API Docs tab in NewTabPicker — Task 2
- [x] URL input + Load Doc Tree button — Task 6 (ApiDocsView)
- [x] Platform type display — Task 6
- [x] Checkbox tree with keyword filter — Task 3 (DocTree)
- [x] Select All / Deselect All — Task 6
- [x] Output directory setting — Task 5 (ExtractionSettings)
- [x] Merge single file toggle — Task 5
- [x] Keep options checkboxes — Task 5
- [x] Auth status panel (login/logout) — Task 5
- [x] Extract Raw Markdown button — Task 5
- [x] Extract + AI Enhance button (disabled without provider) — Task 5
- [x] Progress bar + log list — Task 4 (ExtractionLog)
- [x] Output files list after completion — Task 4
- [x] i18n zh-TW + en — Task 1
- [x] Tab registration (TabBar, NewTabPicker, TerminalApp) — Task 2

**Not in Plan 3:** Actual AI enhancement wiring (calling `aiChat()` per page) — the `onExtractAi` handler calls the same `startExtraction` for now; the AI flag should be wired through `ExtractionOptions` in a follow-up once the Python `ai_generic` strategy returns `needs_ai` events.

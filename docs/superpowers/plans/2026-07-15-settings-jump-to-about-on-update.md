# 有新版本時「設定」直接開啟關於頁 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 當 App 偵測到有新版本可用時，三個進入設定頁的入口（側邊欄收起狀態的齒輪圖示、側邊欄展開狀態的「設定」項目、`Ctrl+,` 快捷鍵）都直接開啟「關於 (About)」頁籤，而不是預設的「一般」頁籤；沒有新版本時行為不變。

**Architecture:** `SettingsView.tsx` 已支援透過 React Router 的 `location.state.tab` 決定初始頁籤（既有邏輯，本次不修改）。三個入口目前都呼叫不帶 state 的 `navigate("/settings")`；本次改動讓它們依 `hasUpdate` 布林值決定要不要帶入 `{ state: { tab: "about" } }`。

**Tech Stack:** React 19、react-router-dom（`MemoryRouter`）、Vitest + React Testing Library。

**參考 spec：** `docs/superpowers/specs/2026-07-15-settings-jump-to-about-on-update-design.md`

---

## Task 1: TabBar 兩個入口（收起齒輪圖示、展開「設定」項目）

**Files:**
- Modify: `src/components/TabBar/index.tsx:96-104`（收起狀態齒輪圖示）
- Modify: `src/components/TabBar/index.tsx:188-198`（展開狀態「設定」項目）
- Create: `src/components/TabBar/index.test.tsx`（此檔案目前不存在）

- [ ] **Step 1: 寫失敗測試**

建立 `src/components/TabBar/index.test.tsx`：

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

const navigateMock = vi.fn();
vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual<typeof import("react-router-dom")>("react-router-dom");
  return { ...actual, useNavigate: () => navigateMock };
});

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TabBar, type Tab, type TabBarProps } from "./index";

const baseTabs: Tab[] = [{ id: "t1", title: "Tab 1", type: "terminal" }];

function renderTabBar(overrides: Partial<TabBarProps> = {}) {
  return render(
    <LocaleProvider>
      <TabBar
        tabs={baseTabs}
        activeId="t1"
        onSelect={() => {}}
        onClose={() => {}}
        onAdd={() => {}}
        isSidebarOpen={false}
        onToggle={() => {}}
        width={260}
        {...overrides}
      />
    </LocaleProvider>,
  );
}

describe("TabBar settings navigation — collapsed gear icon", () => {
  beforeEach(() => {
    navigateMock.mockReset();
  });

  it("navigates to the about tab when hasUpdate is true", () => {
    renderTabBar({ isSidebarOpen: false, hasUpdate: true });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", { state: { tab: "about" } });
  });

  it("navigates to settings without a tab override when hasUpdate is false", () => {
    renderTabBar({ isSidebarOpen: false, hasUpdate: false });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", undefined);
  });
});

describe("TabBar settings navigation — expanded footer item", () => {
  beforeEach(() => {
    navigateMock.mockReset();
  });

  it("navigates to the about tab when hasUpdate is true", () => {
    renderTabBar({ isSidebarOpen: true, hasUpdate: true });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", { state: { tab: "about" } });
  });

  it("navigates to settings without a tab override when hasUpdate is false", () => {
    renderTabBar({ isSidebarOpen: true, hasUpdate: false });
    fireEvent.click(screen.getByTitle(/Ctrl\+,/));
    expect(navigateMock).toHaveBeenCalledWith("/settings", undefined);
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/components/TabBar/index.test.tsx`
Expected: 4 個測試中，`hasUpdate: true` 的兩個會 FAIL（因為目前的原始碼一律呼叫 `navigate("/settings")`，不帶第二個參數，`navigateMock` 收到的第二個參數是 `undefined` 而非 `{ state: { tab: "about" } }`）；`hasUpdate: false` 的兩個會 PASS（因為現況本來就是 `navigate("/settings")` 等同 `navigate("/settings", undefined)`）。

- [ ] **Step 3: 修改 TabBar 的兩個 onClick**

在 `src/components/TabBar/index.tsx`，目前收起狀態齒輪圖示（第 96-104 行）：

```tsx
        <button
          className="aiterm-sidebar-toggle"
          onClick={() => navigate("/settings")}
          title={`${t.settings} (Ctrl+,)`}
          style={{ background: 'transparent', border: 'none', color: '#888', cursor: 'pointer', fontSize: '18px', position: 'relative' }}
        >
          ⚙
          {hasUpdate && <span className="update-badge" aria-label="Update available" />}
        </button>
```

改為：

```tsx
        <button
          className="aiterm-sidebar-toggle"
          onClick={() => navigate("/settings", hasUpdate ? { state: { tab: "about" } } : undefined)}
          title={`${t.settings} (Ctrl+,)`}
          style={{ background: 'transparent', border: 'none', color: '#888', cursor: 'pointer', fontSize: '18px', position: 'relative' }}
        >
          ⚙
          {hasUpdate && <span className="update-badge" aria-label="Update available" />}
        </button>
```

目前展開狀態「設定」項目（第 188-198 行）：

```tsx
      {/* Footer Area with Settings */}
      <div className="aiterm-tabbar-footer">
        <div
          className="aiterm-tab"
          onClick={() => navigate("/settings")}
          title={`${t.settings} (Ctrl+,)`}
        >
          <span className="aiterm-tab-icon">⚙️</span>
          <span className="aiterm-tab-title">{t.settings}</span>
        </div>
      </div>
```

改為：

```tsx
      {/* Footer Area with Settings */}
      <div className="aiterm-tabbar-footer">
        <div
          className="aiterm-tab"
          onClick={() => navigate("/settings", hasUpdate ? { state: { tab: "about" } } : undefined)}
          title={`${t.settings} (Ctrl+,)`}
        >
          <span className="aiterm-tab-icon">⚙️</span>
          <span className="aiterm-tab-title">{t.settings}</span>
        </div>
      </div>
```

- [ ] **Step 4: 執行測試確認通過**

Run: `npx vitest run src/components/TabBar/index.test.tsx`
Expected: PASS (4 passed)

- [ ] **Step 5: Commit**

```bash
git add src/components/TabBar/index.tsx src/components/TabBar/index.test.tsx
git commit -m "feat(settings): open About tab from TabBar's settings entries when an update is available"
```

---

## Task 2: `Ctrl+,` 快捷鍵（`App.tsx`）

**Files:**
- Modify: `src/App.tsx:54-64`

此檔案目前沒有既有測試檔案，且 `AppRoutes` 元件會渲染完整的 `TerminalApp`（掛載 PTY session 等重度副作用），不適合在本次改動範圍內新建測試基礎設施（依 spec「不包含」段落的決定）。本任務僅手動驗證，無自動化測試步驟。

- [ ] **Step 1: 修改 `Ctrl+,` 快捷鍵處理函式**

在 `src/App.tsx`，目前（第 54-64 行）：

```tsx
  // Keyboard shortcut: Ctrl+, → settings
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === ",") {
        e.preventDefault();
        navigate("/settings");
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [navigate]);
```

改為：

```tsx
  // Keyboard shortcut: Ctrl+, → settings
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === ",") {
        e.preventDefault();
        const hasUpdate = updateInfo?.hasUpdate ?? false;
        navigate("/settings", hasUpdate ? { state: { tab: "about" } } : undefined);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [navigate, updateInfo]);
```

注意：`updateInfo` 是同一個 `AppRoutes` 元件裡既有的 state（第 21 行 `const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);`），此處新增為此 `useEffect` 的依賴項，確保 `updateInfo` 在版本檢查完成後更新時，快捷鍵處理函式能拿到最新值（避免閉包過期問題）。

- [ ] **Step 2: 型別檢查**

Run: `npx tsc -b tsconfig.app.json --force`
Expected: 無新增錯誤（這是本專案唯一會真的做型別檢查的指令；純 `npx tsc --noEmit` 在此 repo 是無效指令，因為 `tsconfig.json` 是只有 `references` 的 solution 檔）

- [ ] **Step 3: 手動驗證**

Run: `npm run tauri:dev`

1. 暫時把 `src/App.tsx` 第 44-46 行的 `if (latest !== cur)` 條件改成 `if (true)`（或直接把 `TAGS_API` 常數指向一個一定會回傳不同版本號的假資料），讓 `updateInfo.hasUpdate` 必定為 `true`，以便在本機環境下不用真的發布新版本就能測試。
2. 按 `Ctrl+,`，確認直接開啟「關於」頁籤（不是「一般」）。
3. 改回原本的版本比對邏輯（還原第 1 步的暫時性修改），確認 `git diff src/App.tsx` 除了 Step 1 的正式改動外沒有其他殘留。
4. 再次啟動、確認沒有新版本時按 `Ctrl+,` 開啟「一般」頁籤（現有行為維持不變）。

- [ ] **Step 4: Commit**

```bash
git add src/App.tsx
git commit -m "feat(settings): open About tab via Ctrl+, when an update is available"
```

---

## Task 3: 全域驗證

**Files:** 無（僅驗證）

- [ ] **Step 1: 執行完整前端測試**

Run: `npm run test -- --run`
Expected: 全數通過（含 Task 1 新增的 4 個測試）

- [ ] **Step 2: 型別檢查**

Run: `npx tsc -b tsconfig.app.json --force`
Expected: 無新增錯誤

- [ ] **Step 3: Lint**

Run: `npm run lint`
Expected: 無新增 lint 錯誤（`src/components/TabBar/index.tsx`、`src/components/TabBar/index.test.tsx`、`src/App.tsx` 這三個檔案不應出現新的 lint 問題；`npm run lint` 全域既有的 baseline 問題數量不應增加）

- [ ] **Step 4: 手動驗證三個入口的完整組合**

Run: `npm run tauri:dev`（若 Task 2 已在 dev server 開著的狀態下驗證過，可延續同一個 session）

依 Task 2 Step 3 的方式暫時讓 `hasUpdate` 為 `true`：

1. 收起側邊欄（點擊 `◨` 或 `Ctrl+B`），點擊齒輪圖示 → 確認開啟「關於」頁籤。
2. 從「關於」頁籤點擊左側「一般」切換回去，關閉設定頁（或用瀏覽器上一頁邏輯回到終端機），展開側邊欄，點擊「設定」文字項目 → 確認開啟「關於」頁籤。
3. 按 `Ctrl+,` → 確認開啟「關於」頁籤。
4. 還原暫時性修改，確認 `hasUpdate` 為 `false` 時，以上三種方式都改為開啟「一般」頁籤。

---

## Self-Review Notes

- **Spec coverage：** spec 的「範疇」段落列出的三個入口，Task 1（TabBar 兩處）與 Task 2（Ctrl+,）已全數涵蓋；「核心機制」段落的 `hasUpdate ? { state: { tab: "about" } } : undefined` 寫法在三處改動中完全一致；「錯誤處理」段落的 `?? false` 保底，Task 2 的 `updateInfo?.hasUpdate ?? false` 已對應；「測試」段落對 `TabBar` 的兩個入口 × 兩種 `hasUpdate` 狀態，Task 1 的 4 個測試案例已對應；對 `App.tsx` 快捷鍵「若無既有測試覆蓋則不強制新增」的決定，Task 2 已明確採用手動驗證。
- **Type consistency：** `TabBarProps`（`src/components/TabBar/index.tsx:32-46`）已匯出 `hasUpdate?: boolean`，Task 1 測試檔案的 `Partial<TabBarProps>` 用法與其一致；`navigate` 的第二參數型別（`{ state: { tab: "about" } } | undefined`）在 Task 1、Task 2 三處呼叫點完全一致，且與 spec 的「核心機制」段落程式碼片段一致。

# AITerm 首頁入口 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 AITerm 啟動時先停在一個首頁，集中呈現分頁入口、上次工作、AI 用量、進行中任務，並提供一個由 AI 判斷該開哪種分頁的自然語言輸入框。

**Architecture:** 首頁**不是分頁**，而是側邊欄上一顆固定按鈕；`TerminalApp` 用一個 `homeActive` 布林切換內容區。分頁一律沿用既有的 `visibility: hidden` 隱藏手法，**絕對不可改成卸載**（xterm.js 在無尺寸時 resize 會爆）。首頁四個區塊各自是獨立元件，資料來源優先重用既有 state（`agentProgress`、`aiSummary` 已經在 `tabs` 裡）。

**Tech Stack:** React 19 + TypeScript、Vitest + React Testing Library + jsdom、Tauri IPC（`src/ipc/*`）、i18n 走 `src/lib/i18n.ts` 的 `zhTW` / `enRaw` 兩份物件。

**Spec:** `docs/superpowers/specs/2026-08-16-home-entry-design.md`

---

## 動手前必讀

1. **不要卸載分頁。** `TerminalApp.tsx:434` 那段 `visibility` / `zIndex` / `pointerEvents` 的寫法是刻意的，程式碼裡就有註解。首頁只是多一個「都不 active」的狀態。
2. **`TerminalApp` 沒有測試 harness。** `src/App.test.tsx:6` 把它整支 mock 掉。任何值得測的邏輯都要放進獨立模組或子元件，不要埋進 `TerminalApp` 的 JSX。
3. **i18n 要改兩個地方。** `src/lib/i18n.ts` 的 `zhTW`（約第 9 行起）和 `enRaw`（約第 1220 行起）。只加一邊會讓型別檢查失敗。
4. **快捷鍵只看 `e.ctrlKey`**，三平台一致（`TerminalApp.tsx:334`），macOS 也是 Ctrl 不是 Cmd。
5. 每個 Task 結束都要 commit。驗證指令：`npx vitest run <檔案>`、`npx tsc -b`、`npx eslint <改過的檔案>`。**不要用 `tsc --noEmit`**，根 `tsconfig.json` 是 solution file，永遠回 0。

---

## File Structure

**新增：**

| 檔案 | 責任 |
|---|---|
| `src/components/NewTabPicker/tabCatalog.tsx` | 分頁類型的唯一清單（type / icon / label / desc / hidden），供 `NewTabPicker`、首頁大圖入口、AI 路由提示詞共用 |
| `src/components/HomeView/index.tsx` | 首頁容器，把四個區塊排版起來 |
| `src/components/HomeView/index.css` | 首頁樣式 |
| `src/components/HomeView/LaunchGrid.tsx` | 分頁大圖入口 |
| `src/components/HomeView/RunningTasks.tsx` | 進行中的 agent 任務 |
| `src/components/HomeView/UsageSection.tsx` | AI 用量與配額 |
| `src/components/HomeView/ResumeSection.tsx` | 上次工作：分頁卡片 + 最近專案目錄 |
| `src/components/HomeView/HomeInput.tsx` | 自然語言輸入框 |
| `src/components/HomeView/routeIntent.ts` | AI 路由的解析與防呆（純函式） |
| `src/lib/recentProjects.ts` | 最近專案目錄清單（localStorage，去重、上限 10 筆） |

**修改：**

| 檔案 | 改什麼 |
|---|---|
| `src/components/NewTabPicker/index.tsx` | 改用 `tabCatalog`，刪掉內嵌的清單 |
| `src/components/TabBar/index.tsx` | 加一顆固定的首頁按鈕（在 `.aiterm-tabbar-tabs` 之外） |
| `src/components/TerminalApp.tsx` | `homeActive` 狀態、`Ctrl+0`、內容區切換、`SavedTab` 擴充、`handlePickerSelect` 支援 `initialCwd`/`initialMission` |
| `src/components/TerminalView.tsx` | 新增 `onCwdChange` 回呼，把 cwd 往上報 |
| `src/lib/i18n.ts` | 新字串（zhTW + enRaw 兩邊） |

**分期：** Phase 1（Task 1-4）做完就是可用的切片——能看到首頁、能從首頁開分頁。Phase 2（5-6）加上免費就有的狀態資料。Phase 3（7-10）做持久化與上次工作。Phase 4（11-12）做 AI 輸入框。

---

# Phase 1：首頁的殼與入口

## Task 1：把分頁類型清單抽成單一來源

目前清單內嵌在 `NewTabPicker/index.tsx:50-66`。首頁大圖入口與 AI 路由提示詞都要用同一份，複製第二份必定分岔。

**Files:**
- Create: `src/components/NewTabPicker/tabCatalog.tsx`
- Create: `src/components/NewTabPicker/tabCatalog.test.tsx`
- Modify: `src/components/NewTabPicker/index.tsx:50-66`

- [ ] **Step 1: 寫會失敗的測試**

建立 `src/components/NewTabPicker/tabCatalog.test.tsx`：

```tsx
import { describe, expect, it } from "vitest";
import { getTabCatalog } from "./tabCatalog";
import { translations } from "../../lib/i18n";
import type { TabType } from "../TabBar";

const t = translations["zh-TW"];

// 這個測試存在的理由：新增一種 TabType 卻忘了在 catalog 補一筆時，首頁的大圖
// 入口和 AI 路由都會默默地少一個選項，而且不會有任何錯誤。
const ALL_TYPES: TabType[] = [
  "terminal", "database", "design", "cross-db", "vcs", "doc-converter",
  "api-docs", "loop-studio", "code-assistant", "knowledge-base", "mail",
];

describe("getTabCatalog", () => {
  it("涵蓋每一種 TabType", () => {
    const types = getTabCatalog(t).map((e) => e.type);
    for (const type of ALL_TYPES) {
      expect(types).toContain(type);
    }
  });

  it("每一筆都有非空的標題與說明", () => {
    for (const entry of getTabCatalog(t)) {
      expect(entry.label.length).toBeGreaterThan(0);
      expect(entry.desc.length).toBeGreaterThan(0);
    }
  });

  // mail 的後端是完整的，只是還沒對使用者開放；用 hidden 旗標記錄這件事，
  // 比在兩個地方各註解掉一行可靠。
  it("api-docs 與 mail 標成 hidden，其餘不是", () => {
    const catalog = getTabCatalog(t);
    expect(catalog.filter((e) => e.hidden).map((e) => e.type)).toEqual(["api-docs", "mail"]);
  });

  it("visibleTabCatalog 濾掉 hidden 的項目", () => {
    expect(visibleTabCatalog(t).some((e) => e.type === "mail")).toBe(false);
  });
});
```

在檔案頂端補上 `visibleTabCatalog` 的匯入：

```tsx
import { getTabCatalog, visibleTabCatalog } from "./tabCatalog";
```

- [ ] **Step 2: 執行測試確認它失敗**

Run: `npx vitest run src/components/NewTabPicker/tabCatalog.test.tsx`
Expected: FAIL — `Failed to resolve import "./tabCatalog"`

- [ ] **Step 3: 寫實作**

建立 `src/components/NewTabPicker/tabCatalog.tsx`：

```tsx
import type { Translations } from "../../lib/i18n";
import type { TabType } from "../TabBar";
import {
  TerminalIcon, DatabaseIcon, PaintbrushIcon, LinkIcon, BranchIcon,
  FileTextIcon, BookOpenIcon, RefreshIcon, CodeIcon, LibraryIcon, MailIcon,
} from "../Icons";

export interface TabCatalogEntry {
  type: TabType;
  icon: React.ReactNode;
  label: string;
  desc: string;
  /** 後端完整但尚未對使用者開放。隱藏的理由集中記在這裡，不要散在各個呼叫端。 */
  hidden?: boolean;
}

/** 分頁類型的唯一清單。NewTabPicker、首頁大圖入口、AI 路由提示詞都用這一份。 */
export function getTabCatalog(t: Translations): TabCatalogEntry[] {
  return [
    { type: "terminal",       icon: <TerminalIcon size={18} />,  label: t.terminal_tab,       desc: t.new_terminal_desc },
    { type: "database",       icon: <DatabaseIcon size={18} />,  label: t.database_tab,       desc: t.new_database_desc },
    { type: "design",         icon: <PaintbrushIcon size={18} />, label: t.design_tab,        desc: t.new_design_desc },
    { type: "cross-db",       icon: <LinkIcon size={18} />,      label: t.cross_db_tab,       desc: t.new_cross_db_desc },
    { type: "vcs",            icon: <BranchIcon size={18} />,    label: t.vcs_tab,            desc: t.new_vcs_desc },
    { type: "doc-converter",  icon: <FileTextIcon size={18} />,  label: t.doc_converter_tab,  desc: t.new_doc_converter_desc },
    { type: "api-docs",       icon: <BookOpenIcon size={18} />,  label: t.api_docs_tab,       desc: t.new_api_docs_desc, hidden: true },
    { type: "loop-studio",    icon: <RefreshIcon size={18} />,   label: t.loop_studio_tab,    desc: t.new_loop_studio_desc },
    { type: "code-assistant", icon: <CodeIcon size={18} />,      label: t.code_assistant_tab, desc: t.new_code_assistant_desc },
    { type: "knowledge-base", icon: <LibraryIcon size={18} />,   label: t.knowledge_base_tab, desc: t.new_knowledge_base_desc },
    { type: "mail",           icon: <MailIcon size={18} />,      label: t.mail_tab,           desc: t.new_mail_desc, hidden: true },
  ];
}

export function visibleTabCatalog(t: Translations): TabCatalogEntry[] {
  return getTabCatalog(t).filter((e) => !e.hidden);
}
```

**注意：** `api-docs` 原本不在 `NewTabPicker` 的清單裡，而且那是**刻意**的——commit `3547799 feat(tabs): hide the API Docs entry from the new-tab picker` 明確說「只拿掉入口，程式碼保留」。它和 `mail` 是同一種狀況，所以兩者都要 `hidden: true`。**這個 Task 是純重構，使用者在選單裡看到的項目不可以有任何變化。**

`t.new_api_docs_desc` 這個 i18n key 已經存在於 `zhTW` 與 `enRaw`（實測確認過），不需要新增。

- [ ] **Step 4: 執行測試確認它通過**

Run: `npx vitest run src/components/NewTabPicker/tabCatalog.test.tsx`
Expected: PASS（4 個測試）

- [ ] **Step 5: 讓 NewTabPicker 改用這份清單**

在 `src/components/NewTabPicker/index.tsx` 把第 50-66 行的 `items` 陣列整段換成：

```tsx
  const items = visibleTabCatalog(t);
```

並把頂端 `Icons` 的匯入縮減為只剩 `RobotIcon`（橋接那顆按鈕還在用），其餘圖示的匯入刪掉——它們現在由 `tabCatalog` 匯入。加上：

```tsx
import { visibleTabCatalog } from "./tabCatalog";
```

- [ ] **Step 6: 確認既有測試沒被弄壞**

Run: `npx vitest run src/components/NewTabPicker && npx tsc -b && npx eslint src/components/NewTabPicker`
Expected: 全部通過，eslint 無輸出

- [ ] **Step 7: Commit**

```bash
git add src/components/NewTabPicker src/lib/i18n.ts
git commit -m "refactor(tabs): 分頁類型清單抽成 tabCatalog 單一來源"
```

---

## Task 2：側邊欄的首頁按鈕

**Files:**
- Modify: `src/components/TabBar/index.tsx`
- Modify: `src/components/TabBar/index.css`
- Modify: `src/components/TabBar/index.test.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 寫會失敗的測試**

在 `src/components/TabBar/index.test.tsx` 的 `describe("TabBar 分頁拖曳排序"...)` 之前插入：

```tsx
describe("TabBar 首頁按鈕", () => {
  it("點首頁按鈕會呼叫 onHome", () => {
    const onHome = vi.fn();
    renderTabBar({ onHome, homeActive: false });
    fireEvent.click(screen.getByTitle(/Ctrl\+0/));
    expect(onHome).toHaveBeenCalledTimes(1);
  });

  it("首頁是目前畫面時，按鈕標成 active", () => {
    renderTabBar({ onHome: () => {}, homeActive: true });
    expect(screen.getByTitle(/Ctrl\+0/).className).toContain("active");
  });

  // 首頁不是分頁：它不能被拖曳排序、也不能佔用 Ctrl+1~9 的編號。
  it("首頁按鈕不在分頁清單裡", () => {
    const { container } = renderTabBar({ onHome: () => {}, homeActive: true });
    const inList = container.querySelectorAll(".aiterm-tabbar-tabs .aiterm-tab");
    expect(inList.length).toBe(1); // 只有 baseTabs 的那一個分頁
  });

  // 側邊欄收合成 48px 只剩圖示時，首頁按鈕必須還在。
  it("側邊欄收合時首頁按鈕仍在", () => {
    renderTabBar({ onHome: () => {}, homeActive: false, isSidebarOpen: false });
    expect(screen.getByTitle(/Ctrl\+0/)).toBeTruthy();
  });
});
```

- [ ] **Step 2: 執行測試確認它失敗**

Run: `npx vitest run src/components/TabBar/index.test.tsx -t "首頁按鈕"`
Expected: FAIL — `Unable to find an element with the title: /Ctrl\+0/`

- [ ] **Step 3: 寫實作**

在 `src/components/TabBar/index.tsx` 的 `TabBarProps` 加：

```tsx
  /** 使用者按了首頁。沒給就不顯示首頁按鈕。 */
  onHome?: () => void;
  /** 目前顯示的是不是首頁。 */
  homeActive?: boolean;
```

解構參數加上 `onHome,` 與 `homeActive = false,`。

匯入 `HomeIcon`（若 `src/components/Icons.tsx` 沒有這顆圖示，就在該檔比照既有圖示新增一個 18×18 的房子 SVG，並沿用相同的 `size` prop 慣例）。

在 `{/* Tabs list */}` 那個 `<div className="aiterm-tabbar-tabs">` **之前**插入：

```tsx
      {/* 首頁不是分頁：它固定在這裡，不進 .aiterm-tabbar-tabs，所以不會被
          拖曳排序、也不佔 Ctrl+1~9 的編號。 */}
      {onHome && (
        <button
          className={`aiterm-tab aiterm-home-button ${homeActive ? "active" : ""}`}
          onClick={onHome}
          title={`${t.home_tab} (Ctrl+0)`}
        >
          <span className="aiterm-tab-icon"><HomeIcon size={18} /></span>
          {isSidebarOpen && <span className="aiterm-tab-title">{t.home_tab}</span>}
        </button>
      )}
```

`src/components/TabBar/index.css` 加：

```css
/* 首頁按鈕沿用 .aiterm-tab 的外觀，但它是 <button>，要把瀏覽器預設樣式清掉。 */
.aiterm-home-button {
  border: 1px solid transparent;
  background: transparent;
  font: inherit;
  cursor: pointer;
  margin-bottom: 6px;
}
```

`src/lib/i18n.ts` 兩邊各加：

```ts
// zhTW
home_tab: "首頁",
// enRaw
home_tab: "Home",
```

- [ ] **Step 4: 執行測試確認它通過**

Run: `npx vitest run src/components/TabBar/index.test.tsx`
Expected: PASS（含既有 30 個 + 新增 4 個）

- [ ] **Step 5: Commit**

```bash
git add src/components/TabBar src/components/Icons.tsx src/lib/i18n.ts
git commit -m "feat(home): 側邊欄新增固定的首頁按鈕"
```

---

## Task 3：`homeActive` 狀態與 Ctrl+0

**Files:**
- Modify: `src/components/TerminalApp.tsx`

`TerminalApp` 沒有測試 harness，所以這個 Task 沒有自動化測試——它只是把狀態接起來。真正值得測的東西在 Task 4 的 `HomeView` 裡。**這是本計畫唯一一個沒有測試的 Task，不要把它當成慣例。**

- [ ] **Step 1: 加狀態**

在 `const [tabs, setTabs] = useState<Tab[]>(...)` 附近加：

```tsx
  // 首頁不是分頁，所以它不在 tabs 裡，而是一個「都不 active」的狀態。
  // 預設 true：開 app 先看到首頁，上次的分頁照常還原但不在前景。
  const [homeActive, setHomeActive] = useState(true);
```

- [ ] **Step 2: 選分頁時離開首頁**

修改 `selectTab`（`TerminalApp.tsx:149`），在 `setActiveId(id);` 之後加一行：

```tsx
    setHomeActive(false);
```

- [ ] **Step 3: 內容區切換**

修改 `TerminalApp.tsx:435` 的 `isActive`：

```tsx
          const isActive = tab.id === activeId && !homeActive;
```

在 `{tabs.map((tab) => {` 的**之前**插入首頁：

```tsx
        {/* 首頁蓋在同一塊內容區。分頁一律留在 DOM 裡（見下方 isActive 的註解），
            所以這裡不能改成三元運算把分頁換掉。 */}
        {homeActive && <HomeView onOpenTab={handlePickerSelect} tabs={tabs} />}
```

並在檔案頂端加：

```tsx
import { HomeView } from "./HomeView";
```

- [ ] **Step 4: 快捷鍵**

在 `handleKeyDown`（`TerminalApp.tsx:332`）的 `else if (e.key >= "1" && e.key <= "9")` **之前**插入：

```tsx
      } else if (e.key === "0") {
        // Windows/Linux 的 webview 用 Ctrl+0 重設縮放，一定要擋掉。
        // macOS 的重設縮放是 Cmd+0，不衝突。
        e.preventDefault();
        setHomeActive(true);
```

- [ ] **Step 5: 把狀態接到 TabBar**

在 `<TabBar ... />` 加兩個 prop：

```tsx
          onHome={() => setHomeActive(true)}
          homeActive={homeActive}
```

- [ ] **Step 6: 型別檢查**

Run: `npx tsc -b`
Expected: 失敗，因為 `./HomeView` 還不存在。這是預期的——Task 4 會補上。先不要 commit。

---

## Task 4：HomeView 殼與分頁大圖入口

**Files:**
- Create: `src/components/HomeView/index.tsx`
- Create: `src/components/HomeView/index.css`
- Create: `src/components/HomeView/LaunchGrid.tsx`
- Create: `src/components/HomeView/LaunchGrid.test.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 寫會失敗的測試**

建立 `src/components/HomeView/LaunchGrid.test.tsx`：

```tsx
import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { LaunchGrid } from "./LaunchGrid";

function renderGrid(onOpenTab = vi.fn()) {
  render(
    <LocaleProvider>
      <LaunchGrid onOpenTab={onOpenTab} />
    </LocaleProvider>,
  );
  return onOpenTab;
}

describe("LaunchGrid", () => {
  // LocaleProvider 預設 zh-TW，所以斷言 zh-TW 的字串。
  it("列出可見的分頁類型", () => {
    renderGrid();
    expect(screen.getByText("終端機")).toBeInTheDocument();
    expect(screen.getByText("資料庫")).toBeInTheDocument();
  });

  it("點某一項會用對應的 type 呼叫 onOpenTab", () => {
    const onOpenTab = renderGrid();
    fireEvent.click(screen.getByText("終端機"));
    expect(onOpenTab).toHaveBeenCalledWith("terminal");
  });

  // mail 的後端完整但尚未對使用者開放，首頁不能變成它的後門。
  it("不顯示 hidden 的分頁類型", () => {
    renderGrid();
    expect(screen.queryByText("信箱")).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 執行測試確認它失敗**

Run: `npx vitest run src/components/HomeView/LaunchGrid.test.tsx`
Expected: FAIL — `Failed to resolve import "./LaunchGrid"`

- [ ] **Step 3: 寫實作**

建立 `src/components/HomeView/LaunchGrid.tsx`：

```tsx
import { useLocale } from "../../contexts/LocaleContext";
import { visibleTabCatalog } from "../NewTabPicker/tabCatalog";
import type { TabType } from "../TabBar";

interface Props {
  onOpenTab: (type: TabType) => void;
}

export function LaunchGrid({ onOpenTab }: Props) {
  const { t } = useLocale();
  return (
    <section className="home-section">
      <h2 className="home-section-title">{t.home_launch_title}</h2>
      <div className="home-launch-grid">
        {visibleTabCatalog(t).map((entry) => (
          <button
            key={entry.type}
            className="home-launch-card"
            onClick={() => onOpenTab(entry.type)}
          >
            <span className="home-launch-icon">{entry.icon}</span>
            <span className="home-launch-label">{entry.label}</span>
            <span className="home-launch-desc">{entry.desc}</span>
          </button>
        ))}
      </div>
    </section>
  );
}
```

建立 `src/components/HomeView/index.tsx`：

```tsx
import { LaunchGrid } from "./LaunchGrid";
import type { Tab, TabType } from "../TabBar";
import "./index.css";

interface Props {
  onOpenTab: (type: TabType) => void;
  tabs: Tab[];
}

export function HomeView({ onOpenTab }: Props) {
  return (
    <div className="home-view">
      <LaunchGrid onOpenTab={onOpenTab} />
    </div>
  );
}
```

`tabs` 目前沒用到，但 Task 5 與 Task 10 會用。**保留這個 prop，不要為了消掉未使用警告而拿掉它再加回來。** 若 eslint 抱怨，在解構時暫時不取出即可（如上）。

建立 `src/components/HomeView/index.css`：

```css
.home-view {
  height: 100%;
  overflow-y: auto;
  padding: 32px 40px;
  box-sizing: border-box;
}

.home-section { margin-bottom: 32px; }

.home-section-title {
  font-size: 13px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted, #666);
  margin: 0 0 12px;
}

.home-launch-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
  gap: 12px;
}

.home-launch-card {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
  padding: 14px;
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.02);
  color: var(--text-primary, #eee);
  cursor: pointer;
  text-align: left;
  transition: var(--transition-smooth);
}

.home-launch-card:hover {
  border-color: var(--accent);
  background: var(--accent-dim, rgba(168, 85, 247, 0.06));
}

.home-launch-label { font-size: 14px; font-weight: 600; }
.home-launch-desc  { font-size: 12px; color: var(--text-secondary, #aaa); }
```

`src/lib/i18n.ts` 兩邊各加：

```ts
// zhTW
home_launch_title: "開始工作",
// enRaw
home_launch_title: "Start working",
```

- [ ] **Step 4: 執行測試確認它通過**

Run: `npx vitest run src/components/HomeView && npx tsc -b`
Expected: 測試 PASS，`tsc -b` 這次應該成功（Task 3 缺的匯入補上了）

- [ ] **Step 5: 全套驗證**

Run: `npm run test && npx tsc -b && npx eslint src/components/HomeView src/components/TerminalApp.tsx src/components/TabBar`
Expected: 測試全綠；eslint 只剩 `TerminalApp.tsx` 那個既有的 `set-state-in-effect`（`setLastTerminalPtyId`），不可以有新的錯誤

- [ ] **Step 6: Commit**

```bash
git add src/components/HomeView src/components/TerminalApp.tsx src/lib/i18n.ts
git commit -m "feat(home): 首頁殼與分頁大圖入口，啟動時停在首頁"
```

- [ ] **Step 7: 手動驗證（測試證明不了的部分）**

Run: `npm run tauri:dev`

確認：
1. 開 app 先看到首頁，側邊欄仍列著上次還原的分頁
2. 從首頁點「終端機」→ 開新分頁並離開首頁
3. 點側邊欄任一分頁 → 離開首頁；按首頁按鈕 → 回來
4. **在首頁與終端機分頁之間來回切換至少 5 次，終端機不會變空白或報錯**（xterm resize 地雷）
5. `Ctrl+0` 在終端機有焦點時仍能回首頁，且畫面沒有被縮放

---

# Phase 2：免費就有的狀態資料

## Task 5：進行中的任務

`agentProgress` 已經由 `TerminalView` 的 `onAgentProgress` 寫進 `tabs`（`TerminalApp.tsx:499`）。**不要沿用 `TerminalApp.tsx:522` 那個 `enterpriseTask` 過濾**——那是企業浮動面板的規則，首頁要顯示全部。

**Files:**
- Create: `src/components/HomeView/RunningTasks.tsx`
- Create: `src/components/HomeView/RunningTasks.test.tsx`
- Modify: `src/components/HomeView/index.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 寫會失敗的測試**

建立 `src/components/HomeView/RunningTasks.test.tsx`：

```tsx
import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { RunningTasks } from "./RunningTasks";
import type { Tab } from "../TabBar";

function renderTasks(tabs: Tab[], onSelectTab = vi.fn()) {
  render(
    <LocaleProvider>
      <RunningTasks tabs={tabs} onSelectTab={onSelectTab} />
    </LocaleProvider>,
  );
  return onSelectTab;
}

describe("RunningTasks", () => {
  it("沒有進行中的任務時整區不出現", () => {
    const { container } = render(
      <LocaleProvider>
        <RunningTasks tabs={[{ id: "t1", title: "Tab 1", type: "terminal" }]} onSelectTab={vi.fn()} />
      </LocaleProvider>,
    );
    expect(container.querySelector(".home-running-task")).toBeNull();
  });

  it("列出有 agentProgress 的分頁與其進度", () => {
    renderTasks([
      { id: "t1", title: "建置", type: "terminal", agentProgress: { done: 3, total: 8 } },
    ]);
    expect(screen.getByText("建置")).toBeInTheDocument();
    expect(screen.getByText("3 / 8")).toBeInTheDocument();
  });

  // 企業浮動面板只顯示 enterpriseTask 的任務，首頁不套那個過濾。
  it("一般 agent 任務也要顯示，不是只有企業任務", () => {
    renderTasks([
      { id: "t1", title: "一般任務", type: "terminal", agentProgress: { done: 1, total: 2 } },
    ]);
    expect(screen.getByText("一般任務")).toBeInTheDocument();
  });

  it("點某個任務會切到該分頁", () => {
    const onSelectTab = renderTasks([
      { id: "t9", title: "建置", type: "terminal", agentProgress: { done: 1, total: 2 } },
    ]);
    fireEvent.click(screen.getByText("建置"));
    expect(onSelectTab).toHaveBeenCalledWith("t9");
  });
});
```

- [ ] **Step 2: 執行測試確認它失敗**

Run: `npx vitest run src/components/HomeView/RunningTasks.test.tsx`
Expected: FAIL — `Failed to resolve import "./RunningTasks"`

- [ ] **Step 3: 寫實作**

建立 `src/components/HomeView/RunningTasks.tsx`：

```tsx
import { useLocale } from "../../contexts/LocaleContext";
import type { Tab } from "../TabBar";

interface Props {
  tabs: Tab[];
  onSelectTab: (id: string) => void;
}

export function RunningTasks({ tabs, onSelectTab }: Props) {
  const { t } = useLocale();
  // 刻意不過濾 enterpriseTask：那是企業浮動面板的規則，首頁要顯示所有任務。
  const running = tabs.filter((tab) => tab.agentProgress);
  if (running.length === 0) return null;

  return (
    <section className="home-section">
      <h2 className="home-section-title">{t.home_running_title}</h2>
      <div className="home-running-list">
        {running.map((tab) => {
          const { done, total } = tab.agentProgress!;
          const pct = Math.round((done / Math.max(total, 1)) * 100);
          return (
            <button
              key={tab.id}
              className="home-running-task"
              onClick={() => onSelectTab(tab.id)}
            >
              <span className="home-running-name">{tab.title}</span>
              <span className="home-running-count">{done} / {total}</span>
              <span className="home-running-bar">
                <span className="home-running-fill" style={{ width: `${pct}%` }} />
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
```

`src/components/HomeView/index.css` 加：

```css
.home-running-list { display: flex; flex-direction: column; gap: 8px; }

.home-running-task {
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 4px 12px;
  padding: 10px 14px;
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.02);
  color: var(--text-primary, #eee);
  cursor: pointer;
  text-align: left;
}

.home-running-name  { font-size: 13px; font-weight: 600; }
.home-running-count { font-size: 12px; color: var(--text-secondary, #aaa); }

.home-running-bar {
  grid-column: 1 / -1;
  height: 4px;
  border-radius: 2px;
  background: rgba(255, 255, 255, 0.08);
  overflow: hidden;
}

.home-running-fill { display: block; height: 100%; background: var(--accent); }
```

`src/lib/i18n.ts` 兩邊各加：

```ts
// zhTW
home_running_title: "進行中的任務",
// enRaw
home_running_title: "Running tasks",
```

- [ ] **Step 4: 執行測試確認它通過**

Run: `npx vitest run src/components/HomeView/RunningTasks.test.tsx`
Expected: PASS（4 個測試）

- [ ] **Step 5: 接進 HomeView**

`src/components/HomeView/index.tsx` 改成：

```tsx
import { LaunchGrid } from "./LaunchGrid";
import { RunningTasks } from "./RunningTasks";
import type { Tab, TabType } from "../TabBar";
import "./index.css";

interface Props {
  onOpenTab: (type: TabType) => void;
  onSelectTab: (id: string) => void;
  tabs: Tab[];
}

export function HomeView({ onOpenTab, onSelectTab, tabs }: Props) {
  return (
    <div className="home-view">
      <RunningTasks tabs={tabs} onSelectTab={onSelectTab} />
      <LaunchGrid onOpenTab={onOpenTab} />
    </div>
  );
}
```

`src/components/TerminalApp.tsx` 的 `<HomeView ... />` 補上：

```tsx
        {homeActive && (
          <HomeView onOpenTab={handlePickerSelect} onSelectTab={selectTab} tabs={tabs} />
        )}
```

- [ ] **Step 6: Commit**

```bash
git add src/components/HomeView src/components/TerminalApp.tsx src/lib/i18n.ts
git commit -m "feat(home): 首頁顯示進行中的 agent 任務"
```

---

## Task 6：AI 用量與配額

**Files:**
- Create: `src/components/HomeView/UsageSection.tsx`
- Create: `src/components/HomeView/UsageSection.test.tsx`
- Modify: `src/components/HomeView/index.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 寫會失敗的測試**

建立 `src/components/HomeView/UsageSection.test.tsx`：

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

const usageSummaryMock = vi.fn();
vi.mock("../../ipc/usage", () => ({
  usageSummary: (...args: unknown[]) => usageSummaryMock(...args),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { UsageSection } from "./UsageSection";

function renderUsage() {
  render(
    <LocaleProvider>
      <UsageSection />
    </LocaleProvider>,
  );
}

beforeEach(() => {
  usageSummaryMock.mockReset();
});

describe("UsageSection", () => {
  it("顯示今日各模型的請求數與 token 數", async () => {
    usageSummaryMock.mockResolvedValue([
      {
        provider_id: "anthropic", model: "claude-opus-5", requests: 12,
        prompt_tokens: 1000, completion_tokens: 500,
        cache_read_tokens: 0, cache_write_tokens: 0, estimated_cost_usd: 0.42,
      },
    ]);
    renderUsage();
    expect(await screen.findByText("claude-opus-5")).toBeInTheDocument();
    expect(screen.getByText("12")).toBeInTheDocument();
  });

  // 抓不到用量不能讓整個首頁掛掉，也不能顯示成 0（那是謊話）。
  it("查詢失敗時顯示無法取得，而不是 0", async () => {
    usageSummaryMock.mockRejectedValue(new Error("boom"));
    renderUsage();
    expect(await screen.findByText("無法取得用量資料")).toBeInTheDocument();
    expect(screen.queryByText("0")).not.toBeInTheDocument();
  });

  it("今天還沒用過時顯示空狀態", async () => {
    usageSummaryMock.mockResolvedValue([]);
    renderUsage();
    expect(await screen.findByText("今天還沒有用量")).toBeInTheDocument();
  });

  it("只查今天", async () => {
    usageSummaryMock.mockResolvedValue([]);
    renderUsage();
    await waitFor(() => expect(usageSummaryMock).toHaveBeenCalledWith("today"));
  });
});
```

- [ ] **Step 2: 執行測試確認它失敗**

Run: `npx vitest run src/components/HomeView/UsageSection.test.tsx`
Expected: FAIL — `Failed to resolve import "./UsageSection"`

- [ ] **Step 3: 寫實作**

建立 `src/components/HomeView/UsageSection.tsx`：

```tsx
import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { usageSummary, type UsageSummaryEntry } from "../../ipc/usage";

type State =
  | { kind: "loading" }
  | { kind: "ready"; entries: UsageSummaryEntry[] }
  | { kind: "failed" };

export function UsageSection() {
  const { t } = useLocale();
  const [state, setState] = useState<State>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    usageSummary("today")
      .then((entries) => { if (!cancelled) setState({ kind: "ready", entries }); })
      .catch(() => { if (!cancelled) setState({ kind: "failed" }); });
    return () => { cancelled = true; };
  }, []);

  return (
    <section className="home-section">
      <h2 className="home-section-title">{t.home_usage_title}</h2>
      {/* 查不到就說查不到。顯示 0 會讓使用者以為自己今天沒用過。 */}
      {state.kind === "failed" && <p className="home-empty">{t.home_usage_failed}</p>}
      {state.kind === "ready" && state.entries.length === 0 && (
        <p className="home-empty">{t.home_usage_empty}</p>
      )}
      {state.kind === "ready" && state.entries.length > 0 && (
        <table className="home-usage-table">
          <thead>
            <tr>
              <th>{t.home_usage_model}</th>
              <th>{t.home_usage_requests}</th>
              <th>{t.home_usage_tokens}</th>
            </tr>
          </thead>
          <tbody>
            {state.entries.map((e) => (
              <tr key={`${e.provider_id}/${e.model}`}>
                <td>{e.model}</td>
                <td>{e.requests}</td>
                <td>{(e.prompt_tokens + e.completion_tokens).toLocaleString()}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
```

`src/components/HomeView/index.css` 加：

```css
.home-empty { font-size: 13px; color: var(--text-muted, #666); margin: 0; }

.home-usage-table { width: 100%; border-collapse: collapse; font-size: 13px; }
.home-usage-table th {
  text-align: left;
  font-weight: 500;
  color: var(--text-muted, #666);
  padding: 4px 8px 8px 0;
}
.home-usage-table td { padding: 4px 8px 4px 0; color: var(--text-secondary, #aaa); }
```

`src/lib/i18n.ts` 兩邊各加：

```ts
// zhTW
home_usage_title: "今日 AI 用量",
home_usage_failed: "無法取得用量資料",
home_usage_empty: "今天還沒有用量",
home_usage_model: "模型",
home_usage_requests: "請求數",
home_usage_tokens: "Token",
// enRaw
home_usage_title: "AI usage today",
home_usage_failed: "Could not load usage data",
home_usage_empty: "No usage yet today",
home_usage_model: "Model",
home_usage_requests: "Requests",
home_usage_tokens: "Tokens",
```

- [ ] **Step 4: 執行測試確認它通過**

Run: `npx vitest run src/components/HomeView/UsageSection.test.tsx`
Expected: PASS（4 個測試）

- [ ] **Step 5: 接進 HomeView 並 commit**

在 `src/components/HomeView/index.tsx` 的 `<LaunchGrid ... />` 之後加 `<UsageSection />`，並匯入它。

```bash
npm run test && npx tsc -b
git add src/components/HomeView src/lib/i18n.ts
git commit -m "feat(home): 首頁顯示今日 AI 用量"
```

---

# Phase 3：持久化與上次工作

## Task 7：`handlePickerSelect` 支援起始目錄與任務

「最近專案 → 開終端機並 cd 過去」和 Task 12 的 AI 路由都需要建立分頁時帶參數。目前 `handlePickerSelect`（`TerminalApp.tsx:216`）只吃 `claudeBridge`。

**Files:**
- Modify: `src/components/TerminalApp.tsx:216-247`

- [ ] **Step 1: 擴充 opts**

把簽名改成：

```tsx
  const handlePickerSelect = useCallback((
    type: TabType,
    opts?: { claudeBridge?: boolean; initialCwd?: string; initialMission?: { goal: string; maxSteps: number } },
  ) => {
```

把最後建立分頁那一行（原 `TerminalApp.tsx:242`）改成：

```tsx
    setTabs((prev) => [...prev, {
      id: newId, title, type, claudeBridge,
      initialCwd: opts?.initialCwd,
      initialMission: opts?.initialMission,
    }]);
```

`Tab` 介面已經有 `initialCwd` 與 `initialMission` 欄位（`TabBar/index.tsx:34-35`），且 `TerminalView` 已經吃這兩個 prop（`TerminalApp.tsx:489-490`），所以不需要其他改動。

- [ ] **Step 2: 型別檢查**

Run: `npx tsc -b`
Expected: 通過

- [ ] **Step 3: Commit**

```bash
git add src/components/TerminalApp.tsx
git commit -m "feat(tabs): 建立分頁時可帶起始目錄與 agent 任務"
```

---

## Task 8：把 cwd 與 AI 摘要持久化

`aiSummary` 目前只在記憶體（重開就沒了），cwd 只有一個全域的 `aiterm_last_cwd`。

**Files:**
- Create: `src/lib/sessionTabs.ts`
- Create: `src/lib/sessionTabs.test.ts`
- Modify: `src/components/TerminalApp.tsx:36-56`
- Modify: `src/components/TerminalView.tsx:185-193`

- [ ] **Step 1: 寫會失敗的測試**

`restoreSessionTabs` / `saveSessionTabs` 目前是 `TerminalApp.tsx` 的模組層函式，而 `TerminalApp` 整支在測試裡被 mock，所以測不到。先把它們搬進 `src/lib/sessionTabs.ts`。

建立 `src/lib/sessionTabs.test.ts`：

```ts
import { describe, expect, it, beforeEach } from "vitest";
import { restoreSessionTabs, saveSessionTabs, SESSION_TABS_KEY } from "./sessionTabs";
import type { Tab } from "../components/TabBar";

beforeEach(() => {
  localStorage.clear();
});

describe("sessionTabs", () => {
  it("存下並還原 cwd 與 AI 摘要", () => {
    const tabs: Tab[] = [
      { id: "t1", title: "Tab 1", type: "terminal", cwd: "/repo/aiterm", aiSummary: "跑了建置" },
    ];
    saveSessionTabs(tabs);
    const restored = restoreSessionTabs()!;
    expect(restored[0].cwd).toBe("/repo/aiterm");
    expect(restored[0].aiSummary).toBe("跑了建置");
  });

  // 舊版存的資料沒有這兩個欄位。缺欄位不可以讓整份還原失敗——那會讓使用者
  // 的分頁全部消失。
  it("讀得懂沒有新欄位的舊格式", () => {
    localStorage.setItem(
      SESSION_TABS_KEY,
      JSON.stringify([{ title: "Old", type: "terminal" }]),
    );
    const restored = restoreSessionTabs()!;
    expect(restored).toHaveLength(1);
    expect(restored[0].title).toBe("Old");
    expect(restored[0].cwd).toBeUndefined();
  });

  it("每次還原都給新的 id", () => {
    saveSessionTabs([{ id: "old-id", title: "Tab 1", type: "terminal" }]);
    expect(restoreSessionTabs()![0].id).not.toBe("old-id");
  });

  it("內容壞掉時回 null 而不是丟例外", () => {
    localStorage.setItem(SESSION_TABS_KEY, "{ not json");
    expect(restoreSessionTabs()).toBeNull();
  });

  it("空陣列回 null（沒有可還原的東西）", () => {
    localStorage.setItem(SESSION_TABS_KEY, "[]");
    expect(restoreSessionTabs()).toBeNull();
  });
});
```

- [ ] **Step 2: 執行測試確認它失敗**

Run: `npx vitest run src/lib/sessionTabs.test.ts`
Expected: FAIL — `Failed to resolve import "./sessionTabs"`

- [ ] **Step 3: 寫實作**

先在 `src/components/TabBar/index.tsx` 的 `Tab` 介面補一個欄位（`initialCwd` 是「開新分頁時的起始目錄」，這裡要的是「目前實際所在的目錄」，兩者不同，不要合併）：

```tsx
  /** 這個終端機分頁目前實際所在的工作目錄。由 TerminalView 回報，會持久化。 */
  cwd?: string;
```

建立 `src/lib/sessionTabs.ts`：

```ts
import type { Tab } from "../components/TabBar";

export const SESSION_TABS_KEY = "aiterm-session-tabs";

type SavedTab = Pick<Tab, "title" | "type" | "dbConnectionId" | "cwd" | "aiSummary">;

export function restoreSessionTabs(): Tab[] | null {
  try {
    const raw = localStorage.getItem(SESSION_TABS_KEY);
    if (!raw) return null;
    const saved: SavedTab[] = JSON.parse(raw);
    if (!Array.isArray(saved) || saved.length === 0) return null;
    // 新欄位一律當成選填：缺了就是 undefined，不能讓整份還原失敗。
    return saved.map((s) => ({ ...s, id: crypto.randomUUID() }));
  } catch {
    return null;
  }
}

export function saveSessionTabs(tabs: Tab[]) {
  const toSave: SavedTab[] = tabs.map(
    ({ title, type, dbConnectionId, cwd, aiSummary }) => ({ title, type, dbConnectionId, cwd, aiSummary }),
  );
  localStorage.setItem(SESSION_TABS_KEY, JSON.stringify(toSave));
}
```

在 `src/components/TerminalApp.tsx` 刪掉第 36-56 行的 `SESSION_TABS_KEY`、`SavedTab`、`restoreSessionTabs`、`saveSessionTabs`，改成匯入：

```tsx
import { restoreSessionTabs, saveSessionTabs } from "../lib/sessionTabs";
```

- [ ] **Step 4: 執行測試確認它通過**

Run: `npx vitest run src/lib/sessionTabs.test.ts && npx tsc -b`
Expected: PASS（5 個測試）

- [ ] **Step 5: 讓 TerminalView 把 cwd 往上報**

在 `src/components/TerminalView.tsx` 的 props 介面加：

```tsx
  /** 工作目錄變了就回報一次。上層用它更新分頁狀態並記進最近專案。 */
  onCwdChange?: (cwd: string) => void;
```

在第 185-193 行那段輪詢裡，`lastCwdRef.current = cwd;` 之後加一行：

```tsx
          onCwdChange?.(cwd);
```

**不要改動 `aiterm_last_cwd` 既有的寫入**——它是新終端機分頁的起始目錄來源（`TerminalView.tsx:971`），與這裡是兩件事。

在 `src/components/TerminalApp.tsx` 的 `<TerminalView ... />` 加：

```tsx
                  onCwdChange={(cwd) => {
                    setTabs((prev) =>
                      prev.map((t) => t.id === tab.id ? { ...t, cwd } : t)
                    );
                  }}
```

- [ ] **Step 6: 全套驗證與 commit**

Run: `npm run test && npx tsc -b && npx eslint src/lib/sessionTabs.ts src/components/TerminalView.tsx src/components/TerminalApp.tsx`
Expected: 全綠；eslint 不可以有新錯誤

```bash
git add src/lib/sessionTabs.ts src/lib/sessionTabs.test.ts src/components/TerminalApp.tsx src/components/TerminalView.tsx src/components/TabBar/index.tsx
git commit -m "feat(home): 分頁的工作目錄與 AI 摘要持久化"
```

- [ ] **Step 7: 手動驗證**

Run: `npm run tauri:dev`

確認：在終端機 `cd` 到某個目錄、跑幾個指令讓 AI 摘要出現 → 關掉 app → 重開 → `localStorage` 的 `aiterm-session-tabs` 裡有 `cwd` 與 `aiSummary`。**同時確認 FileExplorer 的路徑同步沒有壞**（側邊的檔案樹要跟著 `cd` 移動）——這段邏輯很脆弱。

---

## Task 9：最近專案目錄清單

**Files:**
- Create: `src/lib/recentProjects.ts`
- Create: `src/lib/recentProjects.test.ts`
- Modify: `src/components/TerminalApp.tsx`

- [ ] **Step 1: 寫會失敗的測試**

建立 `src/lib/recentProjects.test.ts`：

```ts
import { describe, expect, it, beforeEach } from "vitest";
import { recordProject, listRecentProjects, RECENT_PROJECTS_KEY, MAX_RECENT_PROJECTS } from "./recentProjects";

beforeEach(() => {
  localStorage.clear();
});

describe("recentProjects", () => {
  it("記錄下來的目錄讀得回來", () => {
    recordProject("/repo/aiterm");
    expect(listRecentProjects().map((p) => p.path)).toEqual(["/repo/aiterm"]);
  });

  it("最近使用的排在最前面", () => {
    recordProject("/a");
    recordProject("/b");
    expect(listRecentProjects().map((p) => p.path)).toEqual(["/b", "/a"]);
  });

  // 同一個目錄反覆進出很常見，不去重的話清單會被同一筆塞滿。
  it("同一個目錄只留一筆，並移到最前面", () => {
    recordProject("/a");
    recordProject("/b");
    recordProject("/a");
    expect(listRecentProjects().map((p) => p.path)).toEqual(["/a", "/b"]);
  });

  it(`最多保留 ${MAX_RECENT_PROJECTS} 筆，超過就丟掉最舊的`, () => {
    for (let i = 0; i < MAX_RECENT_PROJECTS + 5; i++) recordProject(`/p${i}`);
    const list = listRecentProjects();
    expect(list).toHaveLength(MAX_RECENT_PROJECTS);
    expect(list[0].path).toBe(`/p${MAX_RECENT_PROJECTS + 4}`);
    expect(list.map((p) => p.path)).not.toContain("/p0");
  });

  it("內容壞掉時回空陣列而不是丟例外", () => {
    localStorage.setItem(RECENT_PROJECTS_KEY, "{ not json");
    expect(listRecentProjects()).toEqual([]);
  });

  it("空字串不記錄", () => {
    recordProject("");
    expect(listRecentProjects()).toEqual([]);
  });
});
```

- [ ] **Step 2: 執行測試確認它失敗**

Run: `npx vitest run src/lib/recentProjects.test.ts`
Expected: FAIL — `Failed to resolve import "./recentProjects"`

- [ ] **Step 3: 寫實作**

建立 `src/lib/recentProjects.ts`：

```ts
export const RECENT_PROJECTS_KEY = "aiterm-recent-projects";
export const MAX_RECENT_PROJECTS = 10;

export interface RecentProject {
  path: string;
  /** 最後一次進到這個目錄的時間（epoch ms）。 */
  lastUsedAt: number;
}

export function listRecentProjects(): RecentProject[] {
  try {
    const raw = localStorage.getItem(RECENT_PROJECTS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

export function recordProject(path: string) {
  if (!path) return;
  const rest = listRecentProjects().filter((p) => p.path !== path);
  const next = [{ path, lastUsedAt: Date.now() }, ...rest].slice(0, MAX_RECENT_PROJECTS);
  localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(next));
}
```

- [ ] **Step 4: 執行測試確認它通過**

Run: `npx vitest run src/lib/recentProjects.test.ts`
Expected: PASS（6 個測試）

- [ ] **Step 5: 在 cwd 變動時記錄**

在 `src/components/TerminalApp.tsx` 的 `onCwdChange` 回呼裡加一行（Task 8 建立的那段）：

```tsx
                  onCwdChange={(cwd) => {
                    setTabs((prev) =>
                      prev.map((t) => t.id === tab.id ? { ...t, cwd } : t)
                    );
                    recordProject(cwd);
                  }}
```

並匯入 `import { recordProject } from "../lib/recentProjects";`

- [ ] **Step 6: Commit**

```bash
npm run test && npx tsc -b
git add src/lib/recentProjects.ts src/lib/recentProjects.test.ts src/components/TerminalApp.tsx
git commit -m "feat(home): 記錄最近使用的專案目錄"
```

---

## Task 10：上次工作區塊

**Files:**
- Create: `src/components/HomeView/ResumeSection.tsx`
- Create: `src/components/HomeView/ResumeSection.test.tsx`
- Modify: `src/components/HomeView/index.tsx`
- Modify: `src/components/HomeView/index.css`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 寫會失敗的測試**

建立 `src/components/HomeView/ResumeSection.test.tsx`：

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { ResumeSection } from "./ResumeSection";
import { RECENT_PROJECTS_KEY } from "../../lib/recentProjects";
import type { Tab } from "../TabBar";

function renderResume(tabs: Tab[], onSelectTab = vi.fn(), onOpenProject = vi.fn()) {
  render(
    <LocaleProvider>
      <ResumeSection tabs={tabs} onSelectTab={onSelectTab} onOpenProject={onOpenProject} />
    </LocaleProvider>,
  );
  return { onSelectTab, onOpenProject };
}

beforeEach(() => {
  localStorage.clear();
});

describe("ResumeSection", () => {
  it("分頁卡片顯示工作目錄與 AI 摘要", () => {
    renderResume([
      { id: "t1", title: "Tab 1", type: "terminal", cwd: "/repo/aiterm", aiSummary: "跑了建置" },
    ]);
    expect(screen.getByText("/repo/aiterm")).toBeInTheDocument();
    expect(screen.getByText("跑了建置")).toBeInTheDocument();
  });

  // cwd 只對終端機分頁有意義，aiSummary 只有跑過指令的分頁才有。
  // 沒有的欄位就不要留空位。
  it("沒有 cwd 或摘要的分頁只顯示標題", () => {
    const { container } = renderResume([{ id: "t1", title: "資料庫", type: "database" }]);
    expect(screen.getByText("資料庫")).toBeInTheDocument();
    expect(container.querySelector(".home-resume-cwd")).toBeNull();
    expect(container.querySelector(".home-resume-summary")).toBeNull();
  });

  it("點分頁卡片會切到該分頁", () => {
    const { onSelectTab } = renderResume([{ id: "t7", title: "Tab 7", type: "terminal" }]);
    fireEvent.click(screen.getByText("Tab 7"));
    expect(onSelectTab).toHaveBeenCalledWith("t7");
  });

  it("列出最近的專案目錄", () => {
    localStorage.setItem(
      RECENT_PROJECTS_KEY,
      JSON.stringify([{ path: "/repo/aiterm", lastUsedAt: 1 }]),
    );
    renderResume([]);
    expect(screen.getByText("/repo/aiterm")).toBeInTheDocument();
  });

  it("點最近專案會用該路徑呼叫 onOpenProject", () => {
    localStorage.setItem(
      RECENT_PROJECTS_KEY,
      JSON.stringify([{ path: "/repo/aiterm", lastUsedAt: 1 }]),
    );
    const { onOpenProject } = renderResume([]);
    fireEvent.click(screen.getByText("/repo/aiterm"));
    expect(onOpenProject).toHaveBeenCalledWith("/repo/aiterm");
  });

  it("沒有分頁也沒有最近專案時顯示空狀態", () => {
    renderResume([]);
    expect(screen.getByText("還沒有可以接續的工作")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 執行測試確認它失敗**

Run: `npx vitest run src/components/HomeView/ResumeSection.test.tsx`
Expected: FAIL — `Failed to resolve import "./ResumeSection"`

- [ ] **Step 3: 寫實作**

建立 `src/components/HomeView/ResumeSection.tsx`：

```tsx
import { useLocale } from "../../contexts/LocaleContext";
import { listRecentProjects } from "../../lib/recentProjects";
import type { Tab } from "../TabBar";

interface Props {
  tabs: Tab[];
  onSelectTab: (id: string) => void;
  onOpenProject: (path: string) => void;
}

export function ResumeSection({ tabs, onSelectTab, onOpenProject }: Props) {
  const { t } = useLocale();
  // 首頁每次顯示都重讀：清單只在 cwd 變動時寫入，不需要訂閱機制。
  const projects = listRecentProjects();

  if (tabs.length === 0 && projects.length === 0) {
    return (
      <section className="home-section">
        <h2 className="home-section-title">{t.home_resume_title}</h2>
        <p className="home-empty">{t.home_resume_empty}</p>
      </section>
    );
  }

  return (
    <section className="home-section">
      <h2 className="home-section-title">{t.home_resume_title}</h2>

      <div className="home-resume-grid">
        {tabs.map((tab) => (
          <button key={tab.id} className="home-resume-card" onClick={() => onSelectTab(tab.id)}>
            <span className="home-resume-title">{tab.title}</span>
            {tab.cwd && <span className="home-resume-cwd">{tab.cwd}</span>}
            {tab.aiSummary && <span className="home-resume-summary">{tab.aiSummary}</span>}
          </button>
        ))}
      </div>

      {projects.length > 0 && (
        <>
          <h3 className="home-subsection-title">{t.home_recent_projects}</h3>
          <div className="home-recent-list">
            {projects.map((p) => (
              <button key={p.path} className="home-recent-item" onClick={() => onOpenProject(p.path)}>
                {p.path}
              </button>
            ))}
          </div>
        </>
      )}
    </section>
  );
}
```

`src/components/HomeView/index.css` 加：

```css
.home-resume-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 10px;
}

.home-resume-card {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 12px;
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.02);
  color: var(--text-primary, #eee);
  cursor: pointer;
  text-align: left;
}

.home-resume-title   { font-size: 13px; font-weight: 600; }
.home-resume-cwd     { font-size: 11px; font-family: ui-monospace, monospace; color: var(--text-muted, #666); overflow-wrap: anywhere; }
.home-resume-summary { font-size: 12px; color: var(--text-secondary, #aaa); }

.home-subsection-title {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-muted, #666);
  margin: 16px 0 8px;
}

.home-recent-list { display: flex; flex-direction: column; gap: 4px; }

.home-recent-item {
  padding: 6px 10px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary, #aaa);
  font: 12px/1.5 ui-monospace, monospace;
  text-align: left;
  cursor: pointer;
  overflow-wrap: anywhere;
}

.home-recent-item:hover { background: rgba(255, 255, 255, 0.05); color: var(--text-primary, #eee); }
```

`src/lib/i18n.ts` 兩邊各加：

```ts
// zhTW
home_resume_title: "接續上次的工作",
home_resume_empty: "還沒有可以接續的工作",
home_recent_projects: "最近的專案目錄",
// enRaw
home_resume_title: "Pick up where you left off",
home_resume_empty: "Nothing to resume yet",
home_recent_projects: "Recent projects",
```

- [ ] **Step 4: 執行測試確認它通過**

Run: `npx vitest run src/components/HomeView/ResumeSection.test.tsx`
Expected: PASS（6 個測試）

- [ ] **Step 5: 接進 HomeView**

`src/components/HomeView/index.tsx`：

```tsx
import { LaunchGrid } from "./LaunchGrid";
import { RunningTasks } from "./RunningTasks";
import { ResumeSection } from "./ResumeSection";
import { UsageSection } from "./UsageSection";
import type { Tab, TabType } from "../TabBar";
import "./index.css";

interface Props {
  onOpenTab: (type: TabType, opts?: { initialCwd?: string }) => void;
  onSelectTab: (id: string) => void;
  tabs: Tab[];
}

export function HomeView({ onOpenTab, onSelectTab, tabs }: Props) {
  return (
    <div className="home-view">
      <RunningTasks tabs={tabs} onSelectTab={onSelectTab} />
      <ResumeSection
        tabs={tabs}
        onSelectTab={onSelectTab}
        onOpenProject={(path) => onOpenTab("terminal", { initialCwd: path })}
      />
      <LaunchGrid onOpenTab={onOpenTab} />
      <UsageSection />
    </div>
  );
}
```

- [ ] **Step 6: 全套驗證與 commit**

```bash
npm run test && npx tsc -b && npx eslint src/components/HomeView
git add src/components/HomeView src/lib/i18n.ts
git commit -m "feat(home): 首頁顯示上次工作與最近專案目錄"
```

- [ ] **Step 7: 手動驗證**

Run: `npm run tauri:dev`

確認：從首頁點一個最近專案 → 開出來的終端機真的在那個目錄。

---

# Phase 4：AI 路由的輸入框

## Task 11：路由回應的解析與防呆

**Files:**
- Create: `src/components/HomeView/routeIntent.ts`
- Create: `src/components/HomeView/routeIntent.test.ts`

- [ ] **Step 1: 寫會失敗的測試**

建立 `src/components/HomeView/routeIntent.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { parseRouteReply, fallbackRoute } from "./routeIntent";

describe("fallbackRoute", () => {
  // 降級路徑：AI 掛掉時輸入框仍然要有反應。
  it("降級為終端機分頁並把整句話當成 agent 任務", () => {
    expect(fallbackRoute("幫我看為什麼 build 失敗")).toEqual({
      type: "terminal",
      mission: "幫我看為什麼 build 失敗",
      fallback: true,
    });
  });
});

describe("parseRouteReply", () => {
  const userText = "查一下訂單表有幾筆";

  it("讀得懂乾淨的 JSON", () => {
    expect(parseRouteReply('{"type":"database"}', userText)).toEqual({
      type: "database",
      fallback: false,
    });
  });

  // 模型很愛把 JSON 包在 markdown 圍籬裡。
  it("讀得懂包在 ```json 圍籬裡的回應", () => {
    const raw = "好的\n```json\n{\"type\":\"knowledge-base\"}\n```\n";
    expect(parseRouteReply(raw, userText).type).toBe("knowledge-base");
  });

  it("terminal 會帶上使用者原句當任務目標", () => {
    expect(parseRouteReply('{"type":"terminal"}', userText)).toEqual({
      type: "terminal",
      mission: userText,
      fallback: false,
    });
  });

  // 最重要的防呆：AI 回了清單外的東西，不可以照單全收去開一個不存在的分頁。
  it("清單外的分頁類型一律降級", () => {
    expect(parseRouteReply('{"type":"spreadsheet"}', userText)).toEqual(fallbackRoute(userText));
  });

  it("hidden 的分頁類型也要降級", () => {
    expect(parseRouteReply('{"type":"mail"}', userText)).toEqual(fallbackRoute(userText));
  });

  it("空回應降級", () => {
    expect(parseRouteReply("", userText)).toEqual(fallbackRoute(userText));
  });

  it("完全不是 JSON 的回應降級", () => {
    expect(parseRouteReply("我不確定你想做什麼", userText)).toEqual(fallbackRoute(userText));
  });

  it("是 JSON 但沒有 type 欄位，降級", () => {
    expect(parseRouteReply('{"reason":"不確定"}', userText)).toEqual(fallbackRoute(userText));
  });

  it("type 不是字串，降級", () => {
    expect(parseRouteReply('{"type":123}', userText)).toEqual(fallbackRoute(userText));
  });
});
```

- [ ] **Step 2: 執行測試確認它失敗**

Run: `npx vitest run src/components/HomeView/routeIntent.test.ts`
Expected: FAIL — `Failed to resolve import "./routeIntent"`

- [ ] **Step 3: 寫實作**

建立 `src/components/HomeView/routeIntent.ts`：

```ts
import type { Translations } from "../../lib/i18n";
import { visibleTabCatalog } from "../NewTabPicker/tabCatalog";
import type { TabType } from "../TabBar";

export interface RouteResult {
  type: TabType;
  /** 只有 terminal 會用到：把使用者原句當成 agent mission 的目標。 */
  mission?: string;
  /** true 代表這是降級結果，不是 AI 判斷出來的。UI 據此決定要不要顯示提示。 */
  fallback: boolean;
}

/**
 * 降級路徑：開終端機分頁，把整句話當成 agent 任務目標。
 *
 * AI 未設定、網路失敗、逾時、回了看不懂的東西——全部走這裡。輸入框永遠有反應。
 */
export function fallbackRoute(userText: string): RouteResult {
  return { type: "terminal", mission: userText, fallback: true };
}

/** 從可能夾雜文字或 markdown 圍籬的回應裡撈出第一個 JSON 物件。 */
function extractJson(raw: string): unknown {
  const start = raw.indexOf("{");
  const end = raw.lastIndexOf("}");
  if (start === -1 || end <= start) return null;
  try {
    return JSON.parse(raw.slice(start, end + 1));
  } catch {
    return null;
  }
}

/**
 * 解析 AI 的路由回應。
 *
 * **只接受可見清單裡的分頁類型。** 任何看不懂或超出清單的回應一律降級——照單
 * 全收去開一個不存在的分頁類型，會變成使用者無法理解的錯誤。
 */
export function parseRouteReply(raw: string, userText: string): RouteResult {
  const parsed = extractJson(raw);
  if (!parsed || typeof parsed !== "object") return fallbackRoute(userText);

  const type = (parsed as { type?: unknown }).type;
  if (typeof type !== "string") return fallbackRoute(userText);

  // 用型別清單而非 i18n 字串比對，所以這裡不需要真的 Translations 內容。
  const allowed = visibleTabCatalog({} as Translations).map((e) => e.type);
  if (!allowed.includes(type as TabType)) return fallbackRoute(userText);

  return type === "terminal"
    ? { type: "terminal", mission: userText, fallback: false }
    : { type: type as TabType, fallback: false };
}
```

**注意：** `visibleTabCatalog({} as Translations)` 會讓每一筆的 `label`/`desc` 是 `undefined`，但這裡只讀 `type` 與 `hidden`，所以安全。如果覺得這個轉型太髒，改法是在 `tabCatalog.tsx` 另外匯出一個不需要 `t` 的 `visibleTabTypes: TabType[]` 常數，並讓 `getTabCatalog` 從它推導——**兩種都可以，但不要在兩個地方各維護一份型別清單**。

- [ ] **Step 4: 執行測試確認它通過**

Run: `npx vitest run src/components/HomeView/routeIntent.test.ts`
Expected: PASS（10 個測試）

- [ ] **Step 5: Commit**

```bash
git add src/components/HomeView/routeIntent.ts src/components/HomeView/routeIntent.test.ts
git commit -m "feat(home): AI 路由回應的解析與防呆"
```

---

## Task 12：輸入框接上 AI 路由

**Files:**
- Create: `src/components/HomeView/HomeInput.tsx`
- Create: `src/components/HomeView/HomeInput.test.tsx`
- Modify: `src/components/HomeView/index.tsx`
- Modify: `src/components/HomeView/index.css`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 寫會失敗的測試**

建立 `src/components/HomeView/HomeInput.test.tsx`：

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const invokeAiChatMock = vi.fn();
vi.mock("../../ipc/ai", () => ({
  invokeAiChat: (...args: unknown[]) => invokeAiChatMock(...args),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { HomeInput } from "./HomeInput";

function renderInput(onRoute = vi.fn()) {
  render(
    <LocaleProvider>
      <HomeInput onRoute={onRoute} />
    </LocaleProvider>,
  );
  return onRoute;
}

beforeEach(() => {
  invokeAiChatMock.mockReset();
});

async function submit(text: string) {
  const box = screen.getByPlaceholderText(/想做什麼/);
  fireEvent.change(box, { target: { value: text } });
  fireEvent.keyDown(box, { key: "Enter" });
}

describe("HomeInput", () => {
  it("AI 判斷是資料庫就開資料庫分頁", async () => {
    invokeAiChatMock.mockResolvedValue({ content: '{"type":"database"}' });
    const onRoute = renderInput();
    await submit("查一下訂單表");
    await waitFor(() =>
      expect(onRoute).toHaveBeenCalledWith({ type: "database", fallback: false }),
    );
  });

  // 降級路徑：AI 掛掉不能讓輸入框變成死的。
  it("AI 失敗時降級為終端機 + agent 任務", async () => {
    invokeAiChatMock.mockRejectedValue({ kind: "not_configured" });
    const onRoute = renderInput();
    await submit("幫我修 build");
    await waitFor(() =>
      expect(onRoute).toHaveBeenCalledWith({
        type: "terminal", mission: "幫我修 build", fallback: true,
      }),
    );
  });

  it("空白輸入不送出", async () => {
    const onRoute = renderInput();
    await submit("   ");
    expect(invokeAiChatMock).not.toHaveBeenCalled();
    expect(onRoute).not.toHaveBeenCalled();
  });

  // 判斷中重複按 Enter 會開出好幾個分頁。
  it("判斷中不接受重複送出", async () => {
    invokeAiChatMock.mockImplementation(() => new Promise(() => {}));
    renderInput();
    await submit("做事");
    await submit("做事");
    expect(invokeAiChatMock).toHaveBeenCalledTimes(1);
  });
});
```

- [ ] **Step 2: 執行測試確認它失敗**

Run: `npx vitest run src/components/HomeView/HomeInput.test.tsx`
Expected: FAIL — `Failed to resolve import "./HomeInput"`

- [ ] **Step 3: 寫實作**

建立 `src/components/HomeView/HomeInput.tsx`：

```tsx
import { useRef, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { invokeAiChat } from "../../ipc/ai";
import { visibleTabCatalog } from "../NewTabPicker/tabCatalog";
import { parseRouteReply, fallbackRoute, type RouteResult } from "./routeIntent";

interface Props {
  onRoute: (result: RouteResult) => void;
}

export function HomeInput({ onRoute }: Props) {
  const { t, locale } = useLocale();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const sessionRef = useRef(crypto.randomUUID());

  const submit = async () => {
    const userText = text.trim();
    if (!userText || busy) return;
    setBusy(true);
    try {
      const catalog = visibleTabCatalog(t)
        .map((e) => `- ${e.type}: ${e.label} — ${e.desc}`)
        .join("\n");
      const prompt =
        `AITerm 有以下分頁類型：\n${catalog}\n\n` +
        `使用者說：「${userText}」\n\n` +
        `只回傳 JSON，格式為 {"type":"<上列其中一個 type>"}，不要加任何說明。`;

      const reply = await invokeAiChat(
        [{ role: "user", content: prompt }],
        sessionRef.current,
        undefined,
        false,
        locale,
      );
      onRoute(parseRouteReply(reply.content ?? "", userText));
    } catch {
      // AiError 的每一種 kind 對使用者的意義都一樣：AI 這條路走不通。
      // 統一降級，不要在這裡分流出四種訊息。
      onRoute(fallbackRoute(userText));
    } finally {
      setBusy(false);
      setText("");
    }
  };

  return (
    <section className="home-section">
      <input
        className="home-input"
        value={text}
        disabled={busy}
        placeholder={t.home_input_placeholder}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter") void submit(); }}
      />
    </section>
  );
}
```

**確認 `AiChatReply` 的欄位名稱**：`grep -n "interface AiChatReply" -A 8 src/ipc/ai.ts`。上面的 `reply.content` 是假設值，若實際欄位不同（例如 `message` 或 `text`），改成正確的那個，並同步改測試裡的 mock 回傳值。**兩邊要一致，不要只改一邊。**

`src/components/HomeView/index.css` 加：

```css
.home-input {
  width: 100%;
  padding: 14px 16px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.03);
  color: var(--text-primary, #eee);
  font-size: 14px;
  box-sizing: border-box;
}

.home-input:focus { outline: none; border-color: var(--accent); }
.home-input:disabled { opacity: 0.6; }
```

`src/lib/i18n.ts` 兩邊各加：

```ts
// zhTW
home_input_placeholder: "今天想做什麼？直接說，我幫你開對的地方",
// enRaw
home_input_placeholder: "What do you want to do? Just say it.",
```

- [ ] **Step 4: 執行測試確認它通過**

Run: `npx vitest run src/components/HomeView/HomeInput.test.tsx`
Expected: PASS（4 個測試）

- [ ] **Step 5: 接到 HomeView 與 TerminalApp**

`src/components/HomeView/index.tsx` 的 `Props.onOpenTab` 改成完整簽名，並把 `HomeInput` 放在最上面（它是首頁的主角）：

```tsx
interface Props {
  onOpenTab: (
    type: TabType,
    opts?: { initialCwd?: string; initialMission?: { goal: string; maxSteps: number } },
  ) => void;
  onSelectTab: (id: string) => void;
  tabs: Tab[];
}
```

在 `<div className="home-view">` 內最上面加：

```tsx
      <HomeInput
        onRoute={(r) =>
          onOpenTab(r.type, r.mission ? { initialMission: { goal: r.mission, maxSteps: 20 } } : undefined)
        }
      />
```

並匯入 `HomeInput`。

- [ ] **Step 6: 全套驗證與 commit**

```bash
npm run test && npx tsc -b && npx eslint src/components/HomeView
git add src/components/HomeView src/lib/i18n.ts
git commit -m "feat(home): 首頁自然語言輸入框，由 AI 判斷開哪種分頁"
```

- [ ] **Step 7: 手動驗證**

Run: `npm run tauri:dev`

確認：
1. 打「查一下資料庫有哪些表」→ 開資料庫分頁
2. 打「幫我看為什麼 build 失敗」→ 開終端機分頁並帶著任務跑
3. **把 AI 供應商設定成無效的 API key，再打一次** → 仍然開終端機分頁，不會卡住或無反應

---

## Task 13：猜錯要能反悔的提示

spec 的三個防呆之一。**沒有這個，AI 猜錯就變成使用者無法理解也無法補救的行為。**

提示只在 `fallback === false`（真的是 AI 判斷的）時出現。降級開出來的終端機會直接看到任務在跑，本身就說明了發生什麼事，不需要再加一條訊息。

**Files:**
- Create: `src/components/RouteHint.tsx`
- Create: `src/components/RouteHint.test.tsx`
- Create: `src/components/RouteHint.css`
- Modify: `src/components/TerminalApp.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 寫會失敗的測試**

建立 `src/components/RouteHint.test.tsx`：

```tsx
import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LocaleProvider } from "../contexts/LocaleContext";
import { RouteHint } from "./RouteHint";

function renderHint(onPick = vi.fn(), onDismiss = vi.fn()) {
  render(
    <LocaleProvider>
      <RouteHint pickedType="database" onPick={onPick} onDismiss={onDismiss} />
    </LocaleProvider>,
  );
  return { onPick, onDismiss };
}

describe("RouteHint", () => {
  // LocaleProvider 預設 zh-TW。
  it("說明 AI 判斷的是哪一種分頁", () => {
    renderHint();
    expect(screen.getByText(/AI 判斷你要的是「資料庫」分頁/)).toBeInTheDocument();
  });

  it("關掉提示會呼叫 onDismiss", () => {
    const { onDismiss } = renderHint();
    fireEvent.click(screen.getByRole("button", { name: "關閉提示" }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("選另一種分頁會用該 type 呼叫 onPick", () => {
    const { onPick } = renderHint();
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "terminal" } });
    expect(onPick).toHaveBeenCalledWith("terminal");
  });

  // 已經開的那一種不該出現在「換成」清單裡——選它等於什麼都沒做。
  it("換成清單不含目前這一種", () => {
    renderHint();
    const options = Array.from(
      screen.getByRole("combobox").querySelectorAll("option"),
    ).map((o) => (o as HTMLOptionElement).value);
    expect(options).not.toContain("database");
    expect(options).toContain("terminal");
  });
});
```

- [ ] **Step 2: 執行測試確認它失敗**

Run: `npx vitest run src/components/RouteHint.test.tsx`
Expected: FAIL — `Failed to resolve import "./RouteHint"`

- [ ] **Step 3: 寫實作**

建立 `src/components/RouteHint.tsx`：

```tsx
import { useLocale } from "../contexts/LocaleContext";
import { visibleTabCatalog } from "./NewTabPicker/tabCatalog";
import type { TabType } from "./TabBar";
import "./RouteHint.css";

interface Props {
  pickedType: TabType;
  onPick: (type: TabType) => void;
  onDismiss: () => void;
}

export function RouteHint({ pickedType, onPick, onDismiss }: Props) {
  const { t } = useLocale();
  const catalog = visibleTabCatalog(t);
  const picked = catalog.find((e) => e.type === pickedType);

  return (
    <div className="route-hint">
      <span className="route-hint-text">
        {t.home_route_hint(picked?.label ?? pickedType)}
      </span>
      {/* 猜錯的代價要小：換一種、或關掉，兩個動作都在同一條上。 */}
      <select
        className="route-hint-select"
        value=""
        onChange={(e) => onPick(e.target.value as TabType)}
      >
        <option value="" disabled>{t.home_route_switch}</option>
        {catalog
          .filter((e) => e.type !== pickedType)
          .map((e) => <option key={e.type} value={e.type}>{e.label}</option>)}
      </select>
      <button className="route-hint-close" onClick={onDismiss} aria-label={t.home_route_dismiss}>
        ✕
      </button>
    </div>
  );
}
```

建立 `src/components/RouteHint.css`：

```css
.route-hint {
  position: absolute;
  top: 8px;
  left: 50%;
  transform: translateX(-50%);
  z-index: 20;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border: 1px solid var(--accent);
  border-radius: 8px;
  background: var(--accent-dim, rgba(168, 85, 247, 0.12));
  backdrop-filter: blur(6px);
  font-size: 12px;
  color: var(--text-primary, #eee);
}

.route-hint-select {
  background: transparent;
  color: inherit;
  border: 1px solid rgba(255, 255, 255, 0.2);
  border-radius: 6px;
  padding: 2px 6px;
  font-size: 12px;
}

.route-hint-close {
  background: none;
  border: none;
  color: inherit;
  cursor: pointer;
  opacity: 0.7;
  font-size: 12px;
}
```

`src/lib/i18n.ts` 兩邊各加：

```ts
// zhTW
home_route_hint: (label: string) => `AI 判斷你要的是「${label}」分頁`,
home_route_switch: "換成…",
home_route_dismiss: "關閉提示",
// enRaw
home_route_hint: (label: string) => `AI picked the "${label}" tab`,
home_route_switch: "Switch to…",
home_route_dismiss: "Dismiss",
```

- [ ] **Step 4: 執行測試確認它通過**

Run: `npx vitest run src/components/RouteHint.test.tsx`
Expected: PASS（4 個測試）

- [ ] **Step 5: 接進 TerminalApp**

在 `TerminalApp` 加狀態：

```tsx
  // AI 路由開出來的分頁要能被反悔。記住是哪一個分頁、AI 選了什麼、當初那句話，
  // 換一種類型時要用同一句話重開。
  const [routeHint, setRouteHint] = useState<
    { tabId: string; type: TabType; userText: string } | null
  >(null);
```

`handlePickerSelect` 目前不回傳新分頁的 id，改成回傳它（`return newId;`，並把 `useCallback` 的型別隨之更新為回傳 `string`）。

`<HomeView ... />` 的 `onOpenTab` 改成一個包裝函式：

```tsx
        {homeActive && (
          <HomeView
            onOpenTab={handlePickerSelect}
            onSelectTab={selectTab}
            onRouted={(tabId, type, userText) => setRouteHint({ tabId, type, userText })}
            tabs={tabs}
          />
        )}
```

`HomeView` 的 `HomeInput` 回呼改成：

```tsx
      <HomeInput
        onRoute={(r) => {
          const tabId = onOpenTab(
            r.type,
            r.mission ? { initialMission: { goal: r.mission, maxSteps: 20 } } : undefined,
          );
          // 降級開出來的分頁不顯示提示：終端機裡的任務本身就說明了發生什麼事。
          if (!r.fallback) onRouted(tabId, r.type, r.userText);
        }}
      />
```

這需要 `RouteResult` 多帶原句。在 `src/components/HomeView/routeIntent.ts` 的 `RouteResult` 加一個欄位並在兩個建構點都填上：

```ts
  /** 使用者原本打的那句話。換一種分頁類型時要用同一句話重開。 */
  userText: string;
```

`fallbackRoute` 回傳 `{ type: "terminal", mission: userText, userText, fallback: true }`；`parseRouteReply` 的兩個回傳分支同樣補上 `userText`。**Task 11 的測試要一併更新斷言**，否則會紅。

在 `TerminalApp` 的內容區（`<div style={{ flex: 1, position: "relative", minWidth: 0 }}>` 之內、`tabs.map` 之前）加：

```tsx
        {routeHint && !homeActive && routeHint.tabId === activeId && (
          <RouteHint
            pickedType={routeHint.type}
            onDismiss={() => setRouteHint(null)}
            onPick={(type) => {
              void handleCloseTab(routeHint.tabId);
              const tabId = handlePickerSelect(
                type,
                type === "terminal"
                  ? { initialMission: { goal: routeHint.userText, maxSteps: 20 } }
                  : undefined,
              );
              setRouteHint({ tabId, type, userText: routeHint.userText });
            }}
          />
        )}
```

- [ ] **Step 6: 全套驗證與 commit**

Run: `npm run test && npx tsc -b && npx eslint src/components/RouteHint.tsx src/components/HomeView src/components/TerminalApp.tsx`
Expected: 全綠（含 Task 11 更新後的測試）；eslint 無新錯誤

```bash
git add src/components/RouteHint.tsx src/components/RouteHint.test.tsx src/components/RouteHint.css src/components/HomeView src/components/TerminalApp.tsx src/lib/i18n.ts
git commit -m "feat(home): AI 猜錯分頁類型時可換一種或關掉提示"
```

- [ ] **Step 7: 手動驗證**

Run: `npm run tauri:dev`

確認：從首頁打一句話 → 開出分頁後上方出現提示 → 用「換成…」挑另一種 → 原分頁關掉、新類型開起來且提示跟著更新 → 按 ✕ 提示消失且不再出現。

---

## 最後的整體驗證

- [ ] `npm run test` 全綠
- [ ] `npx tsc -b` 通過
- [ ] `npx eslint src` 沒有**新增**的錯誤（既有的 94 個問題不在此次範圍）
- [ ] `npm run tauri:dev` 手動確認：
  - 開 app 停在首頁，側邊欄列著還原的分頁
  - 首頁 ↔ 終端機分頁來回切換 5 次以上，xterm 不會爆
  - 側邊欄收合成 48px 時首頁按鈕仍可點
  - `Ctrl+0` 在終端機有焦點時可回首頁，畫面沒被縮放
  - FileExplorer 的路徑同步仍然正常
  - AI 路由開出的分頁上有提示，可以換一種類型、也可以關掉

---

## 這份計畫刻意不做的事

- **不做首頁自訂版面**（區塊順序、顯示與否）——YAGNI，先用了再說
- **不檢查最近專案目錄是否還存在**——首頁載入時逐一打檔案系統會拖慢啟動；點下去才提示找不到
- **不做「開分頁前先確認」的兩步流程**——那等於退回沒有 AI 的版本。AI 路由靠的是防呆與可反悔，不是準確率
- **不碰 `aiterm_last_cwd` 既有的語意**——它是新分頁的起始目錄來源，與新增的每分頁 cwd 是兩件事

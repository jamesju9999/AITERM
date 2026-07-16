# 統一按鈕視覺系統 — 設計規格

**日期：** 2026-07-16
**狀態：** 已核准，待實作

---

## 目標

App 目前沒有共用的 Button 元件或 CSS class，每個面板都各自刻一套按鈕樣式。全 app 掃描後統計出約 **40 種不同的按鈕樣式定義**，分散在 15+ 個檔案（CSS class 與 `.tsx` inline `style={{}}` 混雜），造成同語意角色的按鈕（例如「主要動作」「刪除」「目前選中」）在不同面板呈現不同圓角、有無 hover 效果、甚至不同強調色。

起因：API Docs 面板的按鈕被回報「跟其他功能樣式不一致」而修正後，使用者要求評估並統一「相關的按鈕」——評估後發現此問題是全 app 性的，不只 API Docs 一處。

本次目標：建立一套共用 CSS 樣式系統（`src/styles/buttons.css`），並將全部 5 大類別（主要動作／次要／危險／圖示／選中狀態）遷移過去，消除視覺不一致與程式碼重複。

---

## 範疇

**包含：**
- 新建 `src/styles/buttons.css`，定義共用 base class + 5 種 variant + 尺寸修飾。
- 遷移以下 5 大類別、涉及的所有檔案（完整清單見下方「各類別改動清單」）：
  1. 主要動作（CTA）按鈕 — ~12 處
  2. 次要／外框按鈕 — ~20 處
  3. 危險／刪除按鈕 — ~11 處
  4. 圖示／圓形按鈕 — ~10 處
  5. 選中／狀態語彙（tab-active、toggle-on）— ~7 處
- 已知重複貼上的 inline style（`DatabaseConnectionsPage.tsx` / `VcsConnectionsPage.tsx` 的 `btnStyle`）改為共用 class，順便消除重複定義。

**不包含：**
- 不新增 React `<Button>` 元件——維持現有「HTML 原生 `<button>` / `<div onClick>` + className」寫法，只換 className 所指向的樣式來源，避免同時變更元件 API 與樣式造成的雙重風險。
- 不變更任何按鈕的**行為邏輯**（onClick handler、disabled 條件、確認流程等完全不動，只動樣式）。
- 不變更文案／i18n 字串。
- 不處理 CommandPreview 的 risk-level 三態配色（safe/needs_confirm/dangerous）——這是專屬的風險提示語彙，不屬於「按鈕種類」而是「風險語意」，混入統一系統會混淆兩種不同的視覺溝通目的，故保留現狀。
- 不變更 API Docs 面板（`ApiDocsView.css`）——已在前一輪修正完成，符合本次的 primary/secondary 標準，僅需確認其 class 命名未來可視情況接軌，但本次不強制重構。

---

## 核心機制：共用樣式系統

### 新檔案 `src/styles/buttons.css`

```css
.aiterm-btn {
  cursor: pointer;
  font-family: inherit;
  border: 1px solid transparent;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  transition: var(--transition-smooth, all 0.25s cubic-bezier(0.4, 0, 0.2, 1));
}
.aiterm-btn:disabled {
  opacity: 0.4;
  cursor: default;
}

/* 主要動作 */
.aiterm-btn--primary {
  background: var(--accent-gradient, linear-gradient(135deg, var(--accent, #a855f7), #6366f1));
  border-color: transparent;
  border-radius: 8px;
  color: #fff;
  font-weight: 600;
  padding: 8px 16px;
}
.aiterm-btn--primary:hover:not(:disabled) {
  transform: translateY(-1px);
  box-shadow: 0 6px 16px var(--accent-glow, rgba(168, 85, 247, 0.35));
}
.aiterm-btn--primary:disabled {
  transform: none;
  box-shadow: none;
}

/* 次要／外框 */
.aiterm-btn--secondary {
  background: var(--bg-secondary, #1a1a2e);
  border-color: var(--border, #333);
  border-radius: 6px;
  color: inherit;
  padding: 6px 12px;
}
.aiterm-btn--secondary:hover:not(:disabled) {
  background: var(--bg-hover, #252540);
}

/* 危險／刪除 */
.aiterm-btn--danger {
  background: rgba(239, 68, 68, 0.12);
  border-color: #ef4444;
  border-radius: 8px;
  color: #ef4444;
  font-weight: 600;
  padding: 8px 16px;
}
.aiterm-btn--danger:hover:not(:disabled) {
  background: #ef4444;
  color: #fff;
}

/* 純圖示／文字，無底無框 */
.aiterm-btn--ghost {
  background: transparent;
  border-color: transparent;
  border-radius: 6px;
  color: var(--text-secondary, #888);
  padding: 4px 8px;
}
.aiterm-btn--ghost:hover:not(:disabled) {
  background: var(--bg-hover, rgba(255, 255, 255, 0.06));
  color: var(--text-primary, #fff);
}

/* 圓形／方形圖示按鈕，疊加 --primary 或 --danger 決定顏色 */
.aiterm-btn--icon {
  width: 28px;
  height: 28px;
  padding: 0;
  border-radius: 50%;
  flex-shrink: 0;
}

/* 尺寸修飾：密集列表（Settings 卡片操作列等） */
.aiterm-btn--sm {
  padding: 3px 8px;
  font-size: 11px;
  border-radius: 4px;
}
```

各 variant 沿用 App 既有的全域 token（`--accent-gradient`、`--accent-glow`、`--transition-smooth`，定義於 `src/App.css`），確保跟隨使用者選擇的主題色連動（目前預設主題為綠色 `#34d399`）。

### 選中／狀態語彙（獨立於按鈕系統，但一併統一）

依先前確認的原則，區分兩種語意：

- **狀態型**（開關／連線是否成立）：`.aiterm-agent-toggle--on` 等既有 class 維持固定綠色（`#22c55e` 系），不隨主題色改變，因為它代表「功能正在運作中」這種通用狀態語言，不是選中語言。
- **選中型**（nav／tab／按鈕群目前選中哪一項）：統一改用 `var(--accent)` / `var(--accent-dim)`，取代目前混用的綠（`.mcp-tab-btn.active`）／紫（`.sidebar-item--active`）／藍（`.aiterm-subtab--active`、`.fe-btn--active`）。

---

## 各類別改動清單

### 1. 主要動作（CTA）→ `.aiterm-btn .aiterm-btn--primary`

| 檔案 | 現有 class/style | 動作 |
|---|---|---|
| `LoopStudio/styles.css` | `.ls-start-btn` | 移除色彩/圓角/hover 宣告，僅留版面規則；JSX 疊加新 class |
| `DatabaseView/DatabaseAiChat.tsx:642` | inline 送出鈕 | 改為 `.aiterm-btn--primary.aiterm-btn--icon` |
| `Settings/AboutPage.css:49` | `.about-btn` | 同上 |
| `Settings/ProvidersPage.css:19` | `.btn-add` | 同上 |
| `Settings/EnterprisePage.tsx:159,231` | inline | 改為 className |
| `TerminalApp.tsx:419` | inline（Enterprise Execute） | 改為 className |
| `DocConverter/DocConverterView.css:144` | `.doc-converter__btn--primary` | 同上 |
| `Settings/DatabaseConnectionsPage.tsx:89` / `VcsConnectionsPage.tsx:101` | inline「+ Add」 | 改為 className |
| `DatabaseView/DatabaseSqlEditor.tsx:56` | inline「Run」 | 改為 className |
| `DatabaseView/DatabaseAiChat.tsx:457` | inline「Save as doc」 | 改為 className |
| `LoopStudio/styles.css:1103` | `.ls-session-resume-btn` | 同上 |

### 2. 次要／外框 → `.aiterm-btn .aiterm-btn--secondary`（或 `--ghost`，視有無邊框而定）

涵蓋：`Settings/McpServersPage.css`（`.mcp-btn-sm` 加 `--sm`）、`.mcp-tab-btn`（改走選中語彙，非按鈕語彙）、`Settings/ProvidersPage.css` `.provider-card-actions button`、`Settings/SettingsView.css` `.sidebar-item`（維持獨有的 translateX hover 動效，僅顏色/圓角對齊）、`Settings/DatabaseConnectionsPage.tsx` / `VcsConnectionsPage.tsx` 共用的 inline `btnStyle`（兩檔重複定義，順便消除重複）、`Settings/McpMarketplaceTab.tsx`、`Settings/EnterprisePage.tsx:224`、`DatabaseView/index.tsx:83,94`、`DatabaseView/DatabaseBrowser.tsx` 的 `modeBtn`/`pageBtn`、`FileExplorer/FileExplorer.css` `.fe-btn`、`TerminalView.css` `.aiterm-block-btn`/`.terminal-search-btn`、`DocConverter/DocConverterView.css` `.doc-converter__btn--secondary`、`LoopStudio/styles.css` 其餘 ~8 個次要按鈕 class。

### 3. 危險／刪除 → `.aiterm-btn .aiterm-btn--danger`

涵蓋：`Settings/ProvidersPage.css` `.btn-danger`、`Settings/McpServersPage.css` `.mcp-btn-sm.danger`、`Settings/DatabaseConnectionsPage.tsx` / `VcsConnectionsPage.tsx` 刪除鈕、`LoopStudio/styles.css` 的 `.ls-stop-btn`／`.ls-clear-confirm-yes`／`.ls-close-discard-btn`／`.ls-session-delete-btn`／`.ls-clear-all-btn`、`DatabaseView/DatabaseAiChat.tsx:627` 停止鈕（疊加 `--icon`）。`TerminalApp.tsx:431`（Enterprise「Reject」）目前是灰色外框但語意上是拒絕動作，一併改為 `--danger`，讓使用者能一眼辨識這是不可逆操作。

### 4. 圖示／圓形按鈕 → `.aiterm-btn--icon`（疊加 `--primary`/`--danger`/`--ghost`）

現有的漸層圓形送出鈕（VcsView、CrossDbView、AiPanel、DesignView、DatabaseAiChat）已經彼此一致，僅需把重複的 inline/CSS 宣告收斂成疊加 `.aiterm-btn--primary.aiterm-btn--icon`，不改變外觀。`Group 5`（`CommandBookmarks.css` 的 `.bookmarks-close`、`.bookmarks-item-delete`、`LoopStudio` 的 `.ls-dir-clear-btn` 等純圖示關閉鈕）改用 `.aiterm-btn--ghost.aiterm-btn--icon`。

### 5. 選中／狀態語彙

| 檔案 | 現有 | 動作 |
|---|---|---|
| `Settings/SettingsView.css` | `.sidebar-item--active`（紫） | 改用 `var(--accent)`（不變，本來就是 accent，僅確認 token 一致） |
| `TerminalView.css` | `.aiterm-subtab--active`（藍） | 改用 `var(--accent)` |
| `FileExplorer/FileExplorer.css` | `.fe-btn--active`（藍） | 改用 `var(--accent)` |
| `Settings/McpServersPage.css` | `.mcp-tab-btn.active`（綠） | 改用 `var(--accent)` |
| `TerminalView.css` `.aiterm-agent-toggle--on`、`DatabaseView/index.css`、`DesignView/DesignView.tsx` 沿用同一 class | 綠 | **不變**（狀態型，維持固定綠色） |

---

## 遷移策略

- 舊的面板專屬 class（如 `.ls-start-btn`）**保留**，但移除其中的 `background`/`border-radius`/`color`/`hover` 等外觀宣告，只留版面相關規則（`width`、`margin`、`flex` 等）。
- JSX 改為疊加 class：`className="ls-start-btn aiterm-btn aiterm-btn--primary"`。
- Inline `style={{}}` 按鈕：整個 `style` prop 移除，改成 `className`；若原本混有版面相關的 inline style（如 `marginLeft`），拆開保留在 `style` 或另建一個純版面用的 class。
- `DatabaseConnectionsPage.tsx` / `VcsConnectionsPage.tsx` 重複的 `btnStyle` 常數直接刪除，改用共用 class，順手清掉重複程式碼。
- 每個類別（1-5）各自獨立 commit，方便逐一檢查、有問題可單獨回退，不會一次性影響全部按鈕。

---

## 風險與因應

- **視覺回歸範圍大**：15+ 檔案、40+ 處改動，逐類別分批進行並在每類完成後截圖驗證（沿用先前 API Docs 修正時用過的 Playwright 截圖驗證流程），降低一次性大改的風險。
- **主題色連動**：現有 4 個主題（dark/light/nord/dracula）的 `--accent` 值差異大（綠/綠/藍/綠），統一後所有主要按鈕都會跟隨主題色——這是預期行為（跟現有 `.btn-primary`、API Docs 按鈕已經是的行為一致），非本次新增風險。
- **危險按鈕語意變更**（`TerminalApp.tsx:431` Reject 鈕從灰色外框改紅色）：純樣式調整，不改變點擊邏輯或確認流程，但視覺上會更醒目地標示為危險操作——這是本次修正想達成的效果，非副作用。

---

## 測試

- `npx tsc --noEmit` 全數通過（className 變更不影響型別，但 inline style 移除需確認無殘留 unused import 等 TS 錯誤）。
- `npm run lint` 通過，且不新增 lint 錯誤（原有 pre-existing 錯誤不在此次修正範圍內）。
- 每個類別完成後，用 Playwright 對代表性面板（LoopStudio、Settings 任一頁、DatabaseView）截圖比對，人工確認：圓角、hover 效果、主題色連動皆符合預期。
- 不新增自動化測試（純樣式改動，現有 Vitest 測試若涵蓋這些元件的 render，只要不斷言 className 字串，應能維持通過；若有測試斷言舊 className 存在，需同步更新斷言而非刪除測試）。

---

## 成功標準

1. `src/styles/buttons.css` 建立並在全域引入（`App.css` 或 `main.tsx`），定義 base + 5 variant + `--sm`/`--icon` 修飾。
2. 5 大類別涵蓋的所有檔案（清單如上）改用共用 class，舊有的重複顏色/圓角/hover CSS 宣告移除。
3. 「選中」語彙統一為 `var(--accent)`，「狀態（開關/連線）」語彙維持固定綠色不受主題影響。
4. `DatabaseConnectionsPage.tsx` / `VcsConnectionsPage.tsx` 的重複 `btnStyle` 常數消除。
5. `npx tsc --noEmit`、`npm run lint`（不新增錯誤）通過；各類別完成後皆有截圖驗證主要按鈕的漸層/圓角/hover、危險按鈕的紅色語彙、選中狀態的主題色連動皆正確呈現。
6. 任何按鈕的點擊行為、disabled 條件、確認流程與改動前完全一致（僅樣式變更）。

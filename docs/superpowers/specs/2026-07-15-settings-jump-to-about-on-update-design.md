# 有新版本時「設定」直接開啟關於頁 — 設計規格

**日期：** 2026-07-15
**狀態：** 已核准，待實作

---

## 目標

目前 App 偵測到有新版本可用時，只會在收起狀態的側邊欄齒輪圖示上顯示一個小紅點（update badge），但點擊「設定」進入後一律停在「一般」頁籤，使用者得自己再點一次「關於」才能看到新版本資訊、下載連結。

改為：**只要 App 偵測到有新版本可用，任何進入設定的方式都直接開啟「關於 (About)」頁籤**；沒有新版本時，維持現有行為（開啟「一般」頁籤）不變。

---

## 範疇

**包含：**
- 三個進入設定的入口，行為依 `hasUpdate` 狀態一致化：
  1. 側邊欄收起狀態的齒輪圖示（`TabBar/index.tsx`）
  2. 側邊欄展開狀態的「設定」文字項目（`TabBar/index.tsx`）
  3. `Ctrl+,` 鍵盤快捷鍵（`App.tsx`）

**不包含：**
- 不重構 `App.tsx` 與 `AboutPage.tsx` 目前各自獨立打一次 GitHub tags API 的重複更新檢查邏輯（技術債，已在設計討論中記錄，留待未來需要時再處理）。
- 不修改 `SettingsView.tsx` 的頁籤解析邏輯——它已經支援透過 React Router 的 `location.state.tab` 決定初始頁籤（`SettingsView.tsx:27`），本次改動完全複用這個既有機制。
- 不改變 update badge 本身的顯示邏輯（收起狀態齒輪圖示已有 badge，展開狀態「設定」項目目前沒有 badge，本次不新增）。

---

## 核心機制

`SettingsView.tsx` 已存在的邏輯：

```tsx
const initialTab = (location.state as { tab?: SettingsTab } | null)?.tab ?? "general";
const [tab, setTab] = useState<SettingsTab>(initialTab);
```

`SettingsTab` 型別已包含 `"about"` 這個值，對應渲染 `AboutPage` 元件（`SettingsView.tsx:22`, `89-96`）。因此本次改動只需要在三個呼叫 `navigate("/settings")` 的地方，依 `hasUpdate` 決定要不要帶入 `state: { tab: "about" }`：

```tsx
navigate("/settings", hasUpdate ? { state: { tab: "about" } } : undefined);
```

沒有新版本（`hasUpdate === false`）時傳 `undefined`，等同現在的 `navigate("/settings")`，`SettingsView` 落回預設值 `"general"`，行為不變。

---

## 各入口改動細節

### 1. `src/components/TabBar/index.tsx:96-104`（收起狀態齒輪圖示）

現有：
```tsx
<button ... onClick={() => navigate("/settings")} title={`${t.settings} (Ctrl+,)`}>
  ⚙
  {hasUpdate && <span className="update-badge" aria-label="Update available" />}
</button>
```
`hasUpdate` 已經是這個元件既有的 prop（`TabBar/index.tsx:75`），改動僅限 `onClick` 這一行。

### 2. `src/components/TabBar/index.tsx:188-198`（展開狀態「設定」項目）

現有：
```tsx
<div className="aiterm-tab" onClick={() => navigate("/settings")} title={`${t.settings} (Ctrl+,)`}>
  <span className="aiterm-tab-icon">⚙️</span>
  <span className="aiterm-tab-title">{t.settings}</span>
</div>
```
同一個元件作用域內，`hasUpdate` 已可取用，改動僅限 `onClick` 這一行。

### 3. `src/App.tsx:55-64`（`Ctrl+,` 快捷鍵）

這段程式碼與計算 `updateInfo`/`hasUpdate` 的 `useEffect`（`App.tsx:34-52`）在同一個 `AppRoutes` 元件作用域內，`updateInfo?.hasUpdate` 已可直接取用，改動僅限快捷鍵處理函式內呼叫 `navigate` 的那一行。

---

## 資料流

```
App.tsx (AppRoutes)
  └─ updateInfo state（GitHub tags API 比對版本，useEffect 於 mount 時執行一次）
       ├─ 傳給 TerminalApp 的 hasUpdate prop
       │    └─ 傳給 TabBar 的 hasUpdate prop
       │         ├─ 收起齒輪圖示 onClick → navigate("/settings", hasUpdate ? {state:{tab:"about"}} : undefined)
       │         └─ 展開「設定」項目 onClick → 同上
       └─ Ctrl+, 快捷鍵處理函式（同一元件作用域）→ 同上
                                                    ↓
                                    SettingsView 讀取 location.state.tab
                                    （已存在邏輯，本次不修改）
                                                    ↓
                                    tab === "about" → 渲染 AboutPage
                                    （AboutPage 掛載時自行重新檢查一次更新狀態，如現有行為）
```

---

## 錯誤處理

- `updateInfo` 為 `null`（尚未完成版本檢查，或 fetch 失敗）時，`hasUpdate` 的存取一律用 `?? false` 保底（沿用 `App.tsx:84` 既有寫法：`updateInfo?.hasUpdate ?? false`），此時三個入口都退回現有的「不帶 state」行為，開啟「一般」頁籤。
- 不需要額外的 try/catch——`navigate()` 呼叫本身不會拋錯，`hasUpdate` 的計算邏輯完全複用既有、已上線的程式碼。

---

## 測試

- 前端測試（Vitest + RTL）：對 `TabBar` 元件補測試（若 `TabBar` 目前沒有既有測試檔案，新建一個）：
  1. `hasUpdate={true}` 時，點擊收起狀態齒輪圖示 → 斷言 `navigate` 被呼叫時帶有 `{ state: { tab: "about" } }`。
  2. `hasUpdate={false}` 時，點擊同一按鈕 → 斷言 `navigate` 被呼叫時 `state` 為 `undefined`（或未帶 state 參數）。
  3. 展開狀態「設定」項目重複上述兩個案例。
- `App.tsx` 的 `Ctrl+,` 快捷鍵邏輯：若 `App.tsx` 目前有測試檔案覆蓋鍵盤快捷鍵，比照上述兩案例補測試；若目前完全沒有測試覆蓋這段快捷鍵邏輯，本次不強制新增（避免測試基礎建設範圍擴大超出此次改動），僅靠手動驗證。

---

## 成功標準

1. 有新版本可用時（`updateInfo?.hasUpdate === true`），透過收起齒輪圖示、展開「設定」項目、`Ctrl+,` 三種方式進入設定，都直接顯示「關於」頁籤內容。
2. 沒有新版本時，三種方式都維持開啟「一般」頁籤（回歸測試，不能破壞現有行為）。
3. `npm run test`、`npx tsc -b tsconfig.app.json --force`、`npm run lint` 全數通過，且新增/修改的測試涵蓋上述兩種狀態。

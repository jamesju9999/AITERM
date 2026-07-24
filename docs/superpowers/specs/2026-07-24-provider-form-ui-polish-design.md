# AI 供應商設定表單 UI 優化

**日期**：2026-07-24
**狀態**：待審閱

## 背景與目標

使用者回報 `ProviderForm.tsx`（設定 → AI 供應商 → 新增/編輯供應商彈窗）的 GitHub Copilot 驗證區塊視覺上卡住——「OAuth Client ID」輸入框跟「以 GitHub 裝置驗證登入」按鈕被塞在同一個 `.form-group` 內，只有 6px 間距，看起來像重疊/卡住。

檢查程式碼後確認這是結構性問題：`{providerType === "github-copilot" && (...)}` 這個條件區塊把兩組邏輯上不同的欄位（OAuth Client ID 輸入 + 裝置驗證按鈕）塞進同一個 `<div className="form-group">`，而 CSS 對該 class 只有 `display:flex; flex-direction:column; gap:6px`，沒有對子群組做視覺分隔。

順帶發現的另一個問題：所有欄位標籤套用了 `.form-group label { text-transform: uppercase }`，導致英文標籤（Base URL、OAuth Client ID、Model）變成全大寫的喊叫感，跟中文標籤（類型、顯示名稱）風格不一致。

使用者在確認前述兩個具體 bug 之外，進一步要求「跟系統風格一致，例如按鈕的風格」——調查後發現 `ProviderForm.tsx` 完全沒有套用 App 全域已存在的統一按鈕系統（`src/styles/buttons.css` 的 `aiterm-btn` class，詳見 `docs/superpowers/specs/2026-07-16-unified-button-system-design.md`），而是自己刻了一套深藍色（`#2d5aab`）按鈕樣式，跟 App 全域紫色漸層主色（`--accent: #a855f7` → `#6366f1`）脫節。供應商清單頁（`ProvidersPage.tsx`）的「測試／編輯／移除」按鈕已經套用 `aiterm-btn`，形成同一個功能模組內兩種按鈕語言並存的不一致狀態。

本次目標：修正上述具體 bug，並將整個表單重新分區塊、換上 App 既有的視覺語言，讓它跟系統其餘部分一致。

## 範圍界定（已透過視覺化 mockup 與使用者確認）

三個排版方向（A. 只修 bug／B. 分區塊呈現／C. 卡片式供應商選擇+分區）中，使用者選擇 **B**，並加上「套用系統既有視覺語言」的要求。

### 明確排除（Non-goals）

- 不把最上面的供應商類型選單換成卡片式選擇器（方向 C 的範圍，本次不做）。
- 不新增任何 React `<Button>` 元件，維持現有「原生 `<button>` + className」寫法（比照 `2026-07-16-unified-button-system-design.md` 的既有原則）。
- 不變更任何欄位的業務邏輯：不動 `useEffect` 抓取模型清單的邏輯、不動 `handleSave` 的驗證規則本身（只調整其視覺呈現位置）、不動任何 IPC 呼叫、不動任何 Tauri command。純粹是 JSX 結構重組 + CSS 換皮。
- 不寫新的自動化測試 — `ProviderForm.tsx` 目前沒有任何測試涵蓋（先前多次 code review 已確認並接受此現狀），本次不新增測試框架，僅手動驗證。
- 不處理 `ProvidersPage.tsx` 供應商清單卡片本身的樣式（已經套用 `aiterm-btn`，不在本次範圍）。

## 設計

### 1. 三區塊分組規則

把現有欄位重新分組進三個視覺區塊，適用全部 11 種供應商類型：

| 區塊 | 涵蓋欄位 |
|---|---|
| **基本資訊** | 類型（Type 下拉選單）、ID、顯示名稱 |
| **驗證方式** | 通用 API Key 輸入框／Anthropic 的「API Key・Claude Pro/Max」分頁切換與其對應內容／GitHub Copilot 的「OAuth Client ID」輸入框 + 裝置驗證按鈕 |
| **端點與模型** | Base URL（含快選按鈕）、Model（含各供應商各自的動態抓取/datalist 邏輯）、JSON Mode 勾選框 |

**規則：若某供應商類型在該區塊完全沒有對應欄位，該區塊整個不渲染**（例如 Ollama 沒有 API Key 也沒有 OAuth，「驗證方式」區塊直接不出現，不會渲染一個空卡片）。這個規則同時涵蓋現有 11 種類型的所有既有條件邏輯，不需要新增任何欄位顯示/隱藏判斷式，只是把既有的 JSX 區塊搬進對應的區塊容器裡。

### 2. 元件結構

新增一個小型可重用包裝元件（在 `ProviderForm.tsx` 檔案內定義即可，不需要獨立檔案，因為只有這個表單使用）：

```tsx
function FormSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="form-section">
      <div className="form-section-title">{title}</div>
      {children}
    </div>
  );
}
```

三個區塊都用它包裝，內部維持原本的 `.form-group` 結構與既有的條件渲染邏輯不變，只是外層多一層 `<FormSection>`。若某區塊在特定 provider type 下沒有任何子欄位符合條件，該 `<FormSection>` 呼叫本身也不會被渲染（用跟現有欄位一樣的 `{condition && (...)}` 模式包住整個 `<FormSection>`）。

### 3. 視覺與 CSS token 一致性

**顏色**：不新增色碼，全部改用 App 既有的 CSS 變數（定義於 `src/App.css`）：

| 用途 | Token | 目前值 |
|---|---|---|
| 區塊標題文字 | `var(--accent)` | `#a855f7` |
| 區塊卡片背景 | `var(--bg-secondary)` | `#111` |
| 區塊卡片邊框 | `var(--border-color)` | `#2a2a2a` |

**按鈕**：全面改用 `src/styles/buttons.css` 的 `aiterm-btn` 系統：
- 主要動作（儲存、GitHub 裝置驗證登入、Claude OAuth「開啟瀏覽器」/「確認完成驗證」）→ `aiterm-btn aiterm-btn--primary`
- 次要動作（取消、OAuth 登出、快選按鈕、OAuth 取消）→ `aiterm-btn aiterm-btn--secondary`（快選按鈕字級較小，額外疊加 `aiterm-btn--sm`）

移除 `ProviderForm.css` 中對應的舊按鈕樣式定義（`.anthropic-oauth-open`、`.anthropic-oauth-logout`、`.anthropic-oauth-cancel-btn`、`.form-actions button` 等的顏色/背景宣告），改為套用上述 class。Anthropic 的「API Key / Claude Pro/Max」分頁切換（`.auth-tab.active`）也改用 `var(--accent-gradient)` 而非現有的 `#2d5aab`。

**標籤大小寫**：移除 `.form-group label` 的 `text-transform: uppercase; letter-spacing: 0.04em;`，改為一般大小寫呈現。

### 4. 彈窗寬度

`ProvidersPage.css` 的 `.provider-form-panel` 寬度從 `480px` 加寬到 `560px`，因為分區塊卡片加上內距後，480px 在（例如）ID／顯示名稱並排的情境下會偏擠。

## 錯誤處理

不變更任何驗證/錯誤訊息邏輯——`form-error`（頂層儲存錯誤）、`form-hint`／`form-hint--error`（欄位層級提示，如 OAuth 狀態訊息）維持原本的顯示位置與觸發條件，僅隨著其所屬欄位一起搬進對應的 `<FormSection>`。

## 測試計畫

- 不新增自動化測試（維持現狀，理由同上）。
- 存檔後執行 `npx tsc --noEmit -p tsconfig.app.json` 與 `npm run lint`（作用範圍限定 `ProviderForm.tsx`/`ProviderForm.css`/`ProvidersPage.css`，比對修改前後的既有 lint baseline，確保沒有新增問題）確認無新增錯誤。
- 手動驗證：對 11 種供應商類型各開一次「新增供應商」表單，確認：
  1. 每個類型只出現該類型實際需要的區塊（無空區塊）。
  2. GitHub Copilot 的驗證區塊不再視覺卡住。
  3. 按鈕顏色/樣式跟供應商清單頁一致。
  4. 標籤不再全大寫。
  5. 彈窗寬度變化未破版（尤其 480px→560px 後，`.provider-form-overlay` 的置中與 `max-height: 90vh; overflow-y: auto` 仍正常運作）。

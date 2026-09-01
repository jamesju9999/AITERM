# Artifact Panel（AiPanel 落地版）Design Spec

## 背景與問題

使用者希望 AITerm 在任何功能裡，只要 AI 需要顯示「文件」或「圖表」，都有能力處理——
參考 Claude.ai 桌面版的 Artifacts：聊天在左，AI 生成的文件/圖形渲染在右側面板，可展開、
可下載。

這是一個橫跨多個既有子系統的大功能。經過現況調查（見下一節），確認 AITerm 目前有
**三套完全獨立**的聊天介面，沒有任何一套具備這種能力，也完全沒有 iframe 沙盒渲染或
圖表套件的先例——這代表要先建立一套與入口無關的共用機制，而不是在某個特定入口裡
硬塞一個一次性做法。

## 範圍界定：這份 spec 只做第一個里程碑

**這次只在 `AiPanel`（`ChatPanelShell`）落地**，把共用機制的核心（context、面板 UI、
兩種內容類型的渲染、AI 標記協定）做完整、做對。`DesignView`（Design 分頁寫 spec 用的
聊天）與 `DatabaseAiChat`/`CrossDbAiChat`（資料庫分頁的 AI 聊天，使用者關心的「資料庫
圖表」情境就在這裡）**刻意留給之後的獨立里程碑**——但共用機制從一開始就設計成
entry-point-agnostic（見「架構」一節），之後接上時理論上只要訂閱同一個 context、套用
同樣的面板元件，不需要重新設計。

## 現況調查

1. **三套獨立聊天介面，互不共用渲染邏輯**：
   - `AiPanel`/`src/components/ChatPanel/ChatPanelShell.tsx` — 日常終端機側邊欄，
     固定寬度、可拖拉調寬（`MIN_WIDTH=280`、`MAX_WIDTH_RATIO=0.75`，
     `ChatPanelShell.tsx:14-15`）、有「展開至 100% 寬度」的 `expanded` 開關
     （`ChatPanelShell.tsx:132,215,251-258`）。訊息渲染走
     `MessageList` → `MessageBubble`（`ChatPanelShell.tsx:323-331`）。
   - `DesignView`（`src/components/DesignView/DesignView.tsx`）— Design 分頁專用，
     已經有左聊天/右預覽的手刻分割版型（`.design-left-panel` + `.design-resizer` +
     `.design-right-panel`，`DesignView.tsx:436-620`），拖拉邏輯用
     `mousedown`/`mousemove`/`mouseup`（`DesignView.tsx:154-179`）。但預覽只能渲染
     Markdown（`SpecPreview.tsx:198`），協定是字串標記 `[UPDATE_SPEC]...[/UPDATE_SPEC]`
     這類 tag（`DesignView.tsx:325-360`），原始設計文件寫的是要做成 tool call，實際
     卻是字串比對——這是已知的技術債，這次不沿用這個協定設計。
   - `DatabaseAiChat`/`src/components/DatabaseView/DatabaseAiChat.tsx`（756 行）與
     `CrossDbAiChat` — 資料庫分頁「ai」子分頁的**主要內容**（不是側邊欄，該子分頁裡
     沒有終端機跟它搶版面），完全獨立的訊息狀態與渲染（`MessageBubble`/`ResultInline`
     自己刻一份，`DatabaseAiChat.tsx:633-756`）。查詢結果目前純粹是 HTML `<table>`
     （`DatabaseAiChat.tsx:721-756`、`DatabaseSqlEditor.tsx:96-117`），沒有匯出/視覺化
     按鈕。
2. **完全沒有圖表套件**：`package.json` 與 `src/` 全域搜尋 chart/recharts/chart.js/d3/
   plotly 都沒有命中。唯一沾邊的是 Markdown 裡的 mermaid 區塊
   （`src/lib/markdown.tsx:111-113` 的 `code` renderer 特別處理
   ` ```mermaid ` fenced block，渲染成 `MermaidBlock`）——能畫圓餅圖/簡單流程圖，
   但沒有把任意查詢欄位資料綁定成長條圖/折線圖的能力。
3. **完全沒有 iframe/沙盒渲染先例**：`grep -rn "iframe\|sandbox="` 在 `src/` 底下零
   命中。唯一的「注入未經處理內容」先例是 `MermaidBlock.tsx:103,119` 用
   `dangerouslySetInnerHTML` 注入渲染好的 SVG 字串——不是沙盒，只是把已知安全的渲染
   結果塞進 DOM，跟這次要處理「AI 生成的任意 HTML/JS」是不同等級的風險。
4. **`react-markdown` 目前沒有掛 `rehype-raw`**（`src/lib/markdown.tsx:1-7`），代表
   聊天泡泡裡的原始 HTML 目前是被跳脫顯示、不會被渲染——這次新的 Artifact 內容不會
   透過這條既有的 Markdown 渲染路徑輸出成真的 HTML，必須另開一條路徑（見下方
   「安全性」）。

## 設計

### 1. 整體架構

新增一個與入口無關的 React context：`src/contexts/ArtifactPanelContext.tsx`，提供
`useArtifactPanel()` hook，介面：

```ts
type ArtifactKind = "html" | "chart";

interface Artifact {
  id: string;
  kind: ArtifactKind;
  title: string;
  /** kind="html": 完整 HTML 字串。kind="chart": 未解析的 JSON 字串（面板元件自己 parse）。 */
  content: string;
}

interface ArtifactPanelState {
  activeArtifact: Artifact | null;
  showArtifact: (artifact: Artifact) => void;
  clearArtifact: () => void;
}
```

**這個 context 要 per-tab 各自獨立**，不能是全域單例——否則切分頁時會看到別的分頁
殘留的文件/圖表。做法是把 `<ArtifactPanelProvider>` 包在 `ChatPanelShell` 自己的
return 裡（而不是包在 `TerminalView.tsx` 或更上層），因為 `ChatPanelShell` 本來就是
每個分頁各自掛載一份實例，且它自己的 JSX 裡已經直接渲染了 `MessageList`
（`ChatPanelShell.tsx:323-331`，往下是 `MessageBubble`/`MarkdownText`）——`code`
renderer 跟面板 UI 兩個消費端都在同一棵子樹裡，包在 `ChatPanelShell` 內部就能同時
覆蓋兩者，**不需要改動 `TerminalView.tsx`**。

### 2. AI 怎麼標記「這是要顯示的文件/圖表」

擴充 `src/lib/markdown.tsx:108-127` 既有的 `code` renderer（就是目前特殊處理
` ```mermaid ` 的那段），新增兩個 language 分支：

- ` ```artifact-html ` — 內容是完整 HTML 字串。
- ` ```artifact-chart ` — 內容是圖表規格 JSON 字串，例如
  `{"type":"bar","title":"...","data":[{"month":"Jan","sales":120},...],"xKey":"month","series":[{"key":"sales","label":"Sales"}]}`。

跟 mermaid 不同的地方：mermaid 是「原地渲染在聊天泡泡裡」，這兩個新 language 改成
「偵測到就透過 `useArtifactPanel().showArtifact(...)` 登記進 context，聊天泡泡裡只
留一張精簡卡片」（新元件 `ArtifactBlockCard`，放在 `src/components/ArtifactPanel/`）。
登記動作必須放在 `useEffect` 裡（依 `content` 字串是否變動觸發），不能直接在渲染期間
呼叫 `showArtifact`——React 不允許在渲染一個元件的過程中觸發另一個元件的狀態更新。

這個協定沿用既有、已經在跑的 fenced-code-block parsing 機制（`react-markdown` 的
`code` renderer 擴充點），不用像 DesignView 那樣另外設計字串標記協定；也因為
remark 對「還沒收到結束 ` ``` `」的區塊不會產生完整的 `code` node，天生就達到
「串流中不渲染半成品」的效果（見下方「串流行為」）。

### 3. 兩種內容類型的渲染與安全性

**`artifact-chart`**：新元件 `src/components/ArtifactPanel/ArtifactChart.tsx`，用
`recharts`（新增依賴）把 JSON 規格畫成圖。因為 AI 只給結構化資料、實際畫圖的是我們
自己信任的元件，AI 沒有機會夾帶任意程式碼，**不需要沙盒**。配色/圖表類型選擇/圖例/
hover tooltip 這些細節照專案裡 `dataviz` skill 的規範走（分類色固定順序、循序色單一
色相深淺、圖例二個序列以上必備、深色模式獨立驗證），色票值抄一份進
`src/lib/chartPalette.ts`，開發時用該 skill附的 `validate_palette.js` 驗證過再定案。

**`artifact-html`**：新元件 `src/components/ArtifactPanel/ArtifactHtmlFrame.tsx`，用
`<iframe sandbox="allow-scripts" srcDoc={html} />` 渲染。關鍵決定：

- **給 `allow-scripts`**（使用者確認要能跑 JS，畫面才能豐富）。
- **絕對不給 `allow-same-origin`**——這是這個沙盒設計唯一不能退讓的部分。沒有
  `allow-same-origin`，iframe 內容被瀏覽器視為一個獨立的不透明來源（opaque origin），
  裡面的 JS 完全無法碰到主視窗的 DOM、無法讀主視窗的 `localStorage`、更不可能碰到
  Tauri 的 IPC bridge（那是掛在主視窗 origin 上的）。如果 `allow-scripts` 跟
  `allow-same-origin` 同時給，沙盒等於形同虛設。
- 殘留風險（v1 接受、不特別處理）：sandbox 屬性不會擋掉一般的網路請求（`fetch`/
  `<img>` 等），所以裡面的 JS 理論上還是能對外發送請求——但因為它拿不到主視窗任何
  資料，能外洩的頂多是它自己 DOM 裡本來就看得到的內容，風險等級跟一般網頁內嵌廣告
  iframe 相同，v1 不另外加 CSP/網路層限制。

### 4. 面板 UI / 佈局

`ChatPanelShell` 訂閱 `useArtifactPanel()`：`activeArtifact` 非 null 時，在既有
`.aiterm-ai-panel` 內部裂成「窄聊天欄 + `ArtifactPanel` 面板」兩欄，做法比照
`DesignView.tsx:154-179` 的手刻拖拉分割（`mousedown`/`mousemove`/`mouseup`，不用額外
裝分割面板套件——repo 裡目前所有分割版型都是手刻的，維持一致）。同時比照現有
`expanded` 邏輯讓整個 `.aiterm-ai-panel` 寬度自動撐到 `MAX_WIDTH_RATIO`（75%）上限，
讓文件/圖表有足夠空間，終端機仍留在畫面上（不是三欄擁擠，是側邊欄自己變寬、內部再切
兩欄）。`ArtifactPanel.tsx` 是最外層的分派元件：依 `activeArtifact.kind` 選
`ArtifactHtmlFrame` 或 `ArtifactChart`，header 有標題、關閉按鈕（呼叫
`clearArtifact()` 收回成單欄聊天）。

### 5. 串流行為

v1 只在 ` ```artifact-html `/` ```artifact-chart ` 區塊完整收到（remark 解析出完整
`code` node）才渲染進面板，不做「AI 邊生成邊即時更新面板內容」的逐字預覽——這個決定
不需要額外程式碼，是 remark 對未閉合 fenced block 的既有解析行為自然帶來的效果（見
上一節）。之後如果想要即時預覽，需要另外設計「解析半成品 HTML/JSON」的邏輯，複雜度
高很多，留給未來評估。

## 明確排除（Non-goals）

- **不整合 `DesignView`**：它現有的 Markdown-only 預覽 + 字串標記協定維持原樣，之後
  接上 `ArtifactPanelContext` 是獨立里程碑。
- **不整合 `DatabaseAiChat`/`CrossDbAiChat`**：使用者關心的「資料庫查詢結果畫圖表」
  情境會用到這次做的 `ArtifactChart`，但接線工作（讓 `DatabaseAiChat` 訂閱同一個
  context、套用同樣的分割版型）留給下一個里程碑，這次不做。
- **不做 `artifact-html` 的即時串流預覽**（見上）。
- **不處理 sandbox 之外的網路層限制**（CSP 等，見「安全性」殘留風險段落）。
- **不新增分割面板函式庫**：沿用 repo 現有的手刻拖拉分割慣例。

## 對既有程式碼的影響

- 新增：`src/contexts/ArtifactPanelContext.tsx`、
  `src/components/ArtifactPanel/{ArtifactPanel,ArtifactHtmlFrame,ArtifactChart,ArtifactBlockCard}.tsx`、
  `src/lib/chartPalette.ts`。
- 修改：`src/lib/markdown.tsx`（`code` renderer 新增兩個 language 分支）、
  `src/components/ChatPanel/ChatPanelShell.tsx`（訂閱 context、內部分割版型、寬度
  自動撐開邏輯）。
- 新增依賴：`recharts`（`package.json`）。
- 不修改：`TerminalView.tsx`、`DesignView.tsx`、`DatabaseAiChat.tsx`、後端 Rust
  （這次完全是前端功能，AI 只是被動輸出符合協定的 fenced code block，不需要新的
  IPC 或系統提示變更——後端 prompt 目前的 `<cmd>` tag 說明已經足夠讓模型知道有其他
  fenced code block 慣例存在，之後真的需要教模型主動輸出 `artifact-html`/
  `artifact-chart` 時，屬於 prompt 調整範疇，可能需要另外評估要不要在系統提示裡
  加入這個新協定的說明——這點目前有意留白，v1 先驗證面板/渲染機制本身能不能動，
  由使用者手動測試時人工構造符合協定的內容來驗證即可）。

## 測試策略

### 前端單元測試

1. `markdown.tsx` 的 `code` renderer：餵一個含 ` ```artifact-html ` 完整區塊的
   markdown 字串，驗證 `MarkdownText` 渲染出 `ArtifactBlockCard` 而不是原始
   `<pre><code>`；驗證未閉合的區塊（缺結尾 ` ``` `）不會觸發卡片渲染。
   `artifact-chart` 同理。
2. `ArtifactPanelContext`：`showArtifact`/`clearArtifact` 的狀態轉換；同一個
   provider 底下多次 `showArtifact` 會取代前一個（不是疊加）。
3. `ArtifactHtmlFrame`：渲染出的 `<iframe>` 的 `sandbox` 屬性值精確等於
   `"allow-scripts"`（明確斷言**不包含** `allow-same-origin`，這是安全性的核心
   斷言，值得寫一個專門測試防止未來被不小心加回去）。
4. `ArtifactChart`：給定一個範例 JSON 規格，驗證選出正確的圖表類型元件；驗證色票
   來自 `chartPalette.ts`（不是隨機/循環產生）。

### 無法只靠單元測試涵蓋的部分（需要真機驗證）

- `ChatPanelShell` 內部分割版型的實際拖拉手感、寬度自動撐開時終端機是否留有合理
  空間（不同視窗寬度下）。
- 真的請 AI 生成一段 ` ```artifact-html ` 內容（先靠使用者手動在聊天輸入框構造
  符合協定的內容測試渲染路徑本身，因為 v1 還沒教模型主動輸出這個協定，見上方
  「對既有程式碼的影響」最後一點）。
- iframe 沙盒是否真的如預期擋下對主視窗/Tauri IPC 的存取——可以寫一段刻意嘗試
  `window.parent`/`window.top` 存取或呼叫 `window.__TAURI__` 的測試 HTML 餵進
  `artifact-html`，人工確認全部失敗/undefined。

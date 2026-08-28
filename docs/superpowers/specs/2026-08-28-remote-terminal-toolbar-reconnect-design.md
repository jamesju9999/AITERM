# 遠端終端機工具列：品牌樣式、換行、就地重新連線 Design

**Goal:** 在剛完成的遠端終端機工具列基礎上，加入 AI 面板同款的品牌 sparkle 樣式、允許連線資訊文字換行，並新增一顆「連線」按鈕，讓使用者可以在同一個分頁裡直接切換去連另一台主機，不需要每次都跑一次 ADD TAB 流程。

## 背景

使用者測試剛完成的遠端終端機工具列時，發現全螢幕程式（Claude Code CLI）分頁下方仍留有明顯空白——根本原因是本機 `TerminalView.tsx` 的工具列除了 `.aiterm-status` 這一列之外，下面還有一列 `.aiterm-subtabs`（Terminal/Files 分頁切換＋cwd 路徑），遠端分頁目前完全沒有對應物，兩邊佔用的頂部空間本來就不一樣高。這個設計不試圖精確填滿這個結構性差異（那需要重新加回一整列已經在上一輪設計裡明確排除的分頁列），而是採納使用者提出的方向：加入品牌樣式＋允許換行＋加一顆新按鈕，讓工具列本身多用一點空間，同時新增使用者這次額外想要的「就地重新連線」功能。

## 範圍

**這次要做的：**
1. 連線資訊文字前綴加上跟 `AiPanel` 同款的漸層 sparkle 品牌樣式。
2. 整個連線資訊文字允許換行（不再被截斷成單行）。
3. 新增「連線」按鈕，位置在「指令書籤」左側，點下去跳出跟 ADD TAB 完全一樣的 `ConnectDialog`，連線成功後**不開新分頁**，直接讓目前這個分頁切換過去。
4. 底層：`RemoteTerminalView` 用 `key={tab.remoteConnId}` 強制在連線切換時重新掛載，避免逐一手動清空內部 state。

**明確不做的：**
- 不加回本機工具列的第二列（`.aiterm-subtabs` 等價物）——這是已知、這次沒有完全解決的結構性差異，不在這次範圍內。
- 不處理「切換連線前先確認」這類保護性 UI（例如彈出「確定要斷線嗎？」）——使用者沒有要求，且這個操作本身可逆（隨時可以再按一次連線鈕連回去）。
- 不改變「指令書籤」「Ask AI」兩顆既有按鈕的任何行為。

## 1. 品牌樣式與換行

找到 `RemoteTerminalView` 目前的連線資訊 span：

```tsx
<span className="aiterm-status-left" data-tauri-drag-region>
  AITerm · {t.remote_terminal_tab} {hostLabel} · {connectionStatusText(t, phase, elapsedMs)}
</span>
```

把開頭的純文字 `AITerm` 換成沿用 `AiPanel/index.tsx` 既有的漸層 sparkle 樣式（`background: var(--accent-gradient)` + `WebkitBackgroundClip: 'text'` + `WebkitTextFillColor: 'transparent'`，見 `AiPanel/index.tsx:520`），其餘「· 遠端終端機 {hostLabel} · {連線狀態}」原樣接在後面、維持一般文字顏色：

```tsx
<span className="aiterm-status-left" data-tauri-drag-region style={{ whiteSpace: "normal" }}>
  <span style={{ background: "var(--accent-gradient)", WebkitBackgroundClip: "text", WebkitTextFillColor: "transparent", fontWeight: 700 }}>
    ✨ AITerm
  </span>{" "}
  · {t.remote_terminal_tab} {hostLabel} · {connectionStatusText(t, phase, elapsedMs)}
</span>
```

`whiteSpace: "normal"`：`.aiterm-status-left` 目前沒有設定 `white-space`，瀏覽器預設值已經是 `normal`（允許換行），這裡明確寫出來只是為了讓意圖清楚（避免以後有人在 `TerminalView.css` 幫 `.aiterm-status-left` 加 `nowrap` 時，這裡被意外波及卻沒人注意到）——不是修正一個現有的截斷問題。

## 2. 「連線」按鈕

**位置：** `.aiterm-status` 右側按鈕 span 裡，「指令書籤」按鈕之前。

**外觀：** 沿用既有按鈕樣式慣例，`className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"`（跟「指令書籤」同一個次要按鈕樣式），圖示用 `LinkIcon`（`src/components/Icons.tsx` 既有、目前沒地方用到的連結圖示），文字沿用 `t.remote_terminal_tab`（「遠端終端機」）或新增一個更精準的翻譯鍵——用新翻譯鍵 `remote_terminal_toolbar_connect_button`（「連線」/ "Connect"），比直接複用「遠端終端機」這個分頁名稱語意更準確。

**行為：** `onClick` 呼叫新的 `onConnectClick` prop（`RemoteTerminalView` 新增的 callback prop，由 `TerminalApp.tsx` 提供）。

## 3. 資料流：就地重新連線

### `RemoteTerminalView` 新增 prop

```tsx
interface Props {
  // ...既有欄位不動...
  /**
   * 使用者點了工具列的「連線」按鈕。由 TerminalApp.tsx 提供：開啟
   * ConnectDialog，並記住是「這個分頁」要求重新連線——連線成功後
   * TerminalApp.tsx 會更新這個分頁的 remoteConnId/remoteSas/
   * remoteHostLabel，不會開新分頁。
   */
  onConnectClick: () => void;
}
```

不給預設值（不像 `hostLabel` 是選填）：這個 prop 沒有意義的「安全預設值」可以退回（總不能預設成 no-op，那樣按鈕點了沒反應會很奇怪），`TerminalApp.tsx` 這次會同時完成呼叫端更新，不需要顧慮既有測試呼叫端相容性的問題——但測試檔案裡既有的 `render(<RemoteTerminalView ... />)` 呼叫需要補上這個必填 prop（見下方測試段落）。

### `TerminalApp.tsx` 變更

新增狀態，記住「目前這次開啟 ConnectDialog，是不是某個既有分頁要求重新連線」：

```tsx
const [reconnectTabId, setReconnectTabId] = useState<string | null>(null);
```

`<RemoteTerminalView>` 呼叫處：

```tsx
) : tab.type === "remote-terminal" ? (
  <RemoteTerminalView
    key={tab.remoteConnId}
    tabId={tab.id}
    connId={tab.remoteConnId ?? ""}
    sas={tab.remoteSas ?? ""}
    isActive={isActive}
    hostLabel={tab.remoteHostLabel ?? ""}
    onConnectClick={() => {
      setReconnectTabId(tab.id);
      setConnectOpen(true);
    }}
  />
) : (
```

`key={tab.remoteConnId}`：這是這次「就地重新連線」能不手動清空一堆 state 就正確運作的關鍵。`RemoteTerminalView` 內部有 `phase`／`connectedAtRef`／`elapsedMs`／`liveRows`／`hostRows`／`bookmarksOpen`／`aiUnsupported`／`hostPlatform`，以及 `useTerminalBlocks` 內部自己的 `blocks`／`isAlternateBuffer` 狀態，還有 xterm 實例本身——這些全部都是「只在掛載當下初始化一次」的 state，`connId` prop 換了值並不會讓它們自動歸零（React 不會因為 prop 變了就重新跑 `useState` 的初始值運算式）。給元件的 `key` 換成 `tab.remoteConnId`，等於明確告訴 React「`remoteConnId` 不同就是完全不同的元件實例」——`remoteConnId` 改變時 React 會把舊的整個卸載（連帶正確觸發既有的斷線 cleanup effect，斷掉舊連線）、掛一個全新的（從 `phase: {kind:"waiting", sas}` 這個初始狀態重新開始，就跟第一次連線時一模一樣），不需要在元件內部另外寫任何「切換時清空 XXX」的邏輯，也不會有漏清某個 state 的風險。

外層那層 `<div key={tab.id} ...>`（`TerminalApp.tsx` 既有的、每個分頁位置固定用的 key）維持不動——`tab.id`（分頁本身的身分）跟 `tab.remoteConnId`（這個分頁「當下代表哪一條連線」）是兩個不同層次的識別，巢狀用兩個不同的 key 沒有衝突。

`ConnectDialog` 呼叫處：

```tsx
{connectOpen && (
  <ConnectDialog
    onCancel={() => {
      setConnectOpen(false);
      setReconnectTabId(null);
    }}
    onConnected={(connId, sas, hostLabel) => {
      setConnectOpen(false);
      if (reconnectTabId) {
        const targetId = reconnectTabId;
        setReconnectTabId(null);
        setTabs((prev) =>
          prev.map((t) =>
            t.id === targetId
              ? { ...t, title: `${t.remote_terminal_tab}：${hostLabel}`, remoteConnId: connId, remoteHostLabel: hostLabel, remoteSas: sas }
              : t,
          ),
        );
        selectTab(targetId);
        return;
      }
      const newId = crypto.randomUUID();
      setTabs((prev) => [
        ...prev,
        { id: newId, title: `${t.remote_terminal_tab}：${hostLabel}`, type: "remote-terminal", remoteConnId: connId, remoteHostLabel: hostLabel, remoteSas: sas },
      ]);
      selectTab(newId);
    }}
  />
)}
```

`onCancel` 也要清掉 `reconnectTabId`：不清的話，使用者從工具列按了連線鈕、又按取消，下一次改從 ADD TAB 開新分頁走正常的連線流程，會被誤判成「這是剛才那個分頁要求的重新連線」，錯誤地更新舊分頁而不是開新分頁。

**注意**：`t.remote_terminal_tab` 用來組標題字串沿用既有寫法（見 `TerminalApp.tsx:542` 現有的開新分頁邏輯，這裡對稱地用在更新既有分頁的分支裡），這裡的 `t` 是 `useLocale()` 回傳的 `Translations` 物件，不是被更新的那個 `tab` 物件本身——`t.remote_terminal_tab` 固定是「遠端終端機」這個字串，跟 `tab.title`/`tab.id` 無關，只是剛好都叫 `t`/`tab` 容易看錯，寫程式碼時要注意变量名稱不要搞混。

### 效果：舊連線正確斷線

`RemoteTerminalView` 既有的斷線邏輯（`disconnectTimerRef` 那個 effect，`[connId]` 依賴）在元件被卸載時就會觸發，`setTimeout(0)` 之後呼叫 `shareViewerDisconnect(舊 connId)`——這個機制不需要修改，`key` 改變造成的卸載會自然觸發它，行為跟分頁被關閉時完全一樣。**不需要**額外在 `onConnectClick` 或別的地方手動呼叫一次 `shareViewerDisconnect`。

## 測試

- 新增/更新 `RemoteTerminalView/index.test.tsx`：
  1. 既有所有測試呼叫 `render(<RemoteTerminalView ... />)` 的地方，補上 `onConnectClick={vi.fn()}`（TypeScript 必填 prop，不補會編譯錯誤）。
  2. 新測試：點「連線」按鈕，確認 `onConnectClick` 被呼叫一次。
  3. 新測試：確認「AITerm」文字有套用漸層樣式（檢查那段文字所在的 DOM 節點的 inline style 含 `background` 屬性，不用真的去驗證漸層顏色值本身——CSS 變數在 jsdom 底下量不到實際計算後的顏色）。
- 這個 repo 對 `TerminalApp.tsx` 的測試採「一個主題一個檔案」的慣例（例如既有的 `TerminalApp.routeHintCloseGuard.test.tsx`，測的是完全不同的主題），不是單一一份 `TerminalApp.test.tsx` 涵蓋所有邏輯。這次新增 `TerminalApp.remoteReconnect.test.tsx`，跟進同樣的命名慣例與掛載方式（可以直接參考 `TerminalApp.routeHintCloseGuard.test.tsx` 怎麼 mock/掛載 `TerminalApp` 元件）：
  1. 點某個既有遠端分頁的「連線」按鈕、完成 `ConnectDialog` 流程，確認：`tabs` 陣列長度不變（沒有新增分頁）、原本那個分頁的 `remoteConnId`/`remoteHostLabel`/`remoteSas`/`title` 被更新成新值、`activeId` 停在同一個分頁 id 上。
  2. 從 ADD TAB 開新分頁的既有流程（沒有先點過任何分頁的連線鈕）走一次，確認 `tabs` 陣列長度增加、新分頁被選取——確保這次改動沒有破壞既有的「開新分頁」路徑。
  3. 點連線鈕開啟對話框後按取消，確認 `reconnectTabId` 被清空（可以透過「取消後再走一次 ADD TAB 開新分頁流程，確認真的開了新分頁而不是誤更新到剛才那個分頁」這種行為層面的方式驗證，不需要真的匯出 `reconnectTabId` 這個內部 state 出來測）。

## 範圍界定（不在這次做的）

- 不加回本機工具列的第二列（Terminal/Files 分頁列等價物）——已知、這次沒有完全解決的結構性高度差異，如果之後還是想徹底填滿空白，需要另開一輪設計討論。
- 不做連線切換前的確認對話框。
- 不處理「使用者在 ConnectDialog 開著的時候，原本那個分頁的連線在背景自然結束（例如主控端剛好也在這時候停止分享）」這種時間點重疊的邊角案例——`reconnectTabId` 邏輯只在 `onConnected` 真的觸發時才生效，若原連線在等待期間自然結束，使用者會先看到既有的「連線已結束」畫面，這是既有行為，不因這次改動而變化。

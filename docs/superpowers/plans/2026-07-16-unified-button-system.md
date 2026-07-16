# 統一按鈕視覺系統 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 `src/styles/buttons.css` 共用按鈕樣式系統，並將全 app 23 個檔案、約 40 處按鈕樣式定義遷移過去，統一主要動作/次要/危險/圖示/選中狀態五種語意角色的視覺表現。

**Architecture:** 新增一個全域 CSS 檔案定義 `.aiterm-btn` base class + `--primary`/`--secondary`/`--danger`/`--danger-solid`/`--ghost`/`--icon`/`--sm` modifier。所有面板的按鈕改成疊加這些 class；舊有的面板專屬 class（如 `.ls-start-btn`）保留但只留版面規則（寬度/margin），顏色/圓角/hover 宣告移除。Inline `style={{}}` 按鈕直接改成 `className`。不變更任何按鈕的 onClick/disabled/確認邏輯，純樣式遷移。

**Tech Stack:** React 19 + TypeScript + 純 CSS（無 CSS-in-JS/CSS Modules，全域 class 選擇器）。

**Spec:** `docs/superpowers/specs/2026-07-16-unified-button-system-design.md`

**執行策略備註：** Spec 原先建議「5 大類別各自一個 commit」，但同一檔案（例如 `LoopStudio/styles.css`、`DatabaseAiChat.tsx`）常橫跨 3-4 個類別，若照類別切分會同一檔案被改好幾次、diff 難 review。本計畫改成**每個檔案（或緊密相關的檔案組）一個 task、一個 commit**，同樣達到「小單位、可個別回退」的目的，且更符合「files that change together should live together」。

---

## Task 1: 建立共用按鈕樣式系統

**Files:**
- Create: `src/styles/buttons.css`
- Modify: `src/App.css:1`（開頭加一行 `@import` 或在 `main.tsx` 加 import；本專案其他全域樣式的慣例是在 `App.css` 用 `@import`，採用同慣例）

- [ ] **Step 1: 確認全域 CSS 匯入慣例**

Run: `head -5 /Users/jamesju/Documents/GitHub/AITERM/src/App.css`

檢查 `App.css` 開頭是否已有其他 `@import` 語句；若有，沿用同樣寫法在最前面加一行。若沒有任何 `@import` 前例，改在 `src/main.tsx` 最上方加 `import "./styles/buttons.css";`（在 `import "./App.css"` 相關 import 之前或之後皆可，CSS 全域生效與 import 順序在此無關緊要，因為新 class 不會與既有 class 選擇器衝突）。

- [ ] **Step 2: 建立 `src/styles/buttons.css`**

```css
/* src/styles/buttons.css */
/* 共用按鈕視覺系統 — 詳見 docs/superpowers/specs/2026-07-16-unified-button-system-design.md */

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

/* 危險／刪除 — 靜止時淡底，hover 轉實心紅底 */
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

/* 危險／確認鈕 — 已在二次確認情境中，靜止就是實心紅底（不需 hover 才升級） */
.aiterm-btn--danger-solid {
  background: #ef4444;
  border-color: transparent;
  border-radius: 6px;
  color: #fff;
  font-weight: 600;
  padding: 6px 14px;
}
.aiterm-btn--danger-solid:hover:not(:disabled) {
  background: #dc2626;
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

/* 圓形／方形圖示按鈕，固定尺寸，疊加 --primary/--danger/--ghost 決定顏色 */
.aiterm-btn--icon {
  width: 28px;
  height: 28px;
  padding: 0;
  border-radius: 50%;
  flex-shrink: 0;
  font-size: 12px;
}

/* 尺寸修飾：密集列表用（Settings 卡片操作列等） */
.aiterm-btn--sm {
  padding: 3px 8px;
  font-size: 11px;
  border-radius: 4px;
}
```

- [ ] **Step 3: 加入全域匯入**

若 `App.css` 有 `@import` 前例，於檔案最開頭加：
```css
@import "./styles/buttons.css";
```

否則於 `src/main.tsx` 最上方（第一行）加：
```tsx
import "./styles/buttons.css";
```

- [ ] **Step 4: 驗證建置無誤**

Run: `cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit && npm run lint 2>&1 | grep -i "buttons.css\|main.tsx\|App.css"`
Expected: 無輸出（新檔案不含任何 TS/lint 錯誤來源；`buttons.css` 本身不是 lint 掃描對象）。

- [ ] **Step 5: Commit**

```bash
git add src/styles/buttons.css src/App.css src/main.tsx
git commit -m "$(cat <<'EOF'
feat(styles): add shared aiterm-btn button style system

New src/styles/buttons.css defines base + primary/secondary/danger/
danger-solid/ghost/icon/sm variants, globally imported. Groundwork for
migrating ~40 scattered per-panel button styles onto one shared system.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: LoopStudio 按鈕遷移（`styles.css` + 4 個 `.tsx`）

**Files:**
- Modify: `src/components/LoopStudio/styles.css`
- Modify: `src/components/LoopStudio/index.tsx`
- Modify: `src/components/LoopStudio/SessionPicker.tsx`
- Modify: `src/components/LoopStudio/AgentRoster.tsx`

LoopStudio 一個資料夾涵蓋全部 5 大類別中的 4 種，故合併成一個 task 一次處理完，避免同一堆檔案被拆成好幾個 task 分別碰。

- [ ] **Step 1: `styles.css` — `.ls-start-btn`（主要動作）只留版面規則**

現有（`styles.css:422-447`）：
```css
.ls-start-btn {
  background: var(--accent-gradient, linear-gradient(135deg, #a855f7, #6366f1));
  color: #fff;
  border: none;
  border-radius: 8px;
  padding: 10px 20px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: var(--transition-smooth);
  box-shadow: 0 2px 8px var(--accent-glow, rgba(168,85,247,0.3));
  font-family: inherit;
  width: 100%;
}

.ls-start-btn:hover:not(:disabled) {
  transform: translateY(-1px);
  box-shadow: 0 4px 16px var(--accent-glow, rgba(168,85,247,0.5));
}

.ls-start-btn:disabled {
  opacity: 0.35;
  cursor: not-allowed;
  transform: none;
  box-shadow: none;
}
```

改為（只留 `width: 100%` 這個版面規則，其餘由共用 class 提供）：
```css
.ls-start-btn {
  width: 100%;
}
```

- [ ] **Step 2: `LoopStudio/index.tsx:726` — 疊加共用 class**

現有：
```tsx
              className="ls-start-btn"
```

改為：
```tsx
              className="ls-start-btn aiterm-btn aiterm-btn--primary"
```

- [ ] **Step 3: `styles.css` — `.ls-session-resume-btn`（主要動作，小尺寸）**

現有（`styles.css:1103-1113`）：
```css
.ls-session-resume-btn {
  padding: 3px 8px;
  background: var(--ls-accent, #7c3aed);
  color: #fff;
  border: none;
  border-radius: 4px;
  font-size: 11px;
  cursor: pointer;
}
.ls-session-resume-btn:hover:not(:disabled) { opacity: 0.85; }
.ls-session-resume-btn:disabled { opacity: 0.5; cursor: default; }
```

改為（整個規則刪除 — 沒有獨有版面規則需要保留）：
```css
/* .ls-session-resume-btn removed — replaced by aiterm-btn aiterm-btn--primary aiterm-btn--sm */
```

`SessionPicker.tsx:89` 現有：
```tsx
                    className="ls-session-resume-btn"
```

改為：
```tsx
                    className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
```

- [ ] **Step 4: `styles.css` — `.ls-stop-btn`（危險，靜止淡底/hover 實心）只留版面規則**

現有（`styles.css:449-466`）：
```css
.ls-stop-btn {
  background: rgba(239,68,68,0.15);
  color: #f87171;
  border: 1px solid rgba(239,68,68,0.3);
  border-radius: 8px;
  padding: 10px 20px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: var(--transition-smooth);
  font-family: inherit;
  width: 100%;
}

.ls-stop-btn:hover {
  background: rgba(239,68,68,0.25);
  border-color: rgba(239,68,68,0.5);
}
```

改為：
```css
.ls-stop-btn {
  width: 100%;
}
```

`LoopStudio/index.tsx:735` 現有：
```tsx
              className="ls-stop-btn"
```

改為：
```tsx
              className="ls-stop-btn aiterm-btn aiterm-btn--danger"
```

- [ ] **Step 5: `styles.css` — `.ls-session-delete-btn` / `.ls-clear-all-btn`（危險，圖示/小按鈕）**

現有（`styles.css:1115-1125`）：
```css
.ls-session-delete-btn {
  padding: 3px 7px;
  background: transparent;
  border: 1px solid var(--ls-border, #333);
  border-radius: 4px;
  color: var(--ls-text-muted, #888);
  font-size: 13px;
  cursor: pointer;
  line-height: 1;
}
.ls-session-delete-btn:hover { border-color: #ef4444; color: #ef4444; }
```

改為（改走 ghost + hover 時才變紅，用共用 class 疊加自訂 hover 覆寫）：
```css
.ls-session-delete-btn:hover:not(:disabled) {
  border-color: #ef4444;
  color: #ef4444;
}
```

`SessionPicker.tsx:99` 現有：
```tsx
                  className="ls-session-delete-btn"
```

改為：
```tsx
                  className="ls-session-delete-btn aiterm-btn aiterm-btn--ghost"
```

現有 `.ls-clear-all-btn`（`styles.css:1133-1149`）：
```css
.ls-clear-all-btn {
  font-size: 11px;
  padding: 3px 8px;
  background: transparent;
  border: 1px solid #555;
  border-radius: 4px;
  color: #888;
  cursor: pointer;
  width: 100%;
}

.ls-clear-all-btn:hover:not(:disabled) {
  border-color: #ef4444;
  color: #ef4444;
}

.ls-clear-all-btn:disabled { opacity: 0.4; cursor: not-allowed; }
```

改為：
```css
.ls-clear-all-btn {
  width: 100%;
}
.ls-clear-all-btn:hover:not(:disabled) {
  border-color: #ef4444;
  color: #ef4444;
}
```

`SessionPicker.tsx:118` 現有：
```tsx
                className="ls-clear-all-btn"
```

改為：
```tsx
                className="ls-clear-all-btn aiterm-btn aiterm-btn--secondary"
```

- [ ] **Step 6: `styles.css` — `.ls-clear-confirm-yes` / `.ls-close-discard-btn` / `.ls-close-cancel-btn`（危險確認鈕，靜止即實心紅）**

現有 `.ls-clear-confirm-yes`（`styles.css:1161-1171`）：
```css
.ls-clear-confirm-yes {
  padding: 2px 8px;
  font-size: 11px;
  background: #ef4444;
  border: none;
  border-radius: 4px;
  color: #fff;
  cursor: pointer;
}

.ls-clear-confirm-yes:hover { background: #dc2626; }
```

改為（整個規則刪除）：
```css
/* .ls-clear-confirm-yes removed — replaced by aiterm-btn aiterm-btn--danger-solid aiterm-btn--sm */
```

`SessionPicker.tsx:112` 現有：
```tsx
                &lt;button type="button" className="ls-clear-confirm-yes" onClick={handleClearAll}&gt;{t.ls_session_confirm}&lt;/button&gt;
```

改為：
```tsx
                <button type="button" className="aiterm-btn aiterm-btn--danger-solid aiterm-btn--sm" onClick={handleClearAll}>{t.ls_session_confirm}</button>
```

現有 `.ls-close-cancel-btn` + `.ls-close-discard-btn`（`styles.css:1226-1249`）：
```css
.ls-close-cancel-btn {
  padding: 6px 14px;
  font-size: 12px;
  background: transparent;
  border: 1px solid #555;
  border-radius: 5px;
  color: #ccc;
  cursor: pointer;
}

.ls-close-cancel-btn:hover { border-color: #888; color: #fff; }

.ls-close-discard-btn {
  padding: 6px 14px;
  font-size: 12px;
  background: #ef4444;
  border: none;
  border-radius: 5px;
  color: #fff;
  cursor: pointer;
  font-weight: 500;
}

.ls-close-discard-btn:hover { background: #dc2626; }
```

改為（兩個規則都刪除，class 不再需要）：
```css
/* .ls-close-cancel-btn removed — replaced by aiterm-btn aiterm-btn--secondary */
/* .ls-close-discard-btn removed — replaced by aiterm-btn aiterm-btn--danger-solid */
```

`LoopStudio/index.tsx:407` 與 `415` 現有：
```tsx
                className="ls-close-cancel-btn"
...
                className="ls-close-discard-btn"
```

改為：
```tsx
                className="aiterm-btn aiterm-btn--secondary"
...
                className="aiterm-btn aiterm-btn--danger-solid"
```

- [ ] **Step 7: `styles.css` — `.ls-dir-clear-btn`（圖示，ghost）**

現有（`styles.css:315-328`）：
```css
.ls-dir-clear-btn {
  background: none;
  border: none;
  color: var(--text-muted, #888);
  cursor: pointer;
  font-size: 16px;
  padding: 0 4px;
  line-height: 1;
  flex-shrink: 0;
}

.ls-dir-clear-btn:hover {
  color: var(--danger, #e05050);
}
```

改為：
```css
.ls-dir-clear-btn {
  font-size: 16px;
  padding: 0 4px;
  flex-shrink: 0;
}

.ls-dir-clear-btn:hover:not(:disabled) {
  color: var(--danger, #e05050);
}
```

`LoopStudio/index.tsx:505` 現有：
```tsx
                  className="ls-dir-clear-btn"
```

改為：
```tsx
                  className="ls-dir-clear-btn aiterm-btn aiterm-btn--ghost"
```

- [ ] **Step 8: `styles.css` — 次要按鈕群 `.ls-project-btn` / `.ls-enhance-btn` / `.ls-undo-btn` / `.ls-dir-pick-btn` / `.ls-add-btn` / `.ls-session-delete-btn`（已於 Step 5 處理）**

現有 `.ls-project-btn`（`styles.css:96-118`）：
```css
.ls-project-btn {
  padding: 4px 10px;
  font-size: 11px;
  background: rgba(255,255,255,0.04);
  border: 1px solid var(--border-color, #2a2a2a);
  border-radius: 6px;
  color: var(--text-secondary, #bbb);
  cursor: pointer;
  white-space: nowrap;
  transition: var(--transition-smooth);
  font-family: inherit;
}

.ls-project-btn:hover:not(:disabled) {
  background: rgba(255,255,255,0.08);
  border-color: var(--accent, #a855f7);
  color: var(--text-primary, #fff);
}

.ls-project-btn:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}
```

改為（保留 hover 時邊框變 accent 的獨有效果，其餘交給共用 class）：
```css
.ls-project-btn {
  white-space: nowrap;
}

.ls-project-btn:hover:not(:disabled) {
  border-color: var(--accent, #a855f7);
}
```

`LoopStudio/index.tsx` 4 處使用（`className="ls-project-btn"`，行號約 436, 也在後續 save/save-as/load 按鈕重複相同 className 字串）：先用 grep 找出全部出現位置再逐一替換：

Run: `grep -n 'className="ls-project-btn"' /Users/jamesju/Documents/GitHub/AITERM/src/components/LoopStudio/index.tsx`

對每一處，改為：
```tsx
                className="ls-project-btn aiterm-btn aiterm-btn--secondary"
```

現有 `.ls-enhance-btn`（`styles.css:219-238`）：
```css
.ls-enhance-btn {
  padding: 3px 8px;
  font-size: 11px;
  background: transparent;
  border: 1px solid var(--ls-border, #444);
  border-radius: 4px;
  color: var(--ls-text-muted, #aaa);
  cursor: pointer;
}

.ls-enhance-btn:hover:not(:disabled) {
  background: var(--ls-bg-hover, #2a2a2a);
  border-color: #888;
  color: #fff;
}

.ls-enhance-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
```

改為（整個規則刪除）：
```css
/* .ls-enhance-btn removed — replaced by aiterm-btn aiterm-btn--secondary aiterm-btn--sm */
```

`LoopStudio/index.tsx:534` 與 `AgentRoster.tsx:264` 現有：
```tsx
                className="ls-enhance-btn"
```

改為：
```tsx
                className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
```

現有 `.ls-undo-btn`（`styles.css:240-252`）— 這個保留獨有的橘色（撤銷語意，非危險非次要，是「警示但可逆」的第三種語意，不硬塞進 danger/secondary）：
```css
.ls-undo-btn {
  padding: 3px 8px;
  font-size: 11px;
  background: transparent;
  border: 1px solid #555;
  border-radius: 4px;
  color: #f59e0b;
  cursor: pointer;
}

.ls-undo-btn:hover {
  background: rgba(245, 158, 11, 0.1);
}
```

**不變更**——這是撤銷操作專屬的橘色語意，不屬於本次 5 大類別範疇（不是 primary/secondary/danger/ghost/選中），維持現狀。只加上 `aiterm-btn` base class 以取得一致的 `transition`/`inline-flex` 行為：

`LoopStudio/index.tsx:543` 現有：
```tsx
                  className="ls-undo-btn"
```

改為：
```tsx
                  className="ls-undo-btn aiterm-btn"
```

現有 `.ls-dir-pick-btn`（`styles.css:300-313`）：
```css
.ls-dir-pick-btn {
  background: var(--input-bg, #2a2a2a);
  border: 1px solid var(--border, #333);
  border-radius: 4px;
  padding: 5px 8px;
  cursor: pointer;
  font-size: 14px;
  line-height: 1;
  flex-shrink: 0;
}

.ls-dir-pick-btn:hover:not(:disabled) {
  border-color: var(--accent, #4a9eff);
}
```

改為：
```css
.ls-dir-pick-btn {
  font-size: 14px;
  flex-shrink: 0;
}

.ls-dir-pick-btn:hover:not(:disabled) {
  border-color: var(--accent, #a855f7);
}
```

`LoopStudio/index.tsx:497` 現有：
```tsx
                className="ls-dir-pick-btn"
```

改為：
```tsx
                className="ls-dir-pick-btn aiterm-btn aiterm-btn--secondary"
```

現有 `.ls-add-btn`（`styles.css:489-502`，`.ls-add-btn-wrap` 是定位容器不動）：
```css
.ls-add-btn {
  background: transparent;
  border: 1px solid var(--border, #333);
  border-radius: 4px;
  color: var(--text-muted, #888);
  font-size: 11px;
  padding: 3px 8px;
  cursor: pointer;
}

.ls-add-btn:hover {
  border-color: var(--accent, #4a9eff);
  color: var(--accent, #4a9eff);
}
```

改為（整個規則刪除）：
```css
/* .ls-add-btn removed — replaced by aiterm-btn aiterm-btn--secondary aiterm-btn--sm */
```

`AgentRoster.tsx:186` 現有：
```tsx
            className="ls-add-btn"
```

改為：
```tsx
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
```

- [ ] **Step 9: 驗證**

Run: `cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit && npm run lint 2>&1 | grep -i "LoopStudio"`
Expected: `tsc` 無錯誤；lint 輸出不應出現本次修改檔案的**新增**錯誤（若原本就有 pre-existing warning，維持原樣即可，不強求修掉）。

- [ ] **Step 10: Commit**

```bash
git add src/components/LoopStudio/styles.css src/components/LoopStudio/index.tsx src/components/LoopStudio/SessionPicker.tsx src/components/LoopStudio/AgentRoster.tsx
git commit -m "$(cat <<'EOF'
refactor(loop-studio): migrate buttons to shared aiterm-btn classes

Collapses 11 separately-declared LoopStudio button styles onto the
shared primary/secondary/danger/danger-solid/ghost variants. Per-file
classes keep only their unique layout rules (width, hover accent).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: DatabaseView 按鈕遷移（`DatabaseAiChat.tsx` + `index.tsx`/`.css` + `DatabaseSqlEditor.tsx` + `DatabaseBrowser.tsx`）

**Files:**
- Modify: `src/components/DatabaseView/DatabaseAiChat.tsx`
- Modify: `src/components/DatabaseView/index.tsx`
- Modify: `src/components/DatabaseView/index.css`
- Modify: `src/components/DatabaseView/DatabaseSqlEditor.tsx`
- Modify: `src/components/DatabaseView/DatabaseBrowser.tsx`

- [ ] **Step 1: `DatabaseAiChat.tsx:636-653` — 送出鈕（主要動作，圖示，去除三元運算色彩）**

現有：
```tsx
              <button
                id="db-ai-send-btn"
                onClick={send}
                disabled={!input.trim()}
                style={{
                  width: 26, height: 26, borderRadius: "50%",
                  background: input.trim() ? "var(--accent-gradient)" : "rgba(255,255,255,0.05)",
                  color: input.trim() ? "#fff" : "var(--text-muted)",
                  border: "none", display: "flex", alignItems: "center", justifyContent: "center",
                  cursor: input.trim() ? "pointer" : "not-allowed", fontSize: 11, padding: 0, flexShrink: 0,
                  boxShadow: input.trim() ? "0 2px 6px rgba(0,0,0,0.15)" : "none",
                  transition: "all 0.2s"
                }}
                title={t.db_ai_btn_send}
              >
                ▲
              </button>
```

改為（`disabled` 屬性已經反映 `!input.trim()`，讓 CSS `:disabled` 選擇器接手視覺狀態，不需要 JS 三元）：
```tsx
              <button
                id="db-ai-send-btn"
                onClick={send}
                disabled={!input.trim()}
                className="aiterm-btn aiterm-btn--primary aiterm-btn--icon"
                title={t.db_ai_btn_send}
              >
                ▲
              </button>
```

- [ ] **Step 2: `DatabaseAiChat.tsx:623-634` — 停止鈕（危險，圖示）**

現有：
```tsx
              <button
                onClick={stop}
                style={{
                  width: 26, height: 26, borderRadius: "50%", background: "#ef4444", color: "#fff",
                  border: "none", display: "flex", alignItems: "center", justifyContent: "center",
                  cursor: "pointer", fontSize: 11, padding: 0, flexShrink: 0, boxShadow: "0 2px 6px rgba(0,0,0,0.15)"
                }}
                title={t.db_ai_btn_stop}
              >
                ■
              </button>
```

改為：
```tsx
              <button
                onClick={stop}
                className="aiterm-btn aiterm-btn--danger-solid aiterm-btn--icon"
                title={t.db_ai_btn_stop}
              >
                ■
              </button>
```

- [ ] **Step 3: `DatabaseAiChat.tsx:455-460` — 「New chat」主要動作鈕（原 spec 誤植為「Save as doc」，已更正）**

現有：
```tsx
            <button
              onClick={newChat}
              style={{ background: "#1a2a1e", border: "1px solid #2d4a35", color: "#4ade80", fontSize: 10, borderRadius: 4, padding: "2px 8px", cursor: "pointer" }}
            >
              {t.cdb_ai_history_new_btn}
            </button>
```

改為：
```tsx
            <button
              onClick={newChat}
              className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            >
              {t.cdb_ai_history_new_btn}
            </button>
```

- [ ] **Step 4: `DatabaseAiChat.tsx:587-598` — 第二個「New chat」次要動作鈕**

現有：
```tsx
          {messages.length > 0 && (
            <button
              onClick={newChat}
              title={t.cdb_ai_history_new_btn}
              style={{
                background: "transparent", border: "1px solid #2a2a2a", color: "#555",
                borderRadius: 4, padding: "2px 8px", fontSize: 11, cursor: "pointer",
              }}
            >
              {t.cdb_ai_history_new_btn}
            </button>
          )}
```

改為：
```tsx
          {messages.length > 0 && (
            <button
              onClick={newChat}
              title={t.cdb_ai_history_new_btn}
              className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            >
              {t.cdb_ai_history_new_btn}
            </button>
          )}
```

- [ ] **Step 5: `DatabaseAiChat.tsx:482-490` — session 刪除「×」鈕（圖示 ghost，移除 JS hover hack）**

現有：
```tsx
                <button
                  onClick={(e) => deleteSession(s.id, e)}
                  title={t.cdb_ai_delete_tooltip}
                  style={{ background: "transparent", border: "none", color: "#444", fontSize: 14, cursor: "pointer", padding: "2px 5px", borderRadius: 3, flexShrink: 0, lineHeight: 1 }}
                  onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.color = "#f87171"; (e.currentTarget as HTMLButtonElement).style.background = "#2a1a1a"; }}
                  onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.color = "#444"; (e.currentTarget as HTMLButtonElement).style.background = "transparent"; }}
                >
                  ×
                </button>
```

改為（用 CSS class 取代 JS 手動 hover，hover 時偏紅色沿用 `.aiterm-btn--ghost` 但額外加一個局部 style tag 內的 hover override；由於這是唯一需要「hover 變紅」的 ghost 按鈕實例，直接在同檔案內用一個小 CSS class 覆寫即可，不需要新增到全域 buttons.css）：

先在 `src/components/DatabaseView/index.css` 尾端加入：
```css
.db-ai-session-delete-btn:hover:not(:disabled) {
  color: #f87171 !important;
  background: rgba(239, 68, 68, 0.1) !important;
}
```

再改 `DatabaseAiChat.tsx`：
```tsx
                <button
                  onClick={(e) => deleteSession(s.id, e)}
                  title={t.cdb_ai_delete_tooltip}
                  className="db-ai-session-delete-btn aiterm-btn aiterm-btn--ghost"
                  style={{ color: "#444", fontSize: 14, padding: "2px 5px" }}
                >
                  ×
                </button>
```

- [ ] **Step 6: `DatabaseAiChat.tsx:562-573` — 移除 schema doc「×」鈕（圖示 ghost）**

現有：
```tsx
          {schemaDoc && (
            <button
              onClick={removeSchemaDoc}
              title={t.db_ai_schema_tooltip_remove}
              style={{
                background: "transparent", border: "none", color: "#555",
                fontSize: 12, cursor: "pointer", padding: "2px 4px",
              }}
            >
              ×
            </button>
          )}
```

改為：
```tsx
          {schemaDoc && (
            <button
              onClick={removeSchemaDoc}
              title={t.db_ai_schema_tooltip_remove}
              className="aiterm-btn aiterm-btn--ghost"
              style={{ color: "#555", fontSize: 12, padding: "2px 4px" }}
            >
              ×
            </button>
          )}
```

- [ ] **Step 7: `index.css` — `.db-ai-copy-btn`（次要小按鈕，保留 `--copied` 狀態色）**

現有（`index.css:90-110`）：
```css
.db-ai-copy-btn {
  display: none;
  position: absolute;
  top: 4px;
  right: 4px;
  background: #2a2a2a;
  border: 1px solid #444;
  color: #aaa;
  border-radius: 4px;
  padding: 1px 6px;
  font-size: 12px;
  cursor: pointer;
  line-height: 1.4;
}
.db-ai-answer--copyable:hover .db-ai-copy-btn {
  display: block;
}
.db-ai-copy-btn--copied {
  color: #4ade80;
  border-color: #4ade80;
}
```

改為（只留定位/顯示邏輯，顏色交給共用 class；`--copied` 狀態色維持，是功能狀態不是按鈕變體）：
```css
.db-ai-copy-btn {
  display: none;
  position: absolute;
  top: 4px;
  right: 4px;
}
.db-ai-answer--copyable:hover .db-ai-copy-btn {
  display: block;
}
.db-ai-copy-btn--copied {
  color: #4ade80 !important;
  border-color: #4ade80 !important;
}
```

`DatabaseAiChat.tsx:731` 現有：
```tsx
            className={`db-ai-copy-btn${copied ? " db-ai-copy-btn--copied" : ""}`}
```

改為：
```tsx
            className={`db-ai-copy-btn aiterm-btn aiterm-btn--secondary aiterm-btn--sm${copied ? " db-ai-copy-btn--copied" : ""}`}
```

- [ ] **Step 8: `index.tsx:81-97` — retry / reconnect 按鈕（次要）**

現有：
```tsx
            <button
              onClick={() => setConnectError(null)}
              style={{ background: "#1a1a1a", border: "1px solid #3a3a3a", color: "#ccc", borderRadius: 4, padding: "6px 14px", cursor: "pointer" }}
            >
              {t.db_btn_retry}
            </button>
```
```tsx
            <button
              onClick={() => { setConnectError(null); }}
              style={{ marginTop: 12, background: "#1a1a1a", border: "1px solid #3a3a3a", color: "#ccc", borderRadius: 4, padding: "6px 14px", cursor: "pointer" }}
            >
              {t.db_btn_reconnect}
            </button>
```

改為：
```tsx
            <button
              onClick={() => setConnectError(null)}
              className="aiterm-btn aiterm-btn--secondary"
            >
              {t.db_btn_retry}
            </button>
```
```tsx
            <button
              onClick={() => { setConnectError(null); }}
              style={{ marginTop: 12 }}
              className="aiterm-btn aiterm-btn--secondary"
            >
              {t.db_btn_reconnect}
            </button>
```

- [ ] **Step 9: `DatabaseSqlEditor.tsx:53-59` — 「Run」按鈕（主要動作）**

現有：
```tsx
          <button
            onClick={run}
            disabled={running}
            style={{ background: "#1e3a2e", border: "1px solid #34d399", color: "#34d399", borderRadius: 4, padding: "4px 14px", cursor: "pointer", fontSize: 12 }}
          >
            {running ? t.db_sql_running : t.db_sql_btn_run}
          </button>
```

改為：
```tsx
          <button
            onClick={run}
            disabled={running}
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
          >
            {running ? t.db_sql_running : t.db_sql_btn_run}
          </button>
```

- [ ] **Step 10: `DatabaseBrowser.tsx:113-175` — mode/page 按鈕（次要，保留 active 底線）**

現有（`DatabaseBrowser.tsx:208-210`）：
```tsx
const modeBtn: CSSProperties = { background: "transparent", border: "none", color: "#888", fontSize: 12, padding: "4px 12px", cursor: "pointer" };
const modeBtnActive: CSSProperties = { color: "#34d399", borderBottom: "2px solid #34d399" };
const pageBtn: CSSProperties = { background: "#1a1a1a", border: "1px solid #2a2a2a", color: "#888", fontSize: 11, padding: "2px 10px", borderRadius: 3, cursor: "pointer" };
```

改為（`modeBtnActive` 的底線改用主題色，`pageBtn` 顏色交給共用 secondary class，`modeBtn` 是無框 tab 樣式維持自訂但改用主題色 active）：
```tsx
const modeBtnActive: CSSProperties = { color: "var(--accent, #a855f7)", borderBottom: "2px solid var(--accent, #a855f7)" };
```

（`modeBtn` 常數整個刪除，`pageBtn` 常數整個刪除，改用 className）

`DatabaseBrowser.tsx:113-114` 現有：
```tsx
        <button onClick={() => switchMode("data")} style={{ ...modeBtn, ...(viewMode === "data" ? modeBtnActive : {}) }}>{t.db_browser_mode_data}</button>
        <button onClick={() => switchMode("structure")} style={{ ...modeBtn, ...(viewMode === "structure" ? modeBtnActive : {}) }}>{t.db_browser_mode_structure}</button>
```

改為：
```tsx
        <button onClick={() => switchMode("data")} className="aiterm-btn aiterm-btn--ghost" style={viewMode === "data" ? modeBtnActive : undefined}>{t.db_browser_mode_data}</button>
        <button onClick={() => switchMode("structure")} className="aiterm-btn aiterm-btn--ghost" style={viewMode === "structure" ? modeBtnActive : undefined}>{t.db_browser_mode_structure}</button>
```

`DatabaseBrowser.tsx:173,175` 現有：
```tsx
{page > 0 && <button onClick={() => onPageChange(page - 1)} style={pageBtn}>{t.db_prev_page}</button>}
{result.rows.length === pageSize && <button onClick={() => onPageChange(page + 1)} style={pageBtn}>{t.db_next_page}</button>}
```

改為：
```tsx
{page > 0 && <button onClick={() => onPageChange(page - 1)} className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm">{t.db_prev_page}</button>}
{result.rows.length === pageSize && <button onClick={() => onPageChange(page + 1)} className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm">{t.db_next_page}</button>}
```

- [ ] **Step 11: `index.css` — `.db-view__subtab--active` 選中狀態改用主題色**

現有（`index.css:38-41`）：
```css
.db-view__subtab--active {
  color: #34d399;
  border-bottom-color: #34d399;
}
```

改為：
```css
.db-view__subtab--active {
  color: var(--accent, #a855f7);
  border-bottom-color: var(--accent, #a855f7);
}
```

**注意：** `index.css:31-36` 的 `.db-view__subtab.aiterm-agent-toggle--on`（固定綠色狀態鈕）**不變更**，維持固定綠色，這是「連線狀態」語彙，不是「選中」語彙。

- [ ] **Step 12: 驗證**

Run: `cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit && npm run lint 2>&1 | grep -i "DatabaseView\|DatabaseAiChat\|DatabaseSqlEditor\|DatabaseBrowser"`
Expected: `tsc` 無錯誤；不新增 lint 錯誤。

- [ ] **Step 13: Commit**

```bash
git add src/components/DatabaseView/
git commit -m "$(cat <<'EOF'
refactor(database-view): migrate buttons to shared aiterm-btn classes

Removes dynamic ternary-based inline colors on the AI chat send button
in favor of the CSS :disabled state, migrates retry/run/copy/pagination
buttons, and switches the subtab active-selection color to var(--accent)
(leaving the fixed-green connection-status toggle untouched).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Settings — AboutPage / ProvidersPage / McpServersPage / McpMarketplaceTab

**Files:**
- Modify: `src/components/Settings/AboutPage.css`, `AboutPage.tsx`
- Modify: `src/components/Settings/ProvidersPage.css`, `ProvidersPage.tsx`
- Modify: `src/components/Settings/McpServersPage.css`, `McpServersPage.tsx`
- Modify: `src/components/Settings/McpMarketplaceTab.tsx`

- [ ] **Step 1: `AboutPage.css` — `.about-btn`（主要動作）**

現有（`AboutPage.css:49-67`）：
```css
.about-btn {
  background: #1e2a42;
  border: none;
  border-radius: 6px;
  color: #7aadff;
  cursor: pointer;
  font-family: inherit;
  font-size: 13px;
  padding: 8px 18px;
}

.about-btn:hover {
  background: #263450;
}

.about-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
```

改為（整個規則刪除）：
```css
/* .about-btn removed — replaced by aiterm-btn aiterm-btn--primary */
```

`AboutPage.tsx:86` 與 `89` 現有：
```tsx
        <button className="about-btn" onClick={handleGitHub}>
          {t.about_github}
        </button>
        <button
          className="about-btn"
          onClick={handleCheckUpdates}
          disabled={updateStatus === "checking" || version === "…"}
        >
```

改為：
```tsx
        <button className="aiterm-btn aiterm-btn--primary" onClick={handleGitHub}>
          {t.about_github}
        </button>
        <button
          className="aiterm-btn aiterm-btn--primary"
          onClick={handleCheckUpdates}
          disabled={updateStatus === "checking" || version === "…"}
        >
```

- [ ] **Step 2: `ProvidersPage.css` — `.btn-add`（主要動作）、`.provider-card-actions button`（次要）、`.btn-danger`（危險）**

現有（`ProvidersPage.css:19-31`）：
```css
.btn-add {
  background: #2d5aab;
  border: 1px solid #3d6ac0;
  border-radius: 5px;
  color: #fff;
  cursor: pointer;
  font-size: 13px;
  padding: 7px 14px;
}

.btn-add:hover {
  background: #3265be;
}
```

改為：
```css
/* .btn-add removed — replaced by aiterm-btn aiterm-btn--primary */
```

`ProvidersPage.tsx:79` 現有：
```tsx
          className="btn-add"
```

改為：
```tsx
          className="aiterm-btn aiterm-btn--primary"
```

現有（`ProvidersPage.css:110-143`）：
```css
.provider-card-actions {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}

.provider-card-actions button {
  background: #252525;
  border: 1px solid #3a3a3a;
  border-radius: 4px;
  color: #ccc;
  cursor: pointer;
  font-size: 12px;
  padding: 5px 10px;
  white-space: nowrap;
}

.provider-card-actions button:hover:not(:disabled) {
  background: #2e2e2e;
}

.provider-card-actions button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.provider-card-actions .btn-danger {
  color: #f87171;
}

.provider-card-actions .btn-danger:hover:not(:disabled) {
  background: rgba(220, 50, 50, 0.15);
  border-color: rgba(220, 50, 50, 0.4);
}
```

改為（保留容器排版，按鈕本體規則刪除，`.btn-danger` 改成獨立的 hover 覆寫，因為它現在會和 `.aiterm-btn--secondary` 疊加）：
```css
.provider-card-actions {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}

.provider-card-actions .btn-danger {
  color: #f87171;
  border-color: rgba(248, 113, 113, 0.4);
}

.provider-card-actions .btn-danger:hover:not(:disabled) {
  background: rgba(220, 50, 50, 0.15);
  border-color: rgba(220, 50, 50, 0.4);
}
```

`ProvidersPage.tsx` — 這些按鈕目前完全沒有 `className`（純靠 `.provider-card-actions button` 後代選擇器），需要逐一加上 `className`：

Run: `grep -n '<button' /Users/jamesju/Documents/GitHub/AITERM/src/components/Settings/ProvidersPage.tsx | sed -n '1,20p'`

對 `.provider-card-actions` 容器內的每個 `<button>`（test / set-default / edit / cancel），加上：
```tsx
                className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
```

對 `className="btn-danger"` 的兩處（`ProvidersPage.tsx:141`, `149`），改為：
```tsx
                    className="btn-danger aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
```

- [ ] **Step 3: `McpServersPage.css` — `.mcp-btn-sm`（次要小按鈕）、`.mcp-btn-sm.danger`（危險）**

現有（`McpServersPage.css:69-79`）：
```css
.mcp-btn-sm {
  padding: 3px 10px;
  font-size: 12px;
  border-radius: 4px;
  border: 1px solid #333;
  background: transparent;
  color: #ccc;
  cursor: pointer;
}
.mcp-btn-sm:hover { background: rgba(255,255,255,0.06); color: #fff; }
.mcp-btn-sm.danger:hover { background: #2e0f0f; color: #f87171; border-color: #7f1d1d; }
```

改為：
```css
.mcp-btn-sm.danger { color: #f87171; border-color: rgba(248, 113, 113, 0.4); }
.mcp-btn-sm.danger:hover:not(:disabled) { background: #2e0f0f; color: #f87171; border-color: #7f1d1d; }
```

`McpServersPage.tsx` 所有 `className="mcp-btn-sm"` 與 `className="mcp-btn-sm danger"`（行 137, 175, 180, 183, 188, 269）改為：
```tsx
className="mcp-btn-sm aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
```
與
```tsx
className="mcp-btn-sm danger aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
```

- [ ] **Step 4: `McpServersPage.css` — `.mcp-tab-btn.active` 選中狀態改主題色**

現有（`McpServersPage.css:126-145`）：
```css
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

改為（只改 `.active` 顏色，其餘 tab 底色/hover 樣式維持不動——這是獨立於按鈕系統的 tab 樣式）：
```css
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
  color: var(--accent, #a855f7);
  border-bottom-color: var(--accent, #a855f7);
}
```

（`.tsx` 不需改動，class 名稱沒變。）

- [ ] **Step 5: `McpMarketplaceTab.tsx` — sort toggle / install / intro 連結（次要，inline style 改 className）**

現有（`McpMarketplaceTab.tsx:157-170`，sort toggle）：
```tsx
          <button
            onClick={() => setSortByDownloads(prev => !prev)}
            style={{
              background: sortByDownloads ? "#1e3a5f" : "none",
              border: `1px solid ${sortByDownloads ? "#3b82f6" : "#2a2a2a"}`,
              borderRadius: 4,
              color: sortByDownloads ? "#93c5fd" : "#666",
              cursor: "pointer",
              fontSize: 11,
              padding: "3px 8px",
            }}
          >
            ↓ {t.mcp_marketplace_sort_downloads}
          </button>
```

改為（選中狀態改用主題色 `var(--accent-dim)`/`var(--accent)`，維持「按下去有沒有啟用」的視覺區分）：
```tsx
          <button
            onClick={() => setSortByDownloads(prev => !prev)}
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            style={sortByDownloads ? { background: "var(--accent-dim, rgba(168,85,247,0.15))", borderColor: "var(--accent, #a855f7)", color: "var(--accent, #a855f7)" } : undefined}
          >
            ↓ {t.mcp_marketplace_sort_downloads}
          </button>
```

現有（`McpMarketplaceTab.tsx:207-222`，install 按鈕，狀態顏色保留不動——這是安裝結果狀態，非按鈕變體）：
```tsx
                <button
                  onClick={() => handleInstall(server)}
                  disabled={isDisabled}
                  style={{
                    background: state.status === "success" ? "#166534" : "#2a2a2a",
                    border: "1px solid #3a3a3a",
                    borderRadius: 4,
                    color: state.status === "error" ? "#f87171" : "#ccc",
                    cursor: isDisabled ? "default" : "pointer",
                    fontSize: 12,
                    padding: "4px 10px",
                    whiteSpace: "nowrap",
                  }}
                >
                  {getButtonLabel(state.status)}
                </button>
```

改為（保留狀態色邏輯，只把基礎樣式交給共用 class）：
```tsx
                <button
                  onClick={() => handleInstall(server)}
                  disabled={isDisabled}
                  className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                  style={{
                    background: state.status === "success" ? "#166534" : undefined,
                    color: state.status === "error" ? "#f87171" : undefined,
                    whiteSpace: "nowrap",
                  }}
                >
                  {getButtonLabel(state.status)}
                </button>
```

現有（`McpMarketplaceTab.tsx:223-237`，intro 連結按鈕）：
```tsx
                <button
                  onClick={() => openUrl(server.homepage ?? `https://www.npmjs.com/package/${server.qualifiedName}`).catch(console.error)}
                  style={{
                    background: "none",
                    border: "1px solid #2a2a2a",
                    borderRadius: 4,
                    color: "#666",
                    cursor: "pointer",
                    fontSize: 11,
                    padding: "3px 10px",
                    whiteSpace: "nowrap",
                  }}
                >
                  {t.mcp_marketplace_intro}
                </button>
```

改為：
```tsx
                <button
                  onClick={() => openUrl(server.homepage ?? `https://www.npmjs.com/package/${server.qualifiedName}`).catch(console.error)}
                  className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                  style={{ whiteSpace: "nowrap" }}
                >
                  {t.mcp_marketplace_intro}
                </button>
```

（分頁 prev/next 按鈕已用 `className="mcp-btn-sm"`，隨 Step 3 一併帶到。）

- [ ] **Step 6: 驗證**

Run: `cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit && npm run lint 2>&1 | grep -i "AboutPage\|ProvidersPage\|McpServersPage\|McpMarketplaceTab"`
Expected: `tsc` 無錯誤；不新增 lint 錯誤。

- [ ] **Step 7: Commit**

```bash
git add src/components/Settings/AboutPage.css src/components/Settings/AboutPage.tsx src/components/Settings/ProvidersPage.css src/components/Settings/ProvidersPage.tsx src/components/Settings/McpServersPage.css src/components/Settings/McpServersPage.tsx src/components/Settings/McpMarketplaceTab.tsx
git commit -m "$(cat <<'EOF'
refactor(settings): migrate About/Providers/MCP buttons to aiterm-btn

Also switches .mcp-tab-btn.active from a hardcoded green to
var(--accent) to match the selected-state convention used elsewhere.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Settings — EnterprisePage / TerminalApp（Enterprise 相關按鈕、skill-toast）

**Files:**
- Modify: `src/components/Settings/EnterprisePage.tsx`
- Modify: `src/components/TerminalApp.tsx`

- [ ] **Step 1: `EnterprisePage.tsx:156-166` — Register/Re-register（主要動作）**

現有：
```tsx
        <button
          onClick={handleRegister}
          disabled={status === "registering" || !serverUrl.trim() || !deviceName.trim()}
          style={{
            marginTop: 4, padding: "8px 20px", background: "#2a5a9a", color: "#fff",
            border: "none", borderRadius: 4, cursor: "pointer", fontWeight: 600,
            fontSize: 13, opacity: (status === "registering" || !serverUrl.trim() || !deviceName.trim()) ? 0.5 : 1,
          }}
        >
          {status === "registering" ? t.enterprise_registering : isAlreadyRegistered ? t.enterprise_reregister : t.enterprise_register}
        </button>
```

改為（`disabled` 已由屬性驅動，`opacity` 交給 `.aiterm-btn--primary:disabled`）：
```tsx
        <button
          onClick={handleRegister}
          disabled={status === "registering" || !serverUrl.trim() || !deviceName.trim()}
          className="aiterm-btn aiterm-btn--primary"
          style={{ marginTop: 4 }}
        >
          {status === "registering" ? t.enterprise_registering : isAlreadyRegistered ? t.enterprise_reregister : t.enterprise_register}
        </button>
```

- [ ] **Step 2: `EnterprisePage.tsx:228-234` — Install service（主要動作）**

現有：
```tsx
        <button
          onClick={install}
          disabled={installing}
          style={{ padding: "6px 14px", background: "#2a5a9a", border: "none", color: "#fff", borderRadius: 4, cursor: "pointer", fontSize: 13, opacity: installing ? 0.5 : 1 }}
        >
          {installing ? t.enterprise_installing : t.enterprise_install_service}
        </button>
```

改為：
```tsx
        <button
          onClick={install}
          disabled={installing}
          className="aiterm-btn aiterm-btn--primary"
        >
          {installing ? t.enterprise_installing : t.enterprise_install_service}
        </button>
```

- [ ] **Step 3: `EnterprisePage.tsx:222-227` — Preview config（次要）**

現有：
```tsx
        <button
          onClick={preview}
          style={{ padding: "6px 14px", background: "#222", border: "1px solid #444", color: "#e0e0e0", borderRadius: 4, cursor: "pointer", fontSize: 13 }}
        >
          {t.enterprise_preview_config}
        </button>
```

改為：
```tsx
        <button
          onClick={preview}
          className="aiterm-btn aiterm-btn--secondary"
        >
          {t.enterprise_preview_config}
        </button>
```

- [ ] **Step 4: `TerminalApp.tsx:414-425` — Enterprise「Execute」按鈕（主要動作）**

現有：
```tsx
            <button
              onClick={() => {
                enterpriseAcceptTask(pendingTask).catch(console.error);
                setPendingTask(null);
              }}
              style={{
                flex: 1, padding: "6px 0", background: "#2a7a4a", color: "#fff",
                border: "none", borderRadius: 4, cursor: "pointer", fontWeight: 600,
              }}
            >
              Execute
            </button>
```

改為：
```tsx
            <button
              onClick={() => {
                enterpriseAcceptTask(pendingTask).catch(console.error);
                setPendingTask(null);
              }}
              className="aiterm-btn aiterm-btn--primary"
              style={{ flex: 1 }}
            >
              Execute
            </button>
```

- [ ] **Step 5: `TerminalApp.tsx:426-435` — Enterprise「Reject」按鈕（危險 — 語意修正：原為灰色外框，改為明確的危險語彙）**

現有：
```tsx
            <button
              onClick={() => {
                enterpriseRejectTask(pendingTask.task_id).catch(console.error);
                setPendingTask(null);
              }}
              style={{
                flex: 1, padding: "6px 0", background: "transparent", color: "#aaa",
                border: "1px solid #555", borderRadius: 4, cursor: "pointer",
              }}
            >
              Reject
            </button>
```

改為：
```tsx
            <button
              onClick={() => {
                enterpriseRejectTask(pendingTask.task_id).catch(console.error);
                setPendingTask(null);
              }}
              className="aiterm-btn aiterm-btn--danger"
              style={{ flex: 1 }}
            >
              Reject
            </button>
```

- [ ] **Step 6: `TerminalApp.tsx:452-457` — skill-toast 關閉鈕（圖示 ghost）**

現有：
```tsx
          <button
            onClick={() => setSkillToast(null)}
            style={{ marginLeft: 12, background: "none", border: "none", color: "#888", cursor: "pointer" }}
          >
            ✕
          </button>
```

改為：
```tsx
          <button
            onClick={() => setSkillToast(null)}
            className="aiterm-btn aiterm-btn--ghost"
            style={{ marginLeft: 12 }}
          >
            ✕
          </button>
```

- [ ] **Step 7: 驗證**

Run: `cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit && npm run lint 2>&1 | grep -i "EnterprisePage\|TerminalApp"`
Expected: `tsc` 無錯誤；不新增 lint 錯誤。

- [ ] **Step 8: Commit**

```bash
git add src/components/Settings/EnterprisePage.tsx src/components/TerminalApp.tsx
git commit -m "$(cat <<'EOF'
refactor(enterprise): migrate task accept/reject buttons to aiterm-btn

Reject button now uses the danger variant (was a neutral gray outline)
to visually distinguish it as the irreversible-decline action.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: DatabaseConnectionsPage / VcsConnectionsPage（消除重複 `btnStyle`）

**Files:**
- Modify: `src/components/Settings/DatabaseConnectionsPage.tsx`
- Modify: `src/components/Settings/VcsConnectionsPage.tsx`

兩檔案的 `btnStyle` 常數與其所有用法逐字相同，一起處理。

- [ ] **Step 1: `DatabaseConnectionsPage.tsx:233-236` — 刪除 `btnStyle` 常數**

現有：
```tsx
const btnStyle: CSSProperties = {
  background: "transparent", border: "1px solid #3a3a3a", color: "#ccc",
  borderRadius: 4, padding: "4px 12px", cursor: "pointer", fontSize: 12,
};
```

整段刪除（不再需要）。

- [ ] **Step 2: `DatabaseConnectionsPage.tsx` — 「+ Add」按鈕（主要動作，line 87-92）**

現有：
```tsx
          <button
            onClick={() => { setForm(EMPTY_FORM); setShowForm(true); setTestStatus("idle"); }}
            style={{ background: "#1e3a2e", border: "1px solid #34d399", color: "#34d399", borderRadius: 5, padding: "6px 14px", cursor: "pointer", fontSize: 13 }}
          >
            {t.add_connection}
          </button>
```

改為：
```tsx
          <button
            onClick={() => { setForm(EMPTY_FORM); setShowForm(true); setTestStatus("idle"); }}
            className="aiterm-btn aiterm-btn--primary"
          >
            {t.add_connection}
          </button>
```

- [ ] **Step 3: `DatabaseConnectionsPage.tsx` — edit/cancel/test（次要，line 112, 115, 216, 222）**

現有：
```tsx
                <button onClick={() => handleEdit(conn)} style={btnStyle}>{t.edit}</button>
...
                    <button onClick={() => setConfirmingDelete(null)} style={btnStyle}>{t.cancel}</button>
...
          <button onClick={handleTest} disabled={testStatus === "testing"} style={btnStyle}>
...
          <button onClick={() => setShowForm(false)} style={btnStyle}>{t.cancel}</button>
```

改為（4 處全部把 `style={btnStyle}` 換成 `className`）：
```tsx
                <button onClick={() => handleEdit(conn)} className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm">{t.edit}</button>
...
                    <button onClick={() => setConfirmingDelete(null)} className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm">{t.cancel}</button>
...
          <button onClick={handleTest} disabled={testStatus === "testing"} className="aiterm-btn aiterm-btn--secondary">
...
          <button onClick={() => setShowForm(false)} className="aiterm-btn aiterm-btn--secondary">{t.cancel}</button>
```

- [ ] **Step 4: `DatabaseConnectionsPage.tsx` — delete（危險，line 116, 119）**

現有：
```tsx
                    <button onClick={() => handleDelete(conn.id)} style={{ ...btnStyle, color: "#f87171", borderColor: "#f87171" }}>{t.delete}?</button>
...
                  <button onClick={() => setConfirmingDelete(conn.id)} style={{ ...btnStyle, color: "#f87171", borderColor: "#f87171" }}>{t.delete}</button>
```

改為：
```tsx
                    <button onClick={() => handleDelete(conn.id)} className="aiterm-btn aiterm-btn--danger-solid aiterm-btn--sm">{t.delete}?</button>
...
                  <button onClick={() => setConfirmingDelete(conn.id)} className="aiterm-btn aiterm-btn--danger aiterm-btn--sm">{t.delete}</button>
```

（第一個是已進入二次確認的「確定刪除？」，用 `--danger-solid`；第二個是尚未確認的初始「刪除」入口，用 `--danger`。）

- [ ] **Step 5: `DatabaseConnectionsPage.tsx:223` — save（主要動作，原本綠色外框已存在，統一成漸層）**

現有：
```tsx
          <button onClick={handleSave} disabled={saving} style={{ ...btnStyle, background: "#1e3a2e", borderColor: "#34d399", color: "#34d399" }}>
```

改為：
```tsx
          <button onClick={handleSave} disabled={saving} className="aiterm-btn aiterm-btn--primary">
```

- [ ] **Step 6: 對 `VcsConnectionsPage.tsx` 重複同樣的 5 個 Step**（行號分別對應：`btnStyle` 於 221-224；「+ Add」於 99-104；edit/cancel/test 於 124, 127, 204, 210；delete 於 128, 131；save 於 211）

依照 Step 1-5 相同的規則，把 `VcsConnectionsPage.tsx` 對應行做一模一樣的替換（差異只有 i18n 字串 key，例如 `t.vcs_edit_btn` 而非 `t.edit`，`t.vcs_delete_confirm_btn`/`t.vcs_delete_btn` 而非 `t.delete}?`/`t.delete`，`t.vcs_add_conn` 而非 `t.add_connection`）。

- [ ] **Step 7: 驗證**

Run: `cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit && npm run lint 2>&1 | grep -i "DatabaseConnectionsPage\|VcsConnectionsPage"`
Expected: `tsc` 無錯誤（特別確認 `CSSProperties` import 若因 `btnStyle` 刪除而不再被使用，需一併移除該 import，避免 unused-import 錯誤）；不新增 lint 錯誤。

Run: `grep -n "CSSProperties" /Users/jamesju/Documents/GitHub/AITERM/src/components/Settings/DatabaseConnectionsPage.tsx /Users/jamesju/Documents/GitHub/AITERM/src/components/Settings/VcsConnectionsPage.tsx`

若 `labelStyle`/`inputStyle` 等其他常數仍在使用 `CSSProperties`，保留 import；若該檔案唯一使用者就是已刪除的 `btnStyle`，移除 `import type { CSSProperties } from "react";`（或其等效 import 語句）。

- [ ] **Step 8: Commit**

```bash
git add src/components/Settings/DatabaseConnectionsPage.tsx src/components/Settings/VcsConnectionsPage.tsx
git commit -m "$(cat <<'EOF'
refactor(settings): dedupe DatabaseConnectionsPage/VcsConnectionsPage
button styles onto aiterm-btn

Both files had a byte-identical local btnStyle constant; removed in
favor of the shared class system.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: DocConverter / SettingsView（選中狀態）/ FileExplorer

**Files:**
- Modify: `src/components/DocConverter/DocConverterView.css`, `DocConverterView.tsx`
- Modify: `src/components/FileExplorer/FileExplorer.css`

- [ ] **Step 1: `DocConverterView.css` — `.doc-converter__btn--primary` / `--secondary`**

現有（`DocConverterView.css:136-159`）：
```css
.doc-converter__btn {
  border-radius: 6px;
  padding: 7px 16px;
  font-size: 12px;
  cursor: pointer;
  border: 1px solid;
}

.doc-converter__btn--primary {
  background: #1e3a2e;
  border-color: #34d399;
  color: #34d399;
}

.doc-converter__btn--secondary {
  background: transparent;
  border-color: #2a2a2a;
  color: #888;
}

.doc-converter__btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
```

改為（`.doc-converter__btn` base 與兩個 modifier 全部刪除，直接用共用 class）：
```css
/* .doc-converter__btn / --primary / --secondary removed — replaced by aiterm-btn variants */
```

`DocConverterView.tsx` 4 處使用：

`:257-263`：
```tsx
          <button
            className="doc-converter__btn doc-converter__btn--primary"
            onClick={normalizeWithAi}
            disabled={!selectedProviderId}
          >
            {t.dc_normalize_btn}
          </button>
```
改為：
```tsx
          <button
            className="aiterm-btn aiterm-btn--primary"
            onClick={normalizeWithAi}
            disabled={!selectedProviderId}
          >
            {t.dc_normalize_btn}
          </button>
```

`:264-269`、`:271-276`、`:290-295`（皆為 `doc-converter__btn--secondary`）改為 `className="aiterm-btn aiterm-btn--secondary"`。

`:305-311`（`--secondary` + 額外 inline `style={{ fontSize: 11, padding: "2px 8px" }}`）改為：
```tsx
            <button
              className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
              onClick={downloadMd}
            >
              {t.dc_download_btn}
            </button>
```
（`aiterm-btn--sm` 已經提供對應的縮小 padding/font-size，原本的 inline style override 移除。）

- [ ] **Step 2: `FileExplorer.css` — `.fe-btn`（次要）/ `.fe-btn--active`（選中，改主題色）**

現有（`FileExplorer.css:23-55`）：
```css
.fe-btn {
  background: none;
  border: 1px solid #333;
  border-radius: 4px;
  color: #aaa;
  cursor: pointer;
  font-size: 13px;
  line-height: 1;
  padding: 3px 7px;
  flex-shrink: 0;
  transition: background 0.15s, color 0.15s;
}

.fe-btn:hover:not(:disabled) {
  background: #252525;
  color: #fff;
}

.fe-btn:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.fe-btn--active {
  background: #1e3a5f;
  color: #63b3ed;
  border-color: #1e3a5f;
}

.fe-btn--dot {
  font-weight: bold;
  font-family: monospace;
}
```

改為（`.fe-btn` 保留但精簡成版面規則，顏色/hover/disabled 交給共用 class；`--active` 改主題色）：
```css
.fe-btn {
  font-size: 13px;
  line-height: 1;
  padding: 3px 7px;
  flex-shrink: 0;
}

.fe-btn--active {
  background: var(--accent-dim, rgba(168, 85, 247, 0.15));
  color: var(--accent, #a855f7);
  border-color: var(--accent, #a855f7);
}

.fe-btn--dot {
  font-weight: bold;
  font-family: monospace;
}
```

`FileExplorer.tsx` 所有 `className="fe-btn"` / `` `fe-btn fe-btn--dot ${showDotfiles ? "fe-btn--active" : ""}` `` 改為疊加共用 class：

Run: `grep -n 'className.*fe-btn' /Users/jamesju/Documents/GitHub/AITERM/src/components/FileExplorer/FileExplorer.tsx`

對純 `className="fe-btn"` 的每一處，改為：
```tsx
        <button className="fe-btn aiterm-btn aiterm-btn--secondary" ...>
```

對 `` className={`fe-btn fe-btn--dot ${showDotfiles ? "fe-btn--active" : ""}`} `` 這處（line 209-215），改為：
```tsx
          className={`fe-btn fe-btn--dot aiterm-btn aiterm-btn--secondary ${showDotfiles ? "fe-btn--active" : ""}`}
```

- [ ] **Step 3: 驗證**

Run: `cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit && npm run lint 2>&1 | grep -i "DocConverter\|FileExplorer"`
Expected: `tsc` 無錯誤；不新增 lint 錯誤。

- [ ] **Step 4: Commit**

```bash
git add src/components/DocConverter/DocConverterView.css src/components/DocConverter/DocConverterView.tsx src/components/FileExplorer/FileExplorer.css
git commit -m "$(cat <<'EOF'
refactor(doc-converter,file-explorer): migrate buttons to aiterm-btn

fe-btn--active now uses var(--accent) instead of a hardcoded blue,
matching the selected-state convention used elsewhere.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: TerminalView（區塊按鈕、搜尋按鈕、subtab 選中狀態）

**Files:**
- Modify: `src/components/TerminalView.css`

- [ ] **Step 1: `.aiterm-block-btn` / `.aiterm-block-btn-ai`（次要）**

現有（`TerminalView.css:145-176`）：
```css
.aiterm-block-btn {
  background: #1e1e2e;
  color: #ccc;
  border: 1px solid #333;
  border-radius: 4px;
  padding: 2px 8px;
  font-size: 11px;
  font-family: "Cascadia Mono", Consolas, monospace;
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s;
  white-space: nowrap;
}

.aiterm-block-btn:hover {
  background: #2a2a3e;
  border-color: #555;
}

.aiterm-block-btn-ai {
  border-color: #c084fc;
  color: #c084fc;
}

.aiterm-block-btn-ai:hover {
  background: rgba(192, 132, 252, 0.15);
}

.aiterm-block-btn.aiterm-agent-toggle--on {
  background: #0e2a0e;
  border-color: #4ade80;
  color: #4ade80;
}
```

改為（`.aiterm-block-btn` 保留獨有的等寬字體與 `white-space`，底色/邊框/hover 交給共用 class；`-ai` 變體與 `.aiterm-agent-toggle--on` **維持不動**，前者是 AI 專屬的紫色語彙、後者是固定綠色的連線狀態，都不屬於本次 5 大類別）：
```css
.aiterm-block-btn {
  font-family: "Cascadia Mono", Consolas, monospace;
  font-size: 11px;
  padding: 2px 8px;
  white-space: nowrap;
}

.aiterm-block-btn-ai {
  border-color: #c084fc;
  color: #c084fc;
}

.aiterm-block-btn-ai:hover {
  background: rgba(192, 132, 252, 0.15);
}

.aiterm-block-btn.aiterm-agent-toggle--on {
  background: #0e2a0e;
  border-color: #4ade80;
  color: #4ade80;
}
```

`TerminalView.tsx` 所有 `className="aiterm-block-btn"` / `className="aiterm-block-btn aiterm-block-btn-ai"` / `` className={`aiterm-block-btn ${isRemoteEnabled ? 'aiterm-agent-toggle--on' : ''}`} `` （行 827, 838, 978, 990, 1004）改為疊加共用 secondary class：

```tsx
            className={`aiterm-block-btn aiterm-btn aiterm-btn--secondary ${isRemoteEnabled ? 'aiterm-agent-toggle--on' : ''}`}
```
```tsx
            className="aiterm-block-btn aiterm-block-btn-ai aiterm-btn aiterm-btn--secondary"
```
```tsx
            className="aiterm-block-btn aiterm-btn aiterm-btn--secondary"
```

（`.aiterm-agent-toggle--on` 疊加時因為選擇器優先權 `.aiterm-block-btn.aiterm-agent-toggle--on`（雙 class 選擇器）高於 `.aiterm-btn--secondary`（單 class），固定綠色會正確覆蓋過去，不受影響。）

- [ ] **Step 2: `.terminal-search-btn`（次要）**

現有（`TerminalView.css:249-263`）：
```css
.terminal-search-btn {
  background: transparent;
  border: 1px solid #444;
  color: #ccc;
  border-radius: 3px;
  cursor: pointer;
  padding: 2px 6px;
  font-size: 12px;
  line-height: 1.4;
}

.terminal-search-btn:hover {
  background: #333;
  border-color: #666;
}
```

改為：
```css
.terminal-search-btn {
  font-size: 12px;
  line-height: 1.4;
}
```

`TerminalView.tsx:898-900` 現有：
```tsx
            <button onClick={() => doSearch(searchQuery, 'prev')} title={t.term_search_prev} className="terminal-search-btn">↑</button>
            <button onClick={() => doSearch(searchQuery, 'next')} title={t.term_search_next} className="terminal-search-btn">↓</button>
            <button onClick={closeSearch} title={t.term_search_close} className="terminal-search-btn terminal-search-close">✕</button>
```

改為：
```tsx
            <button onClick={() => doSearch(searchQuery, 'prev')} title={t.term_search_prev} className="terminal-search-btn aiterm-btn aiterm-btn--secondary aiterm-btn--sm">↑</button>
            <button onClick={() => doSearch(searchQuery, 'next')} title={t.term_search_next} className="terminal-search-btn aiterm-btn aiterm-btn--secondary aiterm-btn--sm">↓</button>
            <button onClick={closeSearch} title={t.term_search_close} className="terminal-search-btn terminal-search-close aiterm-btn aiterm-btn--secondary aiterm-btn--sm">✕</button>
```

- [ ] **Step 3: `.aiterm-subtab--active`（選中，改主題色）**

現有（`TerminalView.css:189-208`）：
```css
.aiterm-subtab {
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: #666;
  cursor: pointer;
  font-family: "Cascadia Mono", Consolas, monospace;
  font-size: 12px;
  padding: 4px 12px 6px;
  transition: color 0.15s, border-color 0.15s;
}

.aiterm-subtab:hover {
  color: #aaa;
}

.aiterm-subtab--active {
  color: #79c0ff;
  border-bottom-color: #4a9eff;
}
```

改為（只改 `--active` 顏色）：
```css
.aiterm-subtab {
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: #666;
  cursor: pointer;
  font-family: "Cascadia Mono", Consolas, monospace;
  font-size: 12px;
  padding: 4px 12px 6px;
  transition: color 0.15s, border-color 0.15s;
}

.aiterm-subtab:hover {
  color: #aaa;
}

.aiterm-subtab--active {
  color: var(--accent, #a855f7);
  border-bottom-color: var(--accent, #a855f7);
}
```

（`.tsx` 不需改動。）

- [ ] **Step 4: 驗證**

Run: `cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit && npm run lint 2>&1 | grep -i "TerminalView"`
Expected: `tsc` 無錯誤；不新增 lint 錯誤。

- [ ] **Step 5: Commit**

```bash
git add src/components/TerminalView.css src/components/TerminalView.tsx
git commit -m "$(cat <<'EOF'
refactor(terminal-view): migrate block/search buttons to aiterm-btn

Subtab active-selection color switches from a hardcoded blue to
var(--accent). AI-variant purple and the agent-toggle green status
color are left untouched (distinct semantic roles).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: 圓形送出鈕統一（VcsView / CrossDbView / AiPanel / DesignView）+ CommandBookmarks

**Files:**
- Modify: `src/components/VcsView/VcsView.css`, `VcsView.tsx`
- Modify: `src/components/CrossDbView/CrossDbView.css`, `CrossDbAiChat.tsx`
- Modify: `src/components/AiPanel/styles.css`, `AiPanel/index.tsx`
- Modify: `src/components/DesignView/DesignView.css`, `DesignView.tsx`
- Modify: `src/components/CommandBookmarks.css`, `CommandBookmarks.tsx`

這幾個圓形送出鈕彼此外觀本來就一致（都是 26x26 圓形漸層），純粹是重複定義四次，收斂成疊加共用 class，外觀不變。

- [ ] **Step 1: `VcsView.css:124-151` — `.vcs-view__send-btn`**

現有：
```css
.vcs-view__send-btn {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  background: var(--accent-gradient, linear-gradient(135deg, var(--accent), #6366f1));
  color: #fff !important;
  border: none !important;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-size: 11px;
  padding: 0 !important;
  transition: var(--transition-smooth);
  flex-shrink: 0;
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.15);
}

.vcs-view__send-btn:hover:not(:disabled) {
  transform: scale(1.1);
  box-shadow: 0 0 8px var(--accent-glow);
}

.vcs-view__send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
  background: rgba(255, 255, 255, 0.05) !important;
  color: var(--text-muted) !important;
}
```

改為（整個規則刪除——`.aiterm-btn--icon` + `.aiterm-btn--primary` 已提供等價視覺，唯一差異是 hover 用 `scale(1.1)` 而非 `translateY`；為保留這個圓形按鈕特有的「放大」hover 手感而非直接沿用方形按鈕的「上浮」手感，額外加一條局部覆寫）：
```css
.vcs-view__send-btn:hover:not(:disabled) {
  transform: scale(1.1);
  box-shadow: 0 0 8px var(--accent-glow);
}
```

`VcsView.tsx:244-259` 現有：
```tsx
          <button
            className="vcs-view__send-btn"
            onClick={stop}
            style={{ background: "#2a0f0f", borderColor: "#f87171", color: "#f87171" }}
            title={t.vcs_btn_stop}
          >
            ■
          </button>
        ) : (
          <button
            className="vcs-view__send-btn"
            onClick={handleSubmit}
            disabled={!repoInfo || input.trim() === ""}
            title={t.vcs_btn_send}
          >
            ▲
```

改為（停止狀態改用 `--danger-solid` 疊加，取代原本的 inline style 覆寫）：
```tsx
          <button
            className="vcs-view__send-btn aiterm-btn aiterm-btn--danger-solid aiterm-btn--icon"
            onClick={stop}
            title={t.vcs_btn_stop}
          >
            ■
          </button>
        ) : (
          <button
            className="vcs-view__send-btn aiterm-btn aiterm-btn--primary aiterm-btn--icon"
            onClick={handleSubmit}
            disabled={!repoInfo || input.trim() === ""}
            title={t.vcs_btn_send}
          >
            ▲
```

- [ ] **Step 2: `CrossDbView.css:546-581` — `.crossdb-chat__send-btn` / `.crossdb-chat__stop-btn`**

現有：
```css
.crossdb-chat__send-btn,
.crossdb-chat__stop-btn {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  color: #fff !important;
  border: none !important;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-size: 11px;
  padding: 0 !important;
  transition: var(--transition-smooth);
  flex-shrink: 0;
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.15);
}
.crossdb-chat__send-btn {
  background: var(--accent-gradient, linear-gradient(135deg, var(--accent), #6366f1));
}
.crossdb-chat__stop-btn {
  background: var(--error, #ef4444);
}
.crossdb-chat__send-btn:hover:not(:disabled),
.crossdb-chat__stop-btn:hover:not(:disabled) {
  transform: scale(1.1);
  box-shadow: 0 0 8px var(--accent-glow);
}
.crossdb-chat__send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
  background: rgba(255, 255, 255, 0.05) !important;
  color: var(--text-muted) !important;
  box-shadow: none !important;
  transform: none !important;
}
```

改為：
```css
.crossdb-chat__send-btn:hover:not(:disabled),
.crossdb-chat__stop-btn:hover:not(:disabled) {
  transform: scale(1.1);
  box-shadow: 0 0 8px var(--accent-glow);
}
```

`CrossDbAiChat.tsx:622-633` 現有：
```tsx
          {sending ? (
            <button className="crossdb-chat__stop-btn" onClick={stop} title={t.cdb_ai_btn_stop}>■</button>
          ) : (
            <button
              id="crossdb-ai-send-btn"
              className="crossdb-chat__send-btn"
              onClick={send}
              disabled={!input.trim()}
              title={t.cdb_ai_btn_send}
            >
              ▲
            </button>
          )}
```

改為：
```tsx
          {sending ? (
            <button className="crossdb-chat__stop-btn aiterm-btn aiterm-btn--danger-solid aiterm-btn--icon" onClick={stop} title={t.cdb_ai_btn_stop}>■</button>
          ) : (
            <button
              id="crossdb-ai-send-btn"
              className="crossdb-chat__send-btn aiterm-btn aiterm-btn--primary aiterm-btn--icon"
              onClick={send}
              disabled={!input.trim()}
              title={t.cdb_ai_btn_send}
            >
              ▲
            </button>
          )}
```

- [ ] **Step 3: `AiPanel/styles.css:358-386` — `.aiterm-ai-panel-send-btn`（scoped selector）**

現有：
```css
.aiterm-input-pill-container .aiterm-ai-panel-send-btn {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  background: var(--accent-gradient, linear-gradient(135deg, var(--accent, #a855f7), #6366f1));
  color: #fff;
  border: none;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-size: 11px;
  padding: 0;
  transition: var(--transition-smooth);
  flex-shrink: 0;
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.15);
}

.aiterm-input-pill-container .aiterm-ai-panel-send-btn:hover:not(:disabled) {
  transform: scale(1.1);
  box-shadow: 0 0 8px var(--accent-glow);
}

.aiterm-input-pill-container .aiterm-ai-panel-send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
  transform: none;
  box-shadow: none;
}
```

改為：
```css
.aiterm-input-pill-container .aiterm-ai-panel-send-btn:hover:not(:disabled) {
  transform: scale(1.1);
  box-shadow: 0 0 8px var(--accent-glow);
}
```

`AiPanel/index.tsx:570-578` 現有：
```tsx
          <button
            type="button"
            className="aiterm-ai-panel-send-btn"
            onClick={handleSubmit}
            disabled={isDisabled || input.trim() === ""}
            title="送出"
          >
            ▲
          </button>
```

改為：
```tsx
          <button
            type="button"
            className="aiterm-ai-panel-send-btn aiterm-btn aiterm-btn--primary aiterm-btn--icon"
            onClick={handleSubmit}
            disabled={isDisabled || input.trim() === ""}
            title="送出"
          >
            ▲
          </button>
```

- [ ] **Step 4: `DesignView.css:93-123` — `.design-send-btn`**

現有：
```css
.design-send-btn {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  background: var(--accent-gradient, linear-gradient(135deg, var(--accent), #6366f1));
  color: #fff !important;
  border: none !important;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-size: 11px;
  padding: 0 !important;
  transition: var(--transition-smooth);
  flex-shrink: 0;
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.15);
}

.design-send-btn:hover:not(:disabled) {
  transform: scale(1.1);
  box-shadow: 0 0 8px var(--accent-glow);
}

.design-send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
  background: rgba(255, 255, 255, 0.05) !important;
  color: var(--text-muted) !important;
  box-shadow: none !important;
  transform: none !important;
}
```

改為：
```css
.design-send-btn:hover:not(:disabled) {
  transform: scale(1.1);
  box-shadow: 0 0 8px var(--accent-glow);
}
```

`DesignView.tsx:529` 現有：
```tsx
              <button className="design-send-btn" onClick={() => handleSendMessage()} disabled={!inputValue.trim() || isStreaming} title={t.design_send}>
```

改為：
```tsx
              <button className="design-send-btn aiterm-btn aiterm-btn--primary aiterm-btn--icon" onClick={() => handleSendMessage()} disabled={!inputValue.trim() || isStreaming} title={t.design_send}>
```

- [ ] **Step 5: `CommandBookmarks.css` — `.bookmarks-close` / `.bookmarks-item-delete`（圖示 ghost）**

現有（`CommandBookmarks.css:37-47`）：
```css
.bookmarks-close {
  background: transparent;
  border: none;
  cursor: pointer;
  color: #666;
  font-size: 14px;
  padding: 2px 4px;
  line-height: 1;
}

.bookmarks-close:hover { color: #f87171; }
```

改為：
```css
.bookmarks-close {
  font-size: 14px;
  padding: 2px 4px;
}

.bookmarks-close:hover:not(:disabled) { color: #f87171; }
```

`CommandBookmarks.tsx:104` 現有：
```tsx
          <button className="bookmarks-close" onClick={onClose}>✕</button>
```
改為：
```tsx
          <button className="bookmarks-close aiterm-btn aiterm-btn--ghost" onClick={onClose}>✕</button>
```

現有（`CommandBookmarks.css:109-130`）：
```css
.bookmarks-item-delete {
  position: absolute;
  right: 8px;
  top: 50%;
  transform: translateY(-50%);
  background: transparent;
  border: none;
  cursor: pointer;
  color: #555;
  font-size: 16px;
  line-height: 1;
  padding: 2px 4px;
  opacity: 0;
  transition: opacity 0.1s;
}

.bookmarks-item:hover .bookmarks-item-delete,
.bookmarks-item--active .bookmarks-item-delete {
  opacity: 1;
}

.bookmarks-item-delete:hover { color: #f87171; }
```

改為（保留絕對定位與淡入/淡出邏輯，顏色/hover 交給共用 class；`opacity: 0` 的初始隱藏狀態用 `!important` 蓋掉共用 class 沒有設定 opacity 這件事本身不衝突，維持原樣即可）：
```css
.bookmarks-item-delete {
  position: absolute;
  right: 8px;
  top: 50%;
  transform: translateY(-50%);
  font-size: 16px;
  padding: 2px 4px;
  opacity: 0;
  transition: opacity 0.1s;
}

.bookmarks-item:hover .bookmarks-item-delete,
.bookmarks-item--active .bookmarks-item-delete {
  opacity: 1;
}

.bookmarks-item-delete:hover:not(:disabled) { color: #f87171; }
```

`CommandBookmarks.tsx:132-137` 現有：
```tsx
                <button
                  className="bookmarks-item-delete"
                  onClick={(e) => deleteItem(b.id, e)}
                  title={t.bookmarks_delete_tip}
                >
                  ×
```

改為：
```tsx
                <button
                  className="bookmarks-item-delete aiterm-btn aiterm-btn--ghost"
                  onClick={(e) => deleteItem(b.id, e)}
                  title={t.bookmarks_delete_tip}
                >
                  ×
```

- [ ] **Step 6: 驗證**

Run: `cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit && npm run lint 2>&1 | grep -i "VcsView\|CrossDb\|AiPanel\|DesignView\|CommandBookmarks"`
Expected: `tsc` 無錯誤；不新增 lint 錯誤。

- [ ] **Step 7: Commit**

```bash
git add src/components/VcsView/VcsView.css src/components/VcsView/VcsView.tsx src/components/CrossDbView/CrossDbView.css src/components/CrossDbView/CrossDbAiChat.tsx src/components/AiPanel/styles.css src/components/AiPanel/index.tsx src/components/DesignView/DesignView.css src/components/DesignView/DesignView.tsx src/components/CommandBookmarks.css src/components/CommandBookmarks.tsx
git commit -m "$(cat <<'EOF'
refactor(chat-panels): collapse 4 duplicate circular send-button
definitions onto aiterm-btn--icon, migrate bookmark close/delete icons

VcsView and CrossDbAiChat stop buttons now use aiterm-btn--danger-solid
instead of one-off inline style overrides.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: 整體視覺回歸驗證

**Files:** 無新增/修改檔案 — 純驗證 task。

- [ ] **Step 1: 全域型別與 lint 檢查**

Run: `cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit`
Expected: 無錯誤輸出。

Run: `npm run lint 2>&1 | tail -5`
Expected: 錯誤/警告總數與 Task 1 開始前的 baseline 相同或更少（不應因本次改動新增任何錯誤）。若要精確比對，先在 Task 1 開始前記錄一次 `npm run lint 2>&1 | tail -1` 的問題總數作為 baseline。

- [ ] **Step 2: Playwright 截圖驗證（沿用先前 API Docs / 通知圓點修正時的手法）**

對以下 3 個代表性面板各自建立一個臨時 preview harness（比照本次會話中 API Docs 驗證時的做法：暫時替換 `src/main.tsx` 指向一個 mock 過必要 props 的獨立元件，`npm run dev` 後用 `npx playwright screenshot` 截圖，確認後刪除臨時檔並用 `git diff` 確認 `main.tsx` 無殘留變更）：

1. **LoopStudio** — 確認 `.ls-start-btn`（漸層+8px圓角+hover上浮）、`.ls-stop-btn`（淡紅底+hover實心紅）、`.ls-close-cancel-btn`/`.ls-close-discard-btn`（次要+危險確認鈕配對）視覺正確。
2. **Settings → Providers 頁** — 確認 `.btn-add`（漸層主要鈕）、`provider-card-actions` 內按鈕（次要+危險）視覺正確。
3. **DatabaseView → AI Chat** — 確認送出鈕（圓形漸層，disabled 時變暗但不再是三元運算的兩種硬編碼顏色）、停止鈕（圓形實心紅）視覺正確。

Run:
```bash
lsof -ti:1420 | xargs kill -9 2>/dev/null
(npm run dev > /tmp/vite-verify.log 2>&1 &) ; sleep 3; cat /tmp/vite-verify.log
```
Expected: `VITE ... ready` 訊息，無 build error。

- [ ] **Step 3: 確認 `main.tsx` 無殘留變更**

Run: `git status --short && git diff -- src/main.tsx`
Expected: `git status --short` 只列出本次計畫涉及的檔案；`git diff -- src/main.tsx` 若曾用於暫時 preview，此時應為空（已還原）。

Run: `lsof -ti:1420 | xargs kill -9 2>/dev/null; echo stopped`

- [ ] **Step 4: 手動確認選中狀態顏色語彙**

Run: `grep -rn "aiterm-agent-toggle--on" /Users/jamesju/Documents/GitHub/AITERM/src/components/*/[A-Za-z]*.css`
Expected: 只有 `TerminalView.css`（固定綠）、`DatabaseView/index.css`（固定綠）、`AiPanel/styles.css`（`var(--accent)`）三處定義，且顏色值與 Task 開始前一致（本計畫全程未修改這三處，作為最終確認）。

- [ ] **Step 5（無需 commit — 純驗證 task，若步驟 1-4 全部通過，回報使用者計畫完成）**

---

## Self-Review Notes（寫計畫時的自我檢查）

- **Spec 覆蓋度**：Task 1 對應「核心機制」；Task 2-9 依檔案覆蓋 spec「各類別改動清單」列出的全部 5 大類別、23 個檔案；Task 10 對應「測試」與「成功標準」。全部涵蓋，唯一例外：
  - `Settings/SettingsView.css` 的 `.sidebar-item`/`.sidebar-item--active`/`.sidebar-back` — spec 原列在次要按鈕類別，但研究確認 `.sidebar-item--active` 已經是 `color: var(--accent, #a855f7); background: var(--accent-dim, ...)`，圓角已是 8px，且擁有獨有的 `translateX(2px)` hover 動效。這裡沒有任何硬編碼顏色需要修正，**本計畫不建立對應 task**，維持現狀即可（符合 spec 原文「維持獨有的 translateX hover 動效，僅顏色/圓角對齊」——對齊後發現本來就對齊，故無需改動）。
- **Placeholder 掃描**：全部 Step 都附完整程式碼，無 TBD/TODO。
- **一致性**：`.aiterm-btn--danger` vs `.aiterm-btn--danger-solid` 的使用時機在 Task 1 定義後，Task 2/3/6/9 中每個危險按鈕都依「是否已在二次確認情境」明確選擇正確變體，命名/用法前後一致。
- **超出 spec 範圍但必要的修正**：`db-view__subtab--active`（spec 未列出但屬於「選中」語彙，Task 3 Step 11 一併處理）；`DatabaseAiChat.tsx` 的 JS `onMouseEnter`/`onMouseLeave` hover hack 改為 CSS（純樣式手法調整，不影響行為）；`DatabaseConnectionsPage.tsx`/`VcsConnectionsPage.tsx` 若因移除 `btnStyle` 導致 `CSSProperties` import 變成未使用，Task 6 Step 7 已包含檢查與清理步驟。

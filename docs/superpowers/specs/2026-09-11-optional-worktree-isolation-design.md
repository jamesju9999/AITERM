# 工作看板：讓 git worktree 隔離變成可關閉的選項

日期：2026-09-11

## 起因

目前 `prepare_worktree`（`src-tauri/src/tasks/scheduler.rs:70-95`）只要偵測到
專案目錄是 git repo，就**無條件**建立一個 worktree 與 `aiterm-task/<id>` 分支，
讓派工的 Claude Code 在裡面跑；不是 git repo 才退回直接在 `project_dir` 執行。
沒有任何關掉的方式。

使用者的理由不是踩到痛點，而是**現在的 AI Agent 本來就會自己操作 git 版控**，
所以 AITerm 強加的隔離變成多餘的保護，應該讓使用者自己選。

## 目標

讓使用者能關掉 worktree 隔離，讓派工的 Agent 直接在專案目錄工作。

**預設維持現狀**（隔離開著）——這不是要改變既有行為，只是多一個選項。

## 兩層設定

### 全域

`TaskBoardConfig`（`src-tauri/src/config/types.rs:199`）新增：

```rust
/// 派工時是否為每張卡片建立獨立的 git worktree。關掉的話 Agent 直接在
/// 專案目錄工作——適合信任 Agent 自己管 git 版控的使用者。
#[serde(default = "default_true")]
pub isolate_with_worktree: bool,
```

設定頁 `src/components/Settings/TaskBoardPage.tsx` 加一個開關，沿用
`auto_close_finished_tabs` 既有的樣式。

### 每張卡片

`tasks` 表新增 `isolate_worktree` 欄位，型別 `Option<bool>`：

- `NULL`（預設）＝沿用全域設定
- `true` ＝這張卡一定建 worktree
- `false` ＝這張卡直接在專案目錄跑

`TaskEditorDialog` 加一個三態 `select`，沿用該檔案既有的 `bridgeChoice`
做法（`TaskEditorDialog.tsx:62`）：

| 選項 | 存進資料庫的值 |
|------|--------------|
| 沿用全域設定 | `NULL` |
| 建立獨立的 worktree | `true` |
| 直接在專案目錄執行 | `false` |

## 決定時機

在**派工當下**於 `prepare_worktree` 解析：

```
effective = card.isolate_worktree.unwrap_or(global.isolate_with_worktree)
```

放在派工當下而不是建立卡片時，是為了讓「全域設定」真的具有預設值的語意——
調整全域開關會影響所有還在「待執行」、而且沒有個別覆寫的卡片。已經在跑或
跑完的卡片不受影響（它們的 worktree 早就建好了）。

## 關掉之後走哪條路

`prepare_worktree` 直接回傳 `(PathBuf::from(task_project_dir), None)`——**跟
現在「這個目錄不是 git repo」完全同一條路徑**，不是新邏輯。

因此：

- 卡片不會有 `worktree_path` / `worktree_branch`。
- 「合併回原分支」按鈕的顯示條件是 `card.worktree_branch`
  （`TaskCard.tsx`），所以它自然不會出現，不需要額外判斷。
- `tasks_merge_worktree` 對這種卡片本來就會回
  `"this card has no worktree to merge"`，但按鈕根本不會出現，走不到。

## 風險提示

關掉隔離之後，多張卡片同時跑會在**同一個工作目錄**互相覆蓋——這正是
worktree 隔離原本在防的事。Agent 會自己管 git，不代表兩個 Agent 同時改同一
份檔案不會打架。

處理方式：

- 設定頁的說明文字明講這件事。
- 當「關掉隔離」**且** `max_concurrent > 1` 時，額外顯示一行提醒。

**不強制修改 `max_concurrent`**。使用者明確表示信任 Agent 自己管版控，這裡
只提醒、不代勞。

## 測試

Rust：

- `prepare_worktree` 在三種組合下的回傳值：全域開（建 worktree）、全域關
  （回傳 project_dir 且 `None`）、卡片覆寫勝過全域（兩個方向都要測）。
- 舊資料庫沒有 `isolate_worktree` 欄位時 `init_schema` 要能升級且舊資料讀成
  `NULL`——照抄 `store.rs:1434` 既有的遷移測試寫法。

前端：

- 設定頁的開關能存進 config。
- 編輯對話框的三態 select 存出正確的 `null` / `true` / `false`，且既有卡片
  開啟時會回填正確的選項。

## 非目標

- 不做專案層級（`.aitprj`）的設定。兩層已經夠用，第三層只會讓「這張卡到底
  會不會建 worktree」更難回答。
- 不因為關掉隔離就自動調整 `max_concurrent` 或 `parallel_ok`。

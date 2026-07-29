# 更新提示中顯示更新項目

**日期**：2026-07-29
**狀態**：待審閱

## 背景與目標

更新提示的說明欄目前顯示一串網址（`https://github.com/.../releases/tag/v1.2.3`），對使用者沒有意義。應該顯示這一版實際改了什麼。

### 現況調查

三件事已在動手前查證：

**前端已經在渲染這個欄位。** `UpdateModal.tsx:59` 渲染 `state.notes`，其值是 `useUpdater.ts:128` 的 `update.body`，來自 `latest.json` 的 `notes`。紅框裡的網址就是這個欄位，不是硬編碼的連結。

**前端已經能正確顯示多行內容。** `.update-modal-notes`（`UpdateModal.css:34-42`）已有 `white-space: pre-wrap`、`max-height: 120px`、`overflow-y: auto`、`overflow-wrap: anywhere`。多行條列會斷行並可捲動，**不需要任何樣式改動**。

**這份資料在整條管線裡不存在。** `latest.json` 的 `notes` 在 `release.yml:535` 寫死成 release 網址；release body 則是 `create-release` 的一份靜態模板，只有下載指引與 macOS xattr 提示，沒有任何變更清單。

所以這不是「把欄位接上去」，而是要先讓「更新項目」這份資料存在。

### 為什麼不純靠 git log

這個 repo 的 commit 粒度是實作步驟，不是使用者可見的變更。v1.2.4 的 14 個 commit 即使只留 `feat`/`fix` 仍有 8 條，包含 `fix(test): type the invoke mock table so tsc -b passes` 這種對使用者毫無意義的條目；而對使用者來說 v1.2.4 只有一件事：AppImage 可以建立桌面選單項目了。commit 訊息也全是英文，UI 是繁中。

因此採自動草稿 + 人工改寫。

**目標**：更新提示顯示人工確認過、使用者看得懂的變更清單，且 `latest.json` 的內容必定等於發布時 release 頁面上的內容。

## 範圍界定（已與使用者確認）

| 決策點 | 選擇 |
|---|---|
| 內容來源 | 自動產生草稿，人工改寫 |
| 改寫時機 | `finalize` 前停等審核閘 |
| 提示框連結 | 保留，改為指向該版本的 release 頁 |
| 找不到內容時 | fail loud，不退回網址 |

### 明確排除（Non-goals）

- 不做雙語 notes。`latest.json` 的 `notes` 是單一字串，無法依 locale 切換。
- 不引入 markdown 轉譯器。草稿與人工內容都寫成純文字條列，`pre-wrap` 已足夠。
- 不追溯修改 v1.2.4 及更早 release 的 `notes`。
- 不自動判斷哪些 commit「對使用者有意義」——那正是人工那一步存在的理由。

## 資料流

```
create-release ──► draft release body（含 <!-- changelog:start/end --> 區塊，內容為草稿）
                        │
                   六條腿建置（~15 分鐘）
                        │
                   ◄── 使用者在 GitHub 上改寫該區塊
                        │
finalize（停在 Environment 審核閘）
                        │
                   使用者按 Approve
                        │
                   重讀 release body → 取出區塊 → 寫入 latest.json 的 notes → 發布
```

單一來源是 release body。`latest.json` 從它取值，因此兩者不可能不一致。

**既有行為對此有利**：`create-release` 在 release 已存在時走 reuse 路徑，不覆寫 body（`release.yml:76-79`）。因此使用者的編輯在重跑 workflow 時會存活。

## 分隔標記

```
### 更新項目
<!-- changelog:start -->
- feat(appimage): add a Settings section to create or remove the entry
<!-- changelog:end -->
```

用 HTML 註解，因為 GitHub 的 markdown 會把它隱藏——使用者編輯時看得到標記（在編輯框內），瀏覽 release 頁的人看不到。

取出時以第一個 `changelog:start` 為起點、其後第一個 `changelog:end` 為終點。

## `scripts/release_notes.py`

CI 裡的邏輯抽成獨立腳本而非內嵌 heredoc，理由是這個 repo 已經被無法測試的 CI 邏輯咬過兩次：`uploadUpdaterJson` 這個不存在的參數被靜默忽略了整整一輪發版；repack 步驟裡的相對路徑 `$APP` 會讓每一次發版都失敗，而 `bash -n` 與 YAML 解析都驗不出來。純函式加上可在本機執行的測試，是唯一能事前發現這類錯誤的方式。

```python
def filter_commits(subjects: list[str]) -> list[str]:
    """只留 feat/fix，保留 type(scope): 前綴。"""

def render_draft(kept: list[str]) -> str:
    """輸出成 '- <subject>' 的多行字串；kept 為空時輸出佔位行。"""

def extract_changelog(body: str) -> str:
    """取出兩個標記之間的內容並 strip。找不到標記或內容為空時 raise。"""
```

CLI 兩個子命令，皆以 stdin 進、stdout 出：

| 指令 | 用途 |
|---|---|
| `python3 scripts/release_notes.py draft` | stdin 收 `git log --pretty=%s` 的輸出，stdout 給草稿區塊內容 |
| `python3 scripts/release_notes.py extract` | stdin 收 release body，stdout 給更新項目；失敗時 exit 1 並在 stderr 說明 |

佔位行內容：`- （本版無使用者可見的變更，請改寫此行）`。不留空，因為空區塊會讓 `extract` 失敗、擋住發版。

## workflow 改動

### `create-release`

新增兩個步驟於現有的 `create` 之前：

1. `actions/checkout@v4`，`fetch-depth: 0`——需要完整歷史與 tag 才能算出區間。
2. 產生草稿：

```bash
PREV=$(git describe --tags --abbrev=0 "$TAG^" 2>/dev/null || true)
RANGE=${PREV:+$PREV..}$TAG
git log --pretty=format:%s "$RANGE" | python3 scripts/release_notes.py draft
```

`PREV` 為空（第一個 tag）時 `RANGE` 退化為 `$TAG`，涵蓋全部歷史。

草稿以 step output 傳給既有的 `github-script`，插進 `RELEASE_BODY` 的「### 更新項目」區塊，位置在「### 下載」之前。

### `finalize`

1. 新增 `actions/checkout@v4`（預設深度即可，只需要腳本）。
2. 新增 `environment: release-approval`。
3. 「Compose latest.json」改為先取 body 再取出區塊：

```bash
gh release view "$TAG" --json body -q .body > body.md
NOTES=$(python3 scripts/release_notes.py extract < body.md)
```

`NOTES` 取代 `release.yml:535` 寫死的網址。`extract` 失敗時整個 job 失敗，release 留在 draft——使用者看不到任何東西，因為 updater endpoint 走 `releases/latest`，draft 不在其中。

## 審核閘的陷阱

**environment 不存在、或存在但未設 required reviewers 時，GitHub 會讓 job 直接執行，不產生任何錯誤或警告。**

這與本專案稍早遇到的 secret 陷阱同類：secret 設在 environment 層而非 repo 層時，缺失的值展開成空字串，錯誤訊息指向金鑰格式而非「沒設定」。兩者都是「設定沒生效，但外觀完全正常」。

因此設定完成後必須驗證，且不能只看網頁畫面：

```bash
gh api repos/jamesju9999/AITERM/environments/release-approval \
  -q '.protection_rules[] | select(.type=="required_reviewers") | .reviewers[].reviewer.login'
```

必須印出使用者帳號。沒有輸出即代表閘不存在，發版會直接衝過去。

`release-approval` 這個 environment **只用於審核，不得放入任何 secret**。本專案先前有一個 `copilot` environment 存放了更新私鑰的副本，而 repo 層缺少同名 secret 時展開成空字串，導致錯誤訊息指向金鑰格式。簽章金鑰維持在 repo 層。

## 前端改動

唯一的改動是加一個連到該版本 release 頁的連結——網址被更新項目取代後，否則就完全失去看完整說明（下載指引、macOS xattr 提示）的路徑。

- `src/lib/repo.ts` 新增 `releaseTagUrl(version: string)`，回傳 `${GITHUB_REPO_URL}/releases/tag/v${version}`。
- `UpdateModal.tsx` 在 `status === "available"` 時，於更新項目下方渲染一個文字按鈕，點擊呼叫 `openUrl(releaseTagUrl(state.version))`。
- i18n 新增 `update_view_full_notes`：繁中「查看完整說明」，英文 "View full release notes"。

不論 `notes` 是否為空都顯示——`notes` 為空時更需要那個連結。

## 錯誤處理

| 情境 | 行為 |
|---|---|
| release body 找不到標記（編輯時誤刪） | `extract` exit 1，stderr 說明要補回哪兩行；release 留在 draft |
| 標記之間是空的 | 同上 |
| `git describe` 找不到前一個 tag | 涵蓋全部歷史，不視為錯誤 |
| 過濾後沒有任何 commit | 草稿寫佔位行，不留空 |
| 舊版 release 的 `notes` 仍是網址 | 前端原樣顯示，無害 |

## 測試策略

| 層級 | 內容 |
|---|---|
| Python 純函式 | `filter_commits`：留 `feat`/`fix`，丟 `chore`/`docs`/`test`/`style`/`refactor`/`ci`；`feat:` 與 `feat(scope):` 兩種形式都要留；`feature:` 這類不完全相符的不留 |
| Python 純函式 | `render_draft`：空清單輸出佔位行；非空時每行加 `- ` |
| Python 純函式 | `extract_changelog`：正常取出、缺開始標記、缺結束標記、標記順序顛倒、內容全為空白、body 中出現多組標記 |
| 前端 | 多行 `notes` 完整渲染（不因換行被截斷） |
| 前端 | `available` 狀態顯示連結，且連結指向 `releases/tag/v<version>` 而非 `releases/latest` |
| 前端 | `notes` 為空時仍顯示連結 |

Python 測試用標準庫 `unittest`，不引入新相依。執行：`python3 -m unittest discover -s scripts -p 'test_*.py'`。

**mutation testing 為驗收條件**，重點在 `extract_changelog` 的失敗路徑：把 raise 改成 `return ""` 必須有測試失敗。靜默回傳空字串會讓發版繼續進行、`notes` 變空，而所有 job 都是綠的——正是這個 repo 反覆吃虧的失敗模式。

## 驗證限制

**審核閘與端到端流程無法在本機或現有 CI 驗證**，因為這個 repo 只有 `release.yml` 一個 workflow，沒有任何 PR 層級的測試閘；Python 測試與前端測試都只在本機執行。

必須在下次實際發版時確認：

1. draft release body 含「### 更新項目」區塊與草稿內容
2. 編輯該區塊後，`finalize` **確實停下來等待審核**（這是最容易靜默失效的一項）
3. Approve 後 `latest.json` 的 `notes` 等於編輯後的內容
4. 更新提示顯示該內容，多行完整、可捲動
5. 【查看完整說明】開啟的是該版本的 release 頁
6. 故意刪掉一個標記後重跑，`finalize` 失敗且 release 留在 draft

## 驗證結果（v1.2.5，2026-07-29）

| # | 項目 | 結果 |
|---|---|---|
| 1 | draft body 含草稿 | ✅ 10 行 `feat`/`fix`，兩個標記齊全，區塊位於「### 下載」之前 |
| — | `${{ }}` 多行插值 | ✅ 每一行落在第 0 欄，未被 markdown 當成縮排程式碼區塊 |
| — | `GITHUB_OUTPUT` heredoc | ✅ 草稿完整無截斷 |
| 2 | **審核閘確實停住** | ✅ 六條 build 全部 success 後，`finalize` 狀態為 `waiting` |
| 3 | **notes 與編輯後的 body 相符** | ✅ 逐位元組相同（83 bytes），無殘留 `\r` |
| 4 | 更新提示顯示該內容 | ✅ Ubuntu 實機，v1.2.4 收到更新提示，顯示中文更新項目而非網址 |

### 未驗證

**第 5 項（【查看完整說明】）在本次發版無法驗證。** 該按鈕由 App 端程式碼渲染，v1.2.5 才加入；而更新提示只在「有新版本可用」時出現。收到 v1.2.5 提示的機器跑的是 v1.2.4，其程式碼沒有這個按鈕；升級到 v1.2.5 之後又不再有更新可提示。

**要等 v1.2.6 發版才能實機驗證。** 這是本設計的結構性限制，非缺陷：任何只在更新提示中出現的 UI，都無法在引入它的那一版被實機看到。

單元測試涵蓋了行為（點擊帶版本號回呼、`notes` 為空時仍顯示、下載中不顯示、URL 指向 `releases/tag/v<版本>` 而非 `releases/latest`），但那不等於實機驗證。

第 6 項（失敗路徑）尚未執行。

# VCS 團隊可見度面板與引導流程 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在既有的「版本控制」分頁上，新增一個團隊可見度面板與結構化的「開始新功能／完成送審／合併」按鈕流程，讓 2-3 人團隊能在不懂 git/GitHub 術語的情況下，看到彼此在做什麼、開始前避免撞檔案、最後在 AITerm 內直接完成審核與合併。

**Architecture:** 不建新伺服器、不新增資料庫表。所有團隊狀態即時從 GitHub PR API 取得（open PR = 進行中的功能），沿用既有的 `GitClient`（`src-tauri/src/vcs/git.rs`）並在上面擴充幾支新方法。前端在既有的 `VcsView.tsx` 聊天框上方掛一個新面板 + 兩個引導式對話框，全部走新增的、直接呼叫 `GitClient` 的 Tauri 指令（不經過現有的自然語言 `VcsIntent` 解析層，因為按鈕流程已經知道確切要做什麼）。

**Tech Stack:** Rust（`reqwest` 呼叫 GitHub REST + GraphQL API）、`wiremock`（測試假造 GitHub API）、React + TypeScript、Vitest + Testing Library。

**對應 spec：** `docs/superpowers/specs/2026-08-17-vcs-team-collaboration-design.md`（本 plan 只涵蓋「團隊可見度面板」「開始新功能」「完成送審→合併」三塊；MCP 整合是獨立的 `docs/superpowers/plans/2026-08-17-vcs-team-mcp-server.md`，依賴本 plan 完成後的資料層）。

---

## 檔案總覽

**修改（Rust）：**
- `src-tauri/src/vcs/git.rs` — `GitClient` 新增可測試的 API base URL、`create_pr` 加 `draft` 參數並改回傳型別、新增 `list_active_features`／`mark_pr_ready`／`pr_diff` 方法
- `src-tauri/src/vcs/mod.rs` — 新增 `pub mod overlap;`
- `src-tauri/src/vcs/types.rs` — 新增 `ActiveFeature` 型別
- `src-tauri/src/commands/vcs.rs` — 新增 6 個 Tauri 指令、更新 `create_pr` 呼叫端
- `src-tauri/src/lib.rs` — 註冊新指令

**新增（Rust）：**
- `src-tauri/src/vcs/overlap.rs` — 純函式重疊偵測 + 單元測試
- `src-tauri/tests/vcs_git_client.rs` — 新 `GitClient` 方法的 wiremock 整合測試

**新增（前端）：**
- `src/hooks/useTeamFeatures.ts`
- `src/components/VcsView/TeamPanel.tsx` + `.test.tsx`
- `src/components/VcsView/StartFeatureDialog.tsx` + `.test.tsx`
- `src/components/VcsView/FinishFeatureReview.tsx` + `.test.tsx`

**修改（前端）：**
- `src/ipc/vcs.ts` — 新增型別與 IPC 包裝函式
- `src/components/VcsView/VcsView.tsx` — 掛載新面板與對話框
- `src/lib/i18n.ts` — 新增中/英文案

---

## Task 1: 讓 `GitClient` 的 GitHub API base URL 可測試

現況：`git.rs` 裡每個 GitHub API 方法都直接把 `"https://api.github.com"` 寫死在 `format!` 字串裡，導致這個檔案裡呼叫 GitHub API 的方法（`pr_list`、`create_pr`、`merge_pr`...）完全沒有測試覆蓋——沒有任何辦法讓它們在測試時打向 wiremock 假伺服器。這是接下來所有新方法都要用到的測試地基，必須先做。

**Files:**
- Modify: `src-tauri/src/vcs/git.rs:13-21`（`GitClient` struct 與 `new`）

- [ ] **Step 1: 幫 `GitClient` 加上可覆寫的 API base URL 欄位**

把：

```rust
pub struct GitClient {
    pub repo_root: String,
    pub token: Option<String>,
}

impl GitClient {
    pub fn new(repo_root: String, token: Option<String>) -> Self {
        Self { repo_root, token }
    }
```

改成：

```rust
pub struct GitClient {
    pub repo_root: String,
    pub token: Option<String>,
    github_api_base: String,
}

impl GitClient {
    pub fn new(repo_root: String, token: Option<String>) -> Self {
        Self {
            repo_root,
            token,
            github_api_base: "https://api.github.com".to_string(),
        }
    }

    /// 只給測試用：讓 GitHub API 呼叫打向 wiremock 假伺服器而非真的 GitHub。
    pub fn new_with_api_base(repo_root: String, token: Option<String>, github_api_base: String) -> Self {
        Self { repo_root, token, github_api_base }
    }
```

現有所有 `GitClient::new(...)` 呼叫端（`git.rs` 內的兩個測試、`commands/vcs.rs` 的兩個呼叫點）完全不用改，因為 `new` 簽章沒變，行為預設值就是原本寫死的網址。

- [ ] **Step 2: 編譯確認沒有破壞任何東西**

Run: `cd src-tauri && cargo check`
Expected: 編譯成功，因為目前還沒有任何方法讀 `github_api_base` 欄位（會有 `field is never read` 警告，這是暫時的，下一個 task 開始用它就會消失）。

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/vcs/git.rs
git commit -m "refactor(vcs): make GitClient's GitHub API base URL overridable for tests"
```

---

## Task 2: `ActiveFeature` 型別 + 純函式重疊偵測

**Files:**
- Modify: `src-tauri/src/vcs/types.rs`
- Modify: `src-tauri/src/vcs/mod.rs`
- Create: `src-tauri/src/vcs/overlap.rs`

- [ ] **Step 1: 在 `types.rs` 新增 `ActiveFeature`**

在 `src-tauri/src/vcs/types.rs` 裡 `PrEntry` 定義之後（第 113 行後）加入：

```rust
/// 一個進行中功能的完整樣貌——PR 本身的資訊，加上它目前實際改動的檔案。
/// 團隊可見度面板與重疊偵測都是建立在這個型別上；刻意跟既有給自然語言
/// 對話流程用的 `PrEntry` 分開，兩邊的資料契約互不影響。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveFeature {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub draft: bool,
    pub url: String,
    pub updated_at: String,
    pub head_ref: String,
    pub files: Vec<String>,
}
```

- [ ] **Step 2: 註冊新模組**

在 `src-tauri/src/vcs/mod.rs` 第 5-7 行：

```rust
pub mod git;
pub mod svn;
pub mod types;
```

改成：

```rust
pub mod git;
pub mod overlap;
pub mod svn;
pub mod types;
```

- [ ] **Step 3: 寫 `overlap.rs` 的失敗測試**

Create `src-tauri/src/vcs/overlap.rs`：

```rust
//! 純函式：比對「即將要動的檔案」跟「其他人進行中功能已經動過的檔案」有沒有重疊。
//! 不碰 HTTP、不碰 git，方便完整單元測試覆蓋。

use super::types::ActiveFeature;

/// 正規化一個 repo 相對路徑，讓 Windows 產生的 `\` 路徑跟 Unix 的 `/` 路徑能對得起來。
fn normalize(path: &str) -> String {
    path.trim().replace('\\', "/")
}

/// 回傳 `features` 裡跟 `candidate_files` 有任何檔案重疊的項目。
/// `candidate_files` 是空的（使用者沒填「預計檔案」）就直接回傳空清單——
/// 沒有東西可比對，不該假裝比對出了「沒有衝突」。
pub fn find_overlaps(candidate_files: &[String], features: &[ActiveFeature]) -> Vec<ActiveFeature> {
    if candidate_files.is_empty() {
        return Vec::new();
    }
    let candidates: std::collections::HashSet<String> =
        candidate_files.iter().map(|f| normalize(f)).collect();

    features
        .iter()
        .filter(|f| f.files.iter().any(|file| candidates.contains(&normalize(file))))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(author: &str, files: &[&str]) -> ActiveFeature {
        ActiveFeature {
            number: 1,
            title: format!("{author} 的功能"),
            author: author.to_string(),
            draft: true,
            url: "https://github.com/x/y/pull/1".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            head_ref: format!("feature/{author}"),
            files: files.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn detects_exact_file_overlap() {
        let features = vec![feature("bob", &["src/App.tsx", "src/index.ts"])];
        let hits = find_overlaps(&["src/App.tsx".to_string()], &features);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].author, "bob");
    }

    #[test]
    fn no_overlap_returns_empty() {
        let features = vec![feature("bob", &["src/App.tsx"])];
        let hits = find_overlaps(&["src/Other.tsx".to_string()], &features);
        assert!(hits.is_empty());
    }

    #[test]
    fn empty_candidate_list_returns_empty_without_false_positives() {
        let features = vec![feature("bob", &["src/App.tsx"])];
        let hits = find_overlaps(&[], &features);
        assert!(hits.is_empty());
    }

    #[test]
    fn normalizes_windows_backslashes_before_comparing() {
        let features = vec![feature("bob", &["src\\App.tsx"])];
        let hits = find_overlaps(&["src/App.tsx".to_string()], &features);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn does_not_match_different_files_in_same_directory() {
        let features = vec![feature("bob", &["src/App.tsx"])];
        let hits = find_overlaps(&["src/AppTest.tsx".to_string()], &features);
        assert!(hits.is_empty());
    }

    #[test]
    fn matches_against_multiple_features_independently() {
        let features = vec![
            feature("bob", &["src/App.tsx"]),
            feature("carol", &["src/Other.tsx"]),
        ];
        let hits = find_overlaps(&["src/Other.tsx".to_string()], &features);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].author, "carol");
    }
}
```

- [ ] **Step 4: 執行測試確認全過**

Run: `cd src-tauri && cargo test --lib vcs::overlap`
Expected: 6 個測試全部 `ok`（這個函式從一開始就寫對，這裡的「先紅後綠」體現在「先想清楚 6 種情境」而不是先看到失敗——如果想嚴格照紅燈流程，可以先把 `find_overlaps` 函式體改成 `unimplemented!()` 跑一次確認全部失敗，再貼上面的實作）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/vcs/types.rs src-tauri/src/vcs/mod.rs src-tauri/src/vcs/overlap.rs
git commit -m "feat(vcs): add ActiveFeature type and pure file-overlap detection"
```

---

## Task 3: `create_pr` 支援 draft，回傳型別改成結構化資料

現況：`create_pr` 回傳 `Result<VcsResult, String>`，PR 編號跟網址被格式化進一句 `detail` 字串裡（`"Created PR #5: https://..."`），沒辦法在新的引導流程裡直接拿到編號。改成回傳 `(u64, String)`（PR 編號、網址），原本唯一的呼叫端（自然語言對話流程）自己把它包回 `VcsResult::WriteSuccess`，字串內容完全不變。

**Files:**
- Modify: `src-tauri/src/vcs/git.rs:259-289`（`create_pr`）
- Modify: `src-tauri/src/commands/vcs.rs:765-769`（唯一呼叫端）
- Create: `src-tauri/tests/vcs_git_client.rs`

- [ ] **Step 1: 寫 wiremock 測試（先確認會紅）**

Create `src-tauri/tests/vcs_git_client.rs`：

```rust
use aiterm_lib::vcs::git::GitClient;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn create_pr_sends_draft_flag_and_returns_number_and_url() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/repos/acme/widget/pulls"))
        .and(header("authorization", "Bearer test-token"))
        .and(body_json(serde_json::json!({
            "title": "登入頁優化",
            "head": "feature/login-optimize",
            "base": "main",
            "body": "",
            "draft": true,
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 42,
            "html_url": "https://github.com/acme/widget/pull/42",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    std::process::Command::new("git")
        .args(["init", "-q"]).current_dir(dir.path()).status().unwrap();
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/acme/widget.git"])
        .current_dir(dir.path()).status().unwrap();

    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    let (number, url) = client
        .create_pr("登入頁優化", "feature/login-optimize", "main", None, true)
        .await
        .expect("create_pr should succeed");

    assert_eq!(number, 42);
    assert_eq!(url, "https://github.com/acme/widget/pull/42");
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test vcs_git_client create_pr_sends_draft_flag`
Expected: 編譯失敗——`create_pr` 還沒有 `draft` 參數、`new_with_api_base` 目前的 GitHub API 呼叫還是打向真正的 `https://api.github.com`（下一步才會改用 `self.github_api_base`）。

- [ ] **Step 3: 修改 `create_pr`**

`src-tauri/src/vcs/git.rs` 把：

```rust
    pub async fn create_pr(
        &self,
        title: &str,
        head: &str,
        base: &str,
        body: Option<&str>,
    ) -> Result<VcsResult, String> {
        let token = self.require_token(3)?;
        let (owner, repo) = self.parse_remote()?;
        let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls");

        let payload = serde_json::json!({
            "title": title,
            "head": head,
            "base": base,
            "body": body.unwrap_or(""),
        });

        let resp = self
            .gh_post(&token, &url, &payload)
            .await?;

        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let pr_url = json["html_url"].as_str().unwrap_or("").to_string();
        let number = json["number"].as_u64().unwrap_or(0);

        Ok(VcsResult::WriteSuccess {
            operation: "create_pr".to_string(),
            detail: format!("Created PR #{number}: {pr_url}"),
        })
    }
```

改成：

```rust
    pub async fn create_pr(
        &self,
        title: &str,
        head: &str,
        base: &str,
        body: Option<&str>,
        draft: bool,
    ) -> Result<(u64, String), String> {
        let token = self.require_token(3)?;
        let (owner, repo) = self.parse_remote()?;
        let url = format!("{}/repos/{owner}/{repo}/pulls", self.github_api_base);

        let payload = serde_json::json!({
            "title": title,
            "head": head,
            "base": base,
            "body": body.unwrap_or(""),
            "draft": draft,
        });

        let resp = self.gh_post(&token, &url, &payload).await?;

        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let pr_url = json["html_url"].as_str().unwrap_or("").to_string();
        let number = json["number"].as_u64().unwrap_or(0);

        Ok((number, pr_url))
    }
```

- [ ] **Step 4: 更新唯一的呼叫端**

`src-tauri/src/commands/vcs.rs` 第 765-769 行，把：

```rust
        VcsIntent::CreatePr { title, head, base, body } => {
            client
                .create_pr(&title, &head, &base, body.as_deref())
                .await
        }
```

改成：

```rust
        VcsIntent::CreatePr { title, head, base, body } => {
            client
                .create_pr(&title, &head, &base, body.as_deref(), false)
                .await
                .map(|(number, pr_url)| VcsResult::WriteSuccess {
                    operation: "create_pr".to_string(),
                    detail: format!("Created PR #{number}: {pr_url}"),
                })
        }
```

（`draft: false`——自然語言對話流程原本的行為就是開一般 PR，這裡維持不變。）

- [ ] **Step 5: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test vcs_git_client create_pr_sends_draft_flag`
Expected: `test create_pr_sends_draft_flag_and_returns_number_and_url ... ok`

- [ ] **Step 6: 全量編譯確認沒有破壞既有程式碼**

Run: `cd src-tauri && cargo check`
Expected: 編譯成功，無錯誤。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/vcs/git.rs src-tauri/src/commands/vcs.rs src-tauri/tests/vcs_git_client.rs
git commit -m "feat(vcs): support draft PRs and return structured (number, url) from create_pr"
```

---

## Task 4: `GitClient::list_active_features`

拉出目前 repo 所有 open PR，並且各自抓一次改動的檔案清單，組成 `ActiveFeature` 陣列。

**Files:**
- Modify: `src-tauri/src/vcs/git.rs`
- Modify: `src-tauri/tests/vcs_git_client.rs`

- [ ] **Step 1: 寫 wiremock 測試（先確認會紅）**

在 `src-tauri/tests/vcs_git_client.rs` 加入：

```rust
async fn init_repo_with_origin(dir: &std::path::Path) {
    std::process::Command::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap();
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/acme/widget.git"])
        .current_dir(dir).status().unwrap();
}

#[tokio::test]
async fn list_active_features_combines_pr_list_and_per_pr_files() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 7,
                "title": "登入頁優化",
                "user": {"login": "alice"},
                "draft": true,
                "html_url": "https://github.com/acme/widget/pull/7",
                "updated_at": "2026-08-17T00:00:00Z",
                "head": {"ref": "feature/login-optimize"}
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls/7/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"filename": "src/Login.tsx"},
            {"filename": "src/api/auth.ts"}
        ])))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;
    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    let features = client.list_active_features().await.expect("should succeed");

    assert_eq!(features.len(), 1);
    assert_eq!(features[0].number, 7);
    assert_eq!(features[0].author, "alice");
    assert!(features[0].draft);
    assert_eq!(features[0].head_ref, "feature/login-optimize");
    assert_eq!(features[0].files, vec!["src/Login.tsx", "src/api/auth.ts"]);
}

#[tokio::test]
async fn list_active_features_returns_empty_when_no_open_prs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;
    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    let features = client.list_active_features().await.expect("should succeed");
    assert!(features.is_empty());
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test vcs_git_client list_active_features`
Expected: 編譯失敗，`list_active_features` 尚未定義。

- [ ] **Step 3: 實作 `list_active_features` 與私有輔助方法 `pr_files`**

在 `src-tauri/src/vcs/git.rs` 的 `create_pr` 方法後面加入（GitHub API operations 那個區塊裡）：

```rust
    /// 列出目前 repo 所有進行中的功能（open PR，含 draft），每個都附上
    /// 目前實際改動的檔案清單。團隊可見度面板與重疊偵測共用這支方法。
    pub async fn list_active_features(&self) -> Result<Vec<ActiveFeature>, String> {
        let token = self.require_token(2)?;
        let (owner, repo) = self.parse_remote()?;
        let url = format!("{}/repos/{owner}/{repo}/pulls?state=open&per_page=30", self.github_api_base);

        let prs: Vec<GhPrWithHead> = self
            .gh_get(&token, &url)
            .await?
            .json()
            .await
            .map_err(|e| e.to_string())?;

        let mut features = Vec::with_capacity(prs.len());
        for pr in prs {
            let files = self.pr_files(&token, &owner, &repo, pr.number).await?;
            features.push(ActiveFeature {
                number: pr.number,
                title: pr.title,
                author: pr.user.login,
                draft: pr.draft,
                url: pr.html_url,
                updated_at: pr.updated_at,
                head_ref: pr.head.ref_name,
                files,
            });
        }
        Ok(features)
    }

    async fn pr_files(&self, token: &str, owner: &str, repo: &str, pr_number: u64) -> Result<Vec<String>, String> {
        let url = format!("{}/repos/{owner}/{repo}/pulls/{pr_number}/files?per_page=100", self.github_api_base);
        let files: Vec<GhPrFile> = self
            .gh_get(token, &url)
            .await?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        Ok(files.into_iter().map(|f| f.filename).collect())
    }
```

在檔案下方既有的 `GhPr`/`GhUser` 私有 struct 群組附近（約第 613-641 行那一帶）加入新的 deserialize 型別：

```rust
#[derive(Deserialize)]
struct GhPrWithHead {
    number: u64,
    title: String,
    user: GhUser,
    draft: bool,
    html_url: String,
    updated_at: String,
    head: GhPrHead,
}

#[derive(Deserialize)]
struct GhPrHead {
    #[serde(rename = "ref")]
    ref_name: String,
}

#[derive(Deserialize)]
struct GhPrFile {
    filename: String,
}
```

檔案最上方 `use` 區塊（第 8-11 行）把 `ActiveFeature` 加進既有的 import：

```rust
use super::types::{
    ActiveFeature, BlameEntry, BranchEntry, CommitEntry, GitBlockInfo, IssueEntry, PrEntry,
    VcsResult, WorkflowRun,
};
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test vcs_git_client list_active_features`
Expected: 兩個測試都 `ok`。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/vcs/git.rs src-tauri/tests/vcs_git_client.rs
git commit -m "feat(vcs): add GitClient::list_active_features combining open PRs with their changed files"
```

---

## Task 5: `GitClient::mark_pr_ready`

GitHub REST API 沒有「把 draft PR 轉成 ready for review」的端點，只有 GraphQL 的 `markPullRequestReadyForReview` mutation 支援。流程：先用 REST 拿到這個 PR 的 `node_id`，再打 GraphQL mutation。GraphQL 的錯誤是用 HTTP 200 + body 裡的 `errors` 陣列回報，不是靠 HTTP 狀態碼，這點要特別測到。

**Files:**
- Modify: `src-tauri/src/vcs/git.rs`
- Modify: `src-tauri/tests/vcs_git_client.rs`

- [ ] **Step 1: 寫 wiremock 測試（先確認會紅）**

在 `src-tauri/tests/vcs_git_client.rs` 加入：

```rust
#[tokio::test]
async fn mark_pr_ready_fetches_node_id_then_calls_graphql_mutation() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "node_id": "PR_kwABC123"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_json(serde_json::json!({
            "query": "mutation($id: ID!) { markPullRequestReadyForReview(input: {pullRequestId: $id}) { pullRequest { id } } }",
            "variables": { "id": "PR_kwABC123" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "markPullRequestReadyForReview": { "pullRequest": { "id": "PR_kwABC123" } } }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;
    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    client.mark_pr_ready(7).await.expect("should succeed");
}

#[tokio::test]
async fn mark_pr_ready_surfaces_graphql_errors_even_though_http_status_is_200() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "node_id": "PR_kwABC123"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": null,
            "errors": [{"message": "Pull request is already ready for review"}]
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;
    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    let err = client.mark_pr_ready(7).await.unwrap_err();
    assert!(err.contains("already ready for review"), "unexpected error: {err}");
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test vcs_git_client mark_pr_ready`
Expected: 編譯失敗，`mark_pr_ready` 尚未定義。

- [ ] **Step 3: 實作 `mark_pr_ready`**

在 `list_active_features`/`pr_files` 後面加入：

```rust
    /// 把一個 draft PR 轉成 ready for review。GitHub REST 沒有對應端點，
    /// 只能先用 REST 拿 node_id，再打 GraphQL mutation。
    pub async fn mark_pr_ready(&self, pr_number: u64) -> Result<VcsResult, String> {
        let token = self.require_token(3)?;
        let (owner, repo) = self.parse_remote()?;

        let detail_url = format!("{}/repos/{owner}/{repo}/pulls/{pr_number}", self.github_api_base);
        let detail: serde_json::Value = self
            .gh_get(&token, &detail_url)
            .await?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let node_id = detail["node_id"]
            .as_str()
            .ok_or_else(|| "GitHub 回應缺少 node_id".to_string())?;

        let mutation = serde_json::json!({
            "query": "mutation($id: ID!) { markPullRequestReadyForReview(input: {pullRequestId: $id}) { pullRequest { id } } }",
            "variables": { "id": node_id }
        });
        let graphql_url = format!("{}/graphql", self.github_api_base);
        let resp = self.gh_post(&token, &graphql_url, &mutation).await?;
        let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

        // GraphQL 錯誤是 HTTP 200 + body 裡的 errors 陣列，不是靠狀態碼，
        // 所以 gh_post 的狀態碼檢查不會抓到，這裡要自己額外檢查。
        if let Some(errors) = body.get("errors") {
            return Err(format!("GitHub GraphQL error: {errors}"));
        }

        Ok(VcsResult::WriteSuccess {
            operation: "mark_pr_ready".to_string(),
            detail: format!("PR #{pr_number} marked ready for review"),
        })
    }
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test vcs_git_client mark_pr_ready`
Expected: 兩個測試都 `ok`。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/vcs/git.rs src-tauri/tests/vcs_git_client.rs
git commit -m "feat(vcs): add GitClient::mark_pr_ready via GitHub GraphQL"
```

---

## Task 6: `GitClient::pr_diff`

取得整個功能分支相對主線的完整 diff（不是單一 commit），用 GitHub compare 端點、`Accept: application/vnd.github.v3.diff` 拿純文字 diff。

**Files:**
- Modify: `src-tauri/src/vcs/git.rs`
- Modify: `src-tauri/tests/vcs_git_client.rs`

- [ ] **Step 1: 寫 wiremock 測試（先確認會紅）**

在 `src-tauri/tests/vcs_git_client.rs` 加入：

```rust
#[tokio::test]
async fn pr_diff_requests_diff_media_type_and_returns_raw_text() {
    let server = MockServer::start().await;

    let diff_body = "diff --git a/src/Login.tsx b/src/Login.tsx\n+added line\n";
    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/compare/main...feature/login-optimize"))
        .and(header("accept", "application/vnd.github.v3.diff"))
        .respond_with(ResponseTemplate::new(200).set_body_string(diff_body))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;
    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    let result = client.pr_diff("main", "feature/login-optimize").await.expect("should succeed");
    match result {
        aiterm_lib::vcs::VcsResult::Diff { content, .. } => assert_eq!(content, diff_body),
        other => panic!("expected Diff variant, got {other:?}"),
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test vcs_git_client pr_diff_requests_diff_media_type`
Expected: 編譯失敗，`pr_diff` 尚未定義。

- [ ] **Step 3: 實作 `pr_diff`**

在 `mark_pr_ready` 後面加入：

```rust
    /// 取得整個功能分支相對 base 的完整 diff（不是單一 commit）。
    /// 用 GitHub compare 端點，Accept 要求 diff 格式而非 JSON，
    /// 所以不透過 gh_get（它固定要求 application/vnd.github+json）。
    pub async fn pr_diff(&self, base: &str, head: &str) -> Result<VcsResult, String> {
        let token = self.require_token(2)?;
        let (owner, repo) = self.parse_remote()?;
        let url = format!("{}/repos/{owner}/{repo}/compare/{base}...{head}", self.github_api_base);

        let client = reqwest::Client::new();
        let resp = client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(ACCEPT, "application/vnd.github.v3.diff")
            .header(USER_AGENT, "AITerm")
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("GitHub API error {status}: {body}"));
        }
        let content = resp.text().await.map_err(|e| e.to_string())?;
        Ok(VcsResult::Diff { content, revision: format!("{base}...{head}") })
    }
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test vcs_git_client pr_diff_requests_diff_media_type`
Expected: `ok`。

- [ ] **Step 5: 跑過本檔案所有新測試，確認彼此沒有互相干擾**

Run: `cd src-tauri && cargo test --test vcs_git_client`
Expected: 全部通過（Task 3-6 累積的測試，應該有 8 個）。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/vcs/git.rs src-tauri/tests/vcs_git_client.rs
git commit -m "feat(vcs): add GitClient::pr_diff for full-branch diff view"
```

---

## Task 7: Tauri 指令層

把 Task 2-6 的 `GitClient` 方法接成前端能呼叫的指令。新增 `slugify` 純函式（功能名稱轉分支名稱片段）與 6 個新指令。

**Files:**
- Modify: `src-tauri/src/commands/vcs.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 加入 `slugify` 與其單元測試**

在 `src-tauri/src/commands/vcs.rs` 的 `vcs_secret_key` 函式（第 39-41 行）後面加入：

```rust
/// 把功能名稱轉成適合當 git 分支名稱片段的字串：只留英數字元，
/// 其他字元（含中文、空白、標點）都轉成 `-`，並收斂連續的 `-`。
fn slugify(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
```

在檔案最下方既有的 `#[cfg(test)] mod resolve_embedder_config_tests` 之後（該檔案目前沒有這個模組——改成在檔案最下方新增，若之後有其他 vcs command 測試模組沿用這個位置）加入：

```rust
#[cfg(test)]
mod slugify_tests {
    use super::slugify;

    #[test]
    fn ascii_words_become_hyphenated_lowercase() {
        assert_eq!(slugify("Login Fix"), "login-fix");
    }

    #[test]
    fn chinese_characters_are_dropped_not_kept() {
        assert_eq!(slugify("登入頁優化"), "");
    }

    #[test]
    fn mixed_chinese_and_ascii_keeps_only_ascii() {
        assert_eq!(slugify("登入頁 Login 優化"), "login");
    }

    #[test]
    fn consecutive_separators_collapse_to_one_hyphen() {
        assert_eq!(slugify("a   b---c"), "a-b-c");
    }
}
```

- [ ] **Step 2: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib commands::vcs::slugify_tests`
Expected: 4 個測試全過。

（這步驟直接給出正確實作而非先紅——`slugify` 邏輯簡單到不需要刻意先看紅燈；若要嚴格照紅燈流程，可以先把函式體換成 `String::new()` 跑一次確認前三個測試失敗、第四個因為輸入沒有中文而意外通過，再貼上正確版本。）

- [ ] **Step 3: 加入 token/write-mode 解析輔助函式**

在 `slugify` 後面加入：

```rust
fn resolve_vcs_token(repo_info: &VcsRepoInfo, secrets: &SecretStore) -> Option<String> {
    repo_info
        .connection_id
        .as_deref()
        .and_then(|id| secrets.get(&vcs_secret_key(id)).ok().flatten())
}

fn resolve_write_mode(repo_info: &VcsRepoInfo, config: &ConfigStore) -> VcsWriteMode {
    repo_info
        .connection_id
        .as_deref()
        .and_then(|id| {
            config
                .get()
                .vcs_connections
                .into_iter()
                .find(|c| c.id == id)
                .map(|c| c.write_mode)
        })
        .unwrap_or(VcsWriteMode::Guarded)
}
```

- [ ] **Step 4: 加入 `StartFeatureOutcome` 回傳型別與 6 個新指令**

在檔案最後（`pick_folder` 指令之後）加入：

```rust
#[derive(Debug, Serialize)]
pub struct StartFeatureOutcome {
    pub branch_name: String,
    pub pr_number: u64,
    pub pr_url: String,
}

#[tauri::command]
pub async fn vcs_list_active_features(
    repo_info: VcsRepoInfo,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<crate::vcs::ActiveFeature>, String> {
    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);
    client.list_active_features().await
}

#[tauri::command]
pub async fn vcs_check_overlap(
    repo_info: VcsRepoInfo,
    files: Vec<String>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<crate::vcs::ActiveFeature>, String> {
    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);
    let features = client.list_active_features().await?;
    Ok(crate::vcs::overlap::find_overlaps(&files, &features))
}

#[tauri::command]
pub async fn vcs_start_feature(
    repo_info: VcsRepoInfo,
    feature_name: String,
    base_branch: String,
    declared_files: Vec<String>,
    secrets: State<'_, Arc<SecretStore>>,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<StartFeatureOutcome, String> {
    if resolve_write_mode(&repo_info, &config) == VcsWriteMode::ReadOnly {
        return Err("此連線為唯讀模式，無法開始新功能".to_string());
    }

    let slug = slugify(&feature_name);
    if slug.is_empty() {
        return Err("功能名稱需要至少包含一個英文字母或數字".to_string());
    }
    let branch_name = format!("feature/{slug}");

    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);

    client.create_branch(&branch_name, Some(&base_branch)).await?;

    let body = if declared_files.is_empty() {
        String::new()
    } else {
        format!(
            "預計會動到的檔案：\n{}",
            declared_files.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n")
        )
    };
    let (pr_number, pr_url) = client
        .create_pr(&feature_name, &branch_name, &base_branch, Some(&body), true)
        .await?;

    Ok(StartFeatureOutcome { branch_name, pr_number, pr_url })
}

#[tauri::command]
pub async fn vcs_finish_feature(
    repo_info: VcsRepoInfo,
    pr_number: u64,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);
    client.mark_pr_ready(pr_number).await?;
    Ok(())
}

#[tauri::command]
pub async fn vcs_get_feature_diff(
    repo_info: VcsRepoInfo,
    base: String,
    head: String,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);
    match client.pr_diff(&base, &head).await? {
        VcsResult::Diff { content, .. } => Ok(content),
        _ => Err("pr_diff 回傳了非預期的結果型別".to_string()),
    }
}

#[tauri::command]
pub async fn vcs_merge_feature(
    repo_info: VcsRepoInfo,
    pr_number: u64,
    branch_to_delete: Option<String>,
    secrets: State<'_, Arc<SecretStore>>,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<(), String> {
    if resolve_write_mode(&repo_info, &config) == VcsWriteMode::ReadOnly {
        return Err("此連線為唯讀模式，無法合併".to_string());
    }
    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);
    client.merge_pr(pr_number).await?;
    if let Some(branch) = branch_to_delete {
        // 合併已經成功了；刪分支失敗不該讓整個操作回報失敗，忽略即可。
        let _ = client.delete_branch(&branch).await;
    }
    Ok(())
}
```

- [ ] **Step 5: 註冊新指令**

`src-tauri/src/lib.rs` 第 98-100 行的 import 清單，把：

```rust
        pick_folder, vcs_add_connection, vcs_agent_abort_step, vcs_agent_step, vcs_detect_repo,
        vcs_get_block_info, vcs_list_connections, vcs_query, vcs_remove_connection,
        vcs_test_connection, vcs_update_connection, VcsAgentStepRegistry,
```

改成：

```rust
        pick_folder, vcs_add_connection, vcs_agent_abort_step, vcs_agent_step, vcs_check_overlap,
        vcs_detect_repo, vcs_finish_feature, vcs_get_block_info, vcs_get_feature_diff,
        vcs_list_active_features, vcs_list_connections, vcs_merge_feature, vcs_query,
        vcs_remove_connection, vcs_start_feature, vcs_test_connection, vcs_update_connection,
        VcsAgentStepRegistry,
```

第 483-492 行的 `generate_handler!` 清單，把：

```rust
            vcs_list_connections,
            vcs_add_connection,
            vcs_update_connection,
            vcs_remove_connection,
            vcs_test_connection,
            vcs_detect_repo,
            vcs_query,
            vcs_get_block_info,
            vcs_agent_step,
            vcs_agent_abort_step,
```

改成：

```rust
            vcs_list_connections,
            vcs_add_connection,
            vcs_update_connection,
            vcs_remove_connection,
            vcs_test_connection,
            vcs_detect_repo,
            vcs_query,
            vcs_get_block_info,
            vcs_agent_step,
            vcs_agent_abort_step,
            vcs_list_active_features,
            vcs_check_overlap,
            vcs_start_feature,
            vcs_finish_feature,
            vcs_get_feature_diff,
            vcs_merge_feature,
```

- [ ] **Step 6: 全量編譯確認**

Run: `cd src-tauri && cargo check`
Expected: 編譯成功。

- [ ] **Step 7: 跑過整個 vcs 相關測試確認沒有回歸**

Run: `cd src-tauri && cargo test --lib commands::vcs && cargo test --lib vcs:: && cargo test --test vcs_git_client`
Expected: 全部通過。

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/commands/vcs.rs src-tauri/src/lib.rs
git commit -m "feat(vcs): add Tauri commands for the guided start/finish/merge feature flow"
```

---

## Task 8: 前端 IPC 包裝

**Files:**
- Modify: `src/ipc/vcs.ts`

先看一下這個檔案目前的內容再動手（既有型別、既有 `invoke` 包裝的寫法），確保新增的部分風格一致：

- [ ] **Step 1: 讀取現有檔案**

Run（用 Read 工具讀取，不是 shell）：`src/ipc/vcs.ts`

- [ ] **Step 2: 加入新型別與函式**

在檔案裡加入（確切插入位置依現有檔案結構調整，放在既有型別/函式群組附近）：

```typescript
export interface ActiveFeature {
  number: number;
  title: string;
  author: string;
  draft: boolean;
  url: string;
  updated_at: string;
  head_ref: string;
  files: string[];
}

export interface StartFeatureOutcome {
  branch_name: string;
  pr_number: number;
  pr_url: string;
}

export function vcsListActiveFeatures(repoInfo: VcsRepoInfo): Promise<ActiveFeature[]> {
  return invoke<ActiveFeature[]>("vcs_list_active_features", { repoInfo });
}

export function vcsCheckOverlap(repoInfo: VcsRepoInfo, files: string[]): Promise<ActiveFeature[]> {
  return invoke<ActiveFeature[]>("vcs_check_overlap", { repoInfo, files });
}

export function vcsStartFeature(
  repoInfo: VcsRepoInfo,
  featureName: string,
  baseBranch: string,
  declaredFiles: string[],
): Promise<StartFeatureOutcome> {
  return invoke<StartFeatureOutcome>("vcs_start_feature", {
    repoInfo,
    featureName,
    baseBranch,
    declaredFiles,
  });
}

export function vcsFinishFeature(repoInfo: VcsRepoInfo, prNumber: number): Promise<void> {
  return invoke<void>("vcs_finish_feature", { repoInfo, prNumber });
}

export function vcsGetFeatureDiff(repoInfo: VcsRepoInfo, base: string, head: string): Promise<string> {
  return invoke<string>("vcs_get_feature_diff", { repoInfo, base, head });
}

export function vcsMergeFeature(
  repoInfo: VcsRepoInfo,
  prNumber: number,
  branchToDelete?: string | null,
): Promise<void> {
  return invoke<void>("vcs_merge_feature", { repoInfo, prNumber, branchToDelete: branchToDelete ?? null });
}
```

（`VcsRepoInfo` 型別跟 `invoke` import 應該這個檔案已經有了；若命名跟現有的不完全一樣，以現有檔案為準調整。）

- [ ] **Step 3: 型別檢查確認沒有壞掉**

Run: `npx tsc -b`
Expected: 沒有錯誤（這一步只新增匯出，不該影響既有程式碼）。

- [ ] **Step 4: Commit**

```bash
git add src/ipc/vcs.ts
git commit -m "feat(vcs): add IPC wrappers for the guided feature flow commands"
```

---

## Task 9: `useTeamFeatures` hook

**Files:**
- Create: `src/hooks/useTeamFeatures.ts`
- Create: `src/hooks/useTeamFeatures.test.ts`

- [ ] **Step 1: 寫失敗的測試**

Create `src/hooks/useTeamFeatures.test.ts`：

```typescript
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useTeamFeatures } from "./useTeamFeatures";

const REPO_INFO = {
  vcs_type: "git" as const,
  root: "/tmp/repo",
  remote_url: "https://github.com/acme/widget.git",
  connection_id: "conn-1",
};

const FEATURE = {
  number: 7,
  title: "登入頁優化",
  author: "alice",
  draft: true,
  url: "https://github.com/acme/widget/pull/7",
  updated_at: "2026-08-17T00:00:00Z",
  head_ref: "feature/login-optimize",
  files: ["src/Login.tsx"],
};

beforeEach(() => {
  invokeMock.mockReset();
});

describe("useTeamFeatures", () => {
  it("loads active features on mount", async () => {
    invokeMock.mockResolvedValueOnce([FEATURE]);
    const { result } = renderHook(() => useTeamFeatures(REPO_INFO));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.features).toEqual([FEATURE]);
    expect(invokeMock).toHaveBeenCalledWith("vcs_list_active_features", { repoInfo: REPO_INFO });
  });

  it("refresh() re-fetches the list", async () => {
    invokeMock.mockResolvedValueOnce([]);
    const { result } = renderHook(() => useTeamFeatures(REPO_INFO));
    await waitFor(() => expect(result.current.loading).toBe(false));

    invokeMock.mockResolvedValueOnce([FEATURE]);
    await act(async () => {
      await result.current.refresh();
    });

    expect(result.current.features).toEqual([FEATURE]);
  });

  it("surfaces an error message when the fetch fails", async () => {
    invokeMock.mockRejectedValueOnce("此功能需要 GitHub token");
    const { result } = renderHook(() => useTeamFeatures(REPO_INFO));

    await waitFor(() => expect(result.current.error).toBe("此功能需要 GitHub token"));
    expect(result.current.features).toEqual([]);
  });

  it("does nothing when repoInfo is null", async () => {
    const { result } = renderHook(() => useTeamFeatures(null));
    expect(result.current.loading).toBe(false);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/hooks/useTeamFeatures.test.ts`
Expected: 失敗——`./useTeamFeatures` 檔案不存在。

- [ ] **Step 3: 實作 hook**

Create `src/hooks/useTeamFeatures.ts`：

```typescript
import { useCallback, useEffect, useRef, useState } from "react";
import { vcsListActiveFeatures, type ActiveFeature, type VcsRepoInfo } from "../ipc/vcs";

export interface UseTeamFeaturesResult {
  features: ActiveFeature[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

export function useTeamFeatures(repoInfo: VcsRepoInfo | null): UseTeamFeaturesResult {
  const [features, setFeatures] = useState<ActiveFeature[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const refresh = useCallback(async () => {
    if (!repoInfo) return;
    setLoading(true);
    setError(null);
    try {
      const list = await vcsListActiveFeatures(repoInfo);
      if (mountedRef.current) setFeatures(list);
    } catch (e) {
      if (mountedRef.current) setError(String(e));
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [repoInfo]);

  useEffect(() => { void refresh(); }, [refresh]);

  return { features, loading, error, refresh };
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `npx vitest run src/hooks/useTeamFeatures.test.ts`
Expected: 4 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useTeamFeatures.ts src/hooks/useTeamFeatures.test.ts
git commit -m "feat(vcs): add useTeamFeatures hook for the team visibility panel"
```

---

## Task 10: `TeamPanel` 元件

**Files:**
- Create: `src/components/VcsView/TeamPanel.tsx`
- Create: `src/components/VcsView/TeamPanel.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

Create `src/components/VcsView/TeamPanel.test.tsx`：

```typescript
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { TeamPanel } from "./TeamPanel";
import { LocaleProvider } from "../../contexts/LocaleContext";
import type { ActiveFeature } from "../../ipc/vcs";

const FEATURE: ActiveFeature = {
  number: 7,
  title: "登入頁優化",
  author: "alice",
  draft: true,
  url: "https://github.com/acme/widget/pull/7",
  updated_at: "2026-08-17T00:00:00Z",
  head_ref: "feature/login-optimize",
  files: ["src/Login.tsx", "src/api/auth.ts"],
};

function renderPanel(props: Partial<React.ComponentProps<typeof TeamPanel>> = {}) {
  return render(
    <LocaleProvider>
      <TeamPanel
        features={[FEATURE]}
        loading={false}
        onRefresh={vi.fn()}
        onStartFeature={vi.fn()}
        onFinishFeature={vi.fn()}
        {...props}
      />
    </LocaleProvider>
  );
}

describe("TeamPanel", () => {
  it("shows each active feature's title and author", () => {
    renderPanel();
    expect(screen.getByText("登入頁優化")).toBeInTheDocument();
    expect(screen.getByText("alice")).toBeInTheDocument();
  });

  it("expands to show changed files on click", () => {
    renderPanel();
    fireEvent.click(screen.getByText("登入頁優化"));
    expect(screen.getByText("src/Login.tsx")).toBeInTheDocument();
    expect(screen.getByText("src/api/auth.ts")).toBeInTheDocument();
  });

  it("calls onRefresh when the refresh button is clicked", () => {
    const onRefresh = vi.fn();
    renderPanel({ onRefresh });
    fireEvent.click(screen.getByRole("button", { name: /重新整理/ }));
    expect(onRefresh).toHaveBeenCalled();
  });

  it("calls onStartFeature when '開始新功能' is clicked", () => {
    const onStartFeature = vi.fn();
    renderPanel({ onStartFeature });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));
    expect(onStartFeature).toHaveBeenCalled();
  });

  it("shows an empty state when there are no active features", () => {
    renderPanel({ features: [] });
    expect(screen.getByText(/目前沒有進行中的功能/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/components/VcsView/TeamPanel.test.tsx`
Expected: 失敗——`./TeamPanel` 不存在。

- [ ] **Step 3: 實作元件**

Create `src/components/VcsView/TeamPanel.tsx`：

```tsx
import { useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import type { ActiveFeature } from "../../ipc/vcs";

interface Props {
  features: ActiveFeature[];
  loading: boolean;
  onRefresh: () => void;
  onStartFeature: () => void;
  onFinishFeature: (feature: ActiveFeature) => void;
}

export function TeamPanel({ features, loading, onRefresh, onStartFeature, onFinishFeature }: Props) {
  const { t } = useLocale();
  const [expandedNumber, setExpandedNumber] = useState<number | null>(null);

  return (
    <div className="vcs-team-panel">
      <div className="vcs-team-panel__header">
        <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={onStartFeature}>
          {t.vcs_start_feature}
        </button>
        <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onRefresh} disabled={loading}>
          {t.vcs_refresh}
        </button>
      </div>

      {features.length === 0 ? (
        <div className="vcs-team-panel__empty">{t.vcs_no_active_features}</div>
      ) : (
        <ul className="vcs-team-panel__list">
          {features.map((f) => {
            const expanded = expandedNumber === f.number;
            return (
              <li key={f.number} className="vcs-team-panel__item">
                <button
                  className="vcs-team-panel__item-main"
                  onClick={() => setExpandedNumber(expanded ? null : f.number)}
                >
                  <span className="vcs-team-panel__title">{f.title}</span>
                  <span className="vcs-team-panel__author">{f.author}</span>
                  <span className="vcs-team-panel__status">
                    {f.draft ? t.vcs_status_in_progress : t.vcs_status_in_review}
                  </span>
                </button>
                {expanded && (
                  <div className="vcs-team-panel__files">
                    <ul>
                      {f.files.map((file) => (
                        <li key={file}>{file}</li>
                      ))}
                    </ul>
                    {f.draft && (
                      <button
                        className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                        onClick={() => onFinishFeature(f)}
                      >
                        {t.vcs_finish_feature}
                      </button>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
```

- [ ] **Step 4: 加入 i18n 字串（暫時最小化，讓測試能過；完整字串在 Task 14 補齊）**

在 `src/lib/i18n.ts` 的 `zhTW` 物件裡任一 vcs 相關字串附近加入：

```typescript
    vcs_start_feature: "開始新功能",
    vcs_refresh: "重新整理",
    vcs_no_active_features: "目前沒有進行中的功能",
    vcs_status_in_progress: "進行中",
    vcs_status_in_review: "待審核",
    vcs_finish_feature: "完成，送審",
```

（英文版 `en` 物件對應加入 placeholder 英文文案，Task 14 會統一檢查兩邊都補齊。）

- [ ] **Step 5: 執行測試確認通過**

Run: `npx vitest run src/components/VcsView/TeamPanel.test.tsx`
Expected: 5 個測試全過。

- [ ] **Step 6: Commit**

```bash
git add src/components/VcsView/TeamPanel.tsx src/components/VcsView/TeamPanel.test.tsx src/lib/i18n.ts
git commit -m "feat(vcs): add TeamPanel component listing active features"
```

---

## Task 11: `StartFeatureDialog` 元件

「開始新功能」表單：功能名稱＋選填預計檔案，送出前先呼叫 `vcs_check_overlap` 顯示警告。

**Files:**
- Create: `src/components/VcsView/StartFeatureDialog.tsx`
- Create: `src/components/VcsView/StartFeatureDialog.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

Create `src/components/VcsView/StartFeatureDialog.test.tsx`：

```typescript
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { StartFeatureDialog } from "./StartFeatureDialog";
import { LocaleProvider } from "../../contexts/LocaleContext";

const checkOverlapMock = vi.fn();
const startFeatureMock = vi.fn();
vi.mock("../../ipc/vcs", () => ({
  vcsCheckOverlap: (...args: unknown[]) => checkOverlapMock(...args),
  vcsStartFeature: (...args: unknown[]) => startFeatureMock(...args),
}));

const REPO_INFO = {
  vcs_type: "git" as const,
  root: "/tmp/repo",
  remote_url: "https://github.com/acme/widget.git",
  connection_id: "conn-1",
};

beforeEach(() => {
  checkOverlapMock.mockReset();
  startFeatureMock.mockReset();
});

function renderDialog(onStarted = vi.fn(), onClose = vi.fn()) {
  render(
    <LocaleProvider>
      <StartFeatureDialog repoInfo={REPO_INFO} baseBranch="main" onStarted={onStarted} onClose={onClose} />
    </LocaleProvider>
  );
  return { onStarted, onClose };
}

describe("StartFeatureDialog", () => {
  it("starts the feature directly when there is no overlap", async () => {
    checkOverlapMock.mockResolvedValueOnce([]);
    startFeatureMock.mockResolvedValueOnce({
      branch_name: "feature/login-fix",
      pr_number: 7,
      pr_url: "https://github.com/acme/widget/pull/7",
    });
    const { onStarted } = renderDialog();

    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "Login Fix" } });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));

    await waitFor(() => expect(startFeatureMock).toHaveBeenCalledWith(
      REPO_INFO, "Login Fix", "main", [],
    ));
    await waitFor(() => expect(onStarted).toHaveBeenCalled());
  });

  it("shows a warning and waits for confirmation when files overlap", async () => {
    checkOverlapMock.mockResolvedValueOnce([
      { number: 3, title: "Bob 的功能", author: "bob", draft: true, url: "", updated_at: "", head_ref: "", files: ["src/App.tsx"] },
    ]);
    renderDialog();

    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "My Feature" } });
    fireEvent.change(screen.getByLabelText("預計會動到的檔案"), { target: { value: "src/App.tsx" } });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));

    await waitFor(() => expect(screen.getByText(/bob/)).toBeInTheDocument());
    expect(startFeatureMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "仍要開始" }));
    await waitFor(() => expect(startFeatureMock).toHaveBeenCalled());
  });

  it("skips the overlap check entirely when no files are declared", async () => {
    startFeatureMock.mockResolvedValueOnce({
      branch_name: "feature/x", pr_number: 1, pr_url: "",
    });
    renderDialog();

    fireEvent.change(screen.getByLabelText("功能名稱"), { target: { value: "X" } });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));

    await waitFor(() => expect(startFeatureMock).toHaveBeenCalled());
    expect(checkOverlapMock).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/components/VcsView/StartFeatureDialog.test.tsx`
Expected: 失敗——`./StartFeatureDialog` 不存在。

- [ ] **Step 3: 實作元件**

Create `src/components/VcsView/StartFeatureDialog.tsx`：

```tsx
import { useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { vcsCheckOverlap, vcsStartFeature, type ActiveFeature, type VcsRepoInfo } from "../../ipc/vcs";

interface Props {
  repoInfo: VcsRepoInfo;
  baseBranch: string;
  onStarted: () => void;
  onClose: () => void;
}

function parseFileList(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

export function StartFeatureDialog({ repoInfo, baseBranch, onStarted, onClose }: Props) {
  const { t } = useLocale();
  const [name, setName] = useState("");
  const [filesText, setFilesText] = useState("");
  const [overlaps, setOverlaps] = useState<ActiveFeature[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const doStart = async () => {
    setBusy(true);
    setError(null);
    try {
      const outcome = await vcsStartFeature(repoInfo, name, baseBranch, parseFileList(filesText));
      void outcome;
      onStarted();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleSubmit = async () => {
    const declaredFiles = parseFileList(filesText);
    if (declaredFiles.length === 0) {
      await doStart();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const hits = await vcsCheckOverlap(repoInfo, declaredFiles);
      if (hits.length > 0) {
        setOverlaps(hits);
      } else {
        await doStart();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="aiterm-dialog-overlay">
      <div className="aiterm-dialog">
        <h3>{t.vcs_start_feature}</h3>

        <label htmlFor="vcs-feature-name">{t.vcs_feature_name_label}</label>
        <input
          id="vcs-feature-name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          disabled={busy}
        />

        <label htmlFor="vcs-feature-files">{t.vcs_feature_files_label}</label>
        <textarea
          id="vcs-feature-files"
          value={filesText}
          onChange={(e) => setFilesText(e.target.value)}
          disabled={busy}
          rows={4}
        />

        {overlaps && overlaps.length > 0 && (
          <div className="vcs-overlap-warning">
            <p>{t.vcs_overlap_warning}</p>
            <ul>
              {overlaps.map((f) => (
                <li key={f.number}>{t.vcs_overlap_entry(f.author, f.title)}</li>
              ))}
            </ul>
            <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={doStart} disabled={busy}>
              {t.vcs_start_anyway}
            </button>
          </div>
        )}

        {error && <div className="ca-error">{error}</div>}

        <div className="aiterm-dialog__actions">
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onClose} disabled={busy}>
            {t.common_cancel}
          </button>
          {!overlaps && (
            <button
              className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
              onClick={handleSubmit}
              disabled={busy || !name.trim()}
            >
              {t.vcs_start_feature}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: 加入 i18n 字串**

在 `src/lib/i18n.ts` 的 `zhTW` 物件裡加入：

```typescript
    vcs_feature_name_label: "功能名稱",
    vcs_feature_files_label: "預計會動到的檔案（選填，一行一個）",
    vcs_overlap_warning: "這些人目前也在動同樣的檔案：",
    vcs_overlap_entry: (author: string, title: string) => `${author}（${title}）`,
    vcs_start_anyway: "仍要開始",
```

（同步加入 `en` 物件對應英文文案。）

- [ ] **Step 5: 執行測試確認通過**

Run: `npx vitest run src/components/VcsView/StartFeatureDialog.test.tsx`
Expected: 3 個測試全過。

- [ ] **Step 6: Commit**

```bash
git add src/components/VcsView/StartFeatureDialog.tsx src/components/VcsView/StartFeatureDialog.test.tsx src/lib/i18n.ts
git commit -m "feat(vcs): add StartFeatureDialog with proactive overlap warning"
```

---

## Task 12: `FinishFeatureReview` 元件

「完成，送審」後的畫面：顯示完整 diff、按鈕合併。

**Files:**
- Create: `src/components/VcsView/FinishFeatureReview.tsx`
- Create: `src/components/VcsView/FinishFeatureReview.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

Create `src/components/VcsView/FinishFeatureReview.test.tsx`：

```typescript
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { FinishFeatureReview } from "./FinishFeatureReview";
import { LocaleProvider } from "../../contexts/LocaleContext";
import type { ActiveFeature } from "../../ipc/vcs";

const getDiffMock = vi.fn();
const mergeMock = vi.fn();
vi.mock("../../ipc/vcs", () => ({
  vcsGetFeatureDiff: (...args: unknown[]) => getDiffMock(...args),
  vcsMergeFeature: (...args: unknown[]) => mergeMock(...args),
}));

const FEATURE: ActiveFeature = {
  number: 7,
  title: "登入頁優化",
  author: "alice",
  draft: false,
  url: "https://github.com/acme/widget/pull/7",
  updated_at: "2026-08-17T00:00:00Z",
  head_ref: "feature/login-optimize",
  files: ["src/Login.tsx"],
};

const REPO_INFO = {
  vcs_type: "git" as const,
  root: "/tmp/repo",
  remote_url: "https://github.com/acme/widget.git",
  connection_id: "conn-1",
};

beforeEach(() => {
  getDiffMock.mockReset();
  mergeMock.mockReset();
});

describe("FinishFeatureReview", () => {
  it("loads and shows the diff on mount", async () => {
    getDiffMock.mockResolvedValueOnce("diff --git a/x b/x\n+hello\n");
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} baseBranch="main" onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText(/\+hello/)).toBeInTheDocument());
    expect(getDiffMock).toHaveBeenCalledWith(REPO_INFO, "main", "feature/login-optimize");
  });

  it("merges and deletes the branch by default (checkbox starts checked)", async () => {
    getDiffMock.mockResolvedValueOnce("diff content");
    mergeMock.mockResolvedValueOnce(undefined);
    const onMerged = vi.fn();
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} baseBranch="main" onMerged={onMerged} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    expect(screen.getByLabelText("合併後刪除這個功能分支")).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "合併" }));

    await waitFor(() => expect(mergeMock).toHaveBeenCalledWith(REPO_INFO, 7, "feature/login-optimize"));
    await waitFor(() => expect(onMerged).toHaveBeenCalled());
  });

  it("does not delete the branch when the user unchecks the box first", async () => {
    getDiffMock.mockResolvedValueOnce("diff content");
    mergeMock.mockResolvedValueOnce(undefined);
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} baseBranch="main" onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText("合併後刪除這個功能分支"));
    fireEvent.click(screen.getByRole("button", { name: "合併" }));

    await waitFor(() => expect(mergeMock).toHaveBeenCalledWith(REPO_INFO, 7, null));
  });

  it("shows a clear conflict message when merge fails", async () => {
    getDiffMock.mockResolvedValueOnce("diff content");
    mergeMock.mockRejectedValueOnce("GitHub API error 405: Pull Request is not mergeable");
    render(
      <LocaleProvider>
        <FinishFeatureReview repoInfo={REPO_INFO} feature={FEATURE} baseBranch="main" onMerged={vi.fn()} onClose={vi.fn()} />
      </LocaleProvider>
    );

    await waitFor(() => expect(screen.getByText("diff content")).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "合併" }));

    await waitFor(() => expect(screen.getByText(/not mergeable/)).toBeInTheDocument());
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/components/VcsView/FinishFeatureReview.test.tsx`
Expected: 失敗——`./FinishFeatureReview` 不存在。

- [ ] **Step 3: 實作元件**

Create `src/components/VcsView/FinishFeatureReview.tsx`：

```tsx
import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { vcsGetFeatureDiff, vcsMergeFeature, type ActiveFeature, type VcsRepoInfo } from "../../ipc/vcs";

interface Props {
  repoInfo: VcsRepoInfo;
  feature: ActiveFeature;
  baseBranch: string;
  onMerged: () => void;
  onClose: () => void;
}

export function FinishFeatureReview({ repoInfo, feature, baseBranch, onMerged, onClose }: Props) {
  const { t } = useLocale();
  const [diff, setDiff] = useState<string | null>(null);
  const [deleteBranch, setDeleteBranch] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    vcsGetFeatureDiff(repoInfo, baseBranch, feature.head_ref)
      .then(setDiff)
      .catch((e) => setError(String(e)));
  }, [repoInfo, baseBranch, feature.head_ref]);

  const handleMerge = async () => {
    setBusy(true);
    setError(null);
    try {
      await vcsMergeFeature(repoInfo, feature.number, deleteBranch ? feature.head_ref : null);
      onMerged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="aiterm-dialog-overlay">
      <div className="aiterm-dialog aiterm-dialog--wide">
        <h3>{feature.title}</h3>

        {diff === null && !error ? (
          <div>{t.vcs_loading_diff}</div>
        ) : (
          <pre className="vcs-diff-view">{diff}</pre>
        )}

        {error && <div className="ca-error">{error}</div>}

        <label className="vcs-delete-branch-checkbox">
          <input
            type="checkbox"
            checked={deleteBranch}
            onChange={(e) => setDeleteBranch(e.target.checked)}
            disabled={busy}
          />
          {t.vcs_delete_branch_after_merge}
        </label>

        <div className="aiterm-dialog__actions">
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onClose} disabled={busy}>
            {t.common_cancel}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            onClick={handleMerge}
            disabled={busy || diff === null}
          >
            {t.vcs_merge_button}
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: 加入 i18n 字串**

在 `src/lib/i18n.ts` 的 `zhTW` 物件裡加入：

```typescript
    vcs_loading_diff: "載入中…",
    vcs_merge_button: "合併",
    vcs_delete_branch_after_merge: "合併後刪除這個功能分支",
```

（同步加入 `en` 物件對應英文文案。）

- [ ] **Step 5: 執行測試確認通過**

Run: `npx vitest run src/components/VcsView/FinishFeatureReview.test.tsx`
Expected: 3 個測試全過。

- [ ] **Step 6: Commit**

```bash
git add src/components/VcsView/FinishFeatureReview.tsx src/components/VcsView/FinishFeatureReview.test.tsx src/lib/i18n.ts
git commit -m "feat(vcs): add FinishFeatureReview with in-app diff and merge"
```

---

## Task 13: 掛進 `VcsView.tsx`

**Files:**
- Modify: `src/components/VcsView/VcsView.tsx`

- [ ] **Step 1: 讀取現有檔案完整內容**

用 Read 工具讀取 `src/components/VcsView/VcsView.tsx` 全文（先前只讀了前 120 行），確認 `repoInfo`/`writeMode` 等既有變數的確切名稱與 JSX 結構，特別是 `return (` 那段的最外層容器。

- [ ] **Step 2: 加入新的 state 與 hook**

在既有的 `useState`/`useVcsAgentLoop` 附近加入：

```tsx
import { useTeamFeatures } from "../../hooks/useTeamFeatures";
import { TeamPanel } from "./TeamPanel";
import { StartFeatureDialog } from "./StartFeatureDialog";
import { FinishFeatureReview } from "./FinishFeatureReview";
import type { ActiveFeature } from "../../ipc/vcs";

// 在既有 state 宣告群組裡加入：
const { features, loading: featuresLoading, refresh: refreshFeatures } = useTeamFeatures(repoInfo);
const [showStartDialog, setShowStartDialog] = useState(false);
const [reviewingFeature, setReviewingFeature] = useState<ActiveFeature | null>(null);
```

（`repoInfo` 沿用該檔案既有的變數，不重新宣告；若讀取後發現命名不同，以現有檔案為準調整上面的用法。）

- [ ] **Step 3: 在既有的 return JSX 最上方（聊天訊息列表之前）掛入 `TeamPanel`，並在最外層加入兩個對話框的條件渲染**

```tsx
{repoInfo && (
  <TeamPanel
    features={features}
    loading={featuresLoading}
    onRefresh={refreshFeatures}
    onStartFeature={() => setShowStartDialog(true)}
    onFinishFeature={(f) => setReviewingFeature(f)}
  />
)}

{/* ...既有的聊天訊息列表... */}

{showStartDialog && repoInfo && (
  <StartFeatureDialog
    repoInfo={repoInfo}
    baseBranch="main"
    onStarted={() => { setShowStartDialog(false); void refreshFeatures(); }}
    onClose={() => setShowStartDialog(false)}
  />
)}

{reviewingFeature && repoInfo && (
  <FinishFeatureReview
    repoInfo={repoInfo}
    feature={reviewingFeature}
    baseBranch="main"
    onMerged={() => { setReviewingFeature(null); void refreshFeatures(); }}
    onClose={() => setReviewingFeature(null)}
  />
)}
```

（`baseBranch="main"` 先寫死——這個 repo 的預設主分支名稱之後可以從 `repoInfo` 或 `vcs_detect_repo` 擴充取得，目前 spec 沒有涵蓋偵測非 `main` 命名的主分支，是已知的後續加強點，不在本 plan 範圍內。實際插入位置需要對照 Step 1 讀到的檔案結構調整，確保 JSX 巢狀正確。）

- [ ] **Step 4: 型別檢查**

Run: `npx tsc -b`
Expected: 沒有錯誤。

- [ ] **Step 5: 啟動 dev server 手動驗證**

Run: `npm run tauri:dev`（或依專案慣例的啟動方式）

在一個已設定 GitHub token 的 VCS 連線 repo 底下打開版本控制分頁，確認：
- 面板顯示「開始新功能」按鈕與「目前沒有進行中的功能」（或既有的 open PR 清單）
- 點「開始新功能」跳出表單，能送出
- 既有的自然語言對話功能沒有被破壞（先問一句「列出最近的 commit」確認原本的流程還正常）

- [ ] **Step 6: Commit**

```bash
git add src/components/VcsView/VcsView.tsx
git commit -m "feat(vcs): wire TeamPanel and guided start/finish dialogs into VcsView"
```

---

## Task 14: i18n 完整性檢查

前面幾個 task 陸續加了中文字串，英文版可能還缺。這個 task 專門補齊、並確認兩邊 key 對得上。

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 列出這個 plan 新增的所有 key**

```
vcs_start_feature, vcs_refresh, vcs_no_active_features, vcs_status_in_progress,
vcs_status_in_review, vcs_finish_feature, vcs_feature_name_label, vcs_feature_files_label,
vcs_overlap_warning, vcs_overlap_entry, vcs_start_anyway, vcs_loading_diff, vcs_merge_button,
vcs_delete_branch_after_merge
```

- [ ] **Step 2: 確認 `zhTW` 物件裡這些 key 全部存在**

用 grep 檢查：

Run: `grep -c "vcs_start_feature:\|vcs_refresh:\|vcs_no_active_features:\|vcs_status_in_progress:\|vcs_status_in_review:\|vcs_finish_feature:\|vcs_feature_name_label:\|vcs_feature_files_label:\|vcs_overlap_warning:\|vcs_overlap_entry:\|vcs_start_anyway:\|vcs_loading_diff:\|vcs_merge_button:\|vcs_delete_branch_after_merge:" src/lib/i18n.ts`
Expected: `28`（14 個 key，中英各一次）——如果不是 28，代表某個 key 只在其中一個語言物件裡出現，要補上。

- [ ] **Step 3: 補齊缺漏的英文（或中文）字串**

英文版對照文案：

```typescript
    vcs_start_feature: "Start New Feature",
    vcs_refresh: "Refresh",
    vcs_no_active_features: "No features in progress",
    vcs_status_in_progress: "In Progress",
    vcs_status_in_review: "In Review",
    vcs_finish_feature: "Finish & Submit",
    vcs_feature_name_label: "Feature Name",
    vcs_feature_files_label: "Files you expect to touch (optional, one per line)",
    vcs_overlap_warning: "These people are currently working on the same files:",
    vcs_overlap_entry: (author: string, title: string) => `${author} (${title})`,
    vcs_start_anyway: "Start Anyway",
    vcs_loading_diff: "Loading…",
    vcs_merge_button: "Merge",
    vcs_delete_branch_after_merge: "Delete this feature branch after merging",
```

- [ ] **Step 4: 型別檢查與測試全量跑過**

Run: `npx tsc -b && npx vitest run`
Expected: 型別檢查通過；所有測試（含這個 plan 新增的 4 個測試檔）通過。

- [ ] **Step 5: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "chore(vcs): fill in missing English strings for the team visibility feature"
```

---

## 完成後驗證

- [ ] `cd src-tauri && cargo test --lib vcs:: && cargo test --lib commands::vcs && cargo test --test vcs_git_client` 全綠
- [ ] `npx vitest run` 全綠
- [ ] `npx tsc -b` 無錯誤
- [ ] `cd src-tauri && cargo check` 無錯誤
- [ ] 手動驗證：一個設定好 GitHub token 的 repo，走一遍「開始新功能 → （模擬另一人已經在改同檔案）看到重疊警告 → 仍要開始 → 完成送審 → 看到 diff → 合併」全流程

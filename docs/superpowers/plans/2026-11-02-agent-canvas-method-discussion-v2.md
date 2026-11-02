# 方法討論 v2：真的 Claude Code session 當討論通道

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**為什麼改：** v1（`2026-11-02-agent-canvas-method-discussion.md`）用 `invokeAiComplete` 純文字補全當對話通道，實機驗收發現：(1) AI 讀不到參考資料夾，無法解析舊系統，連自己都說「沒辦法讀取檔案」；(2) 版面是自刻的對話串，會被決定卡蓋住、標題重複，不專業。使用者要求討論階段要有「單一 Session 的完整能力」。

**Goal:** 勾「先討論做法」後，開一個真的 Claude Code session（工作目錄是程式庫、參考資料夾用 `--add-dir` 掛進去、可讀檔跑指令），使用者在該 session 與 AI 討論；AI 把做法決定寫進一個 JSON 檔，AITerm 畫面讀到後顯示決定卡，使用者按確認才建團隊。

**Architecture:**
- 後端新增 `tasks/method_discussion.rs`（路徑、id 驗證、提示詞填入、讀決定檔；純函式＋測試）與兩個命令：`agents_method_discussion_start`（重用 `dispatch::spawn_and_run` 開 claude 分頁，跟派工同一條路，分頁會以「Agent: claude」出現在終端機）與 `agents_method_discussion_read`（讀決定檔文字）。
- 前端 `MethodDiscussion` 不再自己畫對話：只有「開場說明＋開始討論」→「討論進行中」→「決定卡＋確認」三個狀態，輪詢決定檔。對話在 Claude Code 分頁裡進行。
- 提示詞在前端組（`buildDiscussionPrompt`），含 `{{DECISION_PATH}}` 占位，後端填入真實路徑並附上參考資料夾清單（`references::prompt_note`）。
- 決定檔放 `<專案>/tasks/discussion-<id>/decision.json`（跟 `verify.json` 同一套位置），不污染程式庫。

**已經有的、直接重用：** `dispatch::spawn_and_run`／`launch_command_with`／`looks_like_claude`、`references::{get, add_dir_args, deny_edit_rules, settings_with_deny, prompt_note}`、`quota::settings_flag`、`tasks::task_dir`、`startFromRequirement` 的 `decision` 參數、`MethodDecision` 型別、`formatDecision`。

**要刪掉的（v1 的對話通道，不再使用）：** `buildDiscussionSystem`、`buildDiscussionMessage`、`DiscussionTurn`、`parseDecision`（從回覆抓 ```` ```decision ```` 區塊）、元件裡的對話串與 `complete`／`getContext` props、字串 `ac_md_thinking`／`ac_md_error`（若改版後沒用到）。`startFromRequirement`、i18n 其他字串、`RequirementStart` 的開關保留。

**範圍界線：**
- 不做「在 AITerm 內嵌終端機」：對話在 Claude Code 分頁，使用者自己切過去。
- 非 Claude 的派工指令（`looks_like_claude` 為 false）直接回錯誤，不處理 Codex。
- 一個專案同時只追蹤一個討論；重新「開始討論」會開新 session 與新決定檔。

---

### Task 1: 前端純函式改為「提示詞＋決定檔解析」

**Files:** Modify `src/components/AgentCanvas/methodDecision.ts`、`methodDecision.test.ts`

- [ ] **Step 1: 改測試（先紅）。** 移除 `parseDecision`／`buildDiscussionMessage` 的測試；新增：

```ts
import { buildDiscussionPrompt, DECISION_PATH_PLACEHOLDER, formatDecision, parseDecisionJson, type MethodDecision } from "./methodDecision";

const good: MethodDecision = { approach: "batch", reason: "533 支畫面骨架相同", trial: "先試 5 支，看通過率", requirement: "把 Cm 畫面轉成網頁" };

describe("parseDecisionJson", () => {
  it("reads a valid decision file", () => {
    expect(parseDecisionJson(JSON.stringify(good))).toEqual(good);
  });
  it("returns null for malformed JSON (the file may be half-written)", () => {
    expect(parseDecisionJson("{oops")).toBeNull();
  });
  it("returns null for empty text", () => {
    expect(parseDecisionJson("")).toBeNull();
  });
  it("rejects an unknown approach", () => {
    expect(parseDecisionJson(JSON.stringify({ ...good, approach: "magic" }))).toBeNull();
  });
  it("rejects an empty requirement", () => {
    expect(parseDecisionJson(JSON.stringify({ ...good, requirement: "  " }))).toBeNull();
  });
  it("accepts reason and trial being absent", () => {
    expect(parseDecisionJson(JSON.stringify({ approach: "single", requirement: "x" }))).toEqual({ approach: "single", reason: "", trial: "", requirement: "x" });
  });
});

describe("buildDiscussionPrompt", () => {
  it("keeps the placeholder exactly once so the backend can fill the real path", () => {
    const p = buildDiscussionPrompt("要轉換舊平台");
    expect(p.split(DECISION_PATH_PLACEHOLDER).length - 1).toBe(1);
  });
  it("includes the user's opening and the file schema", () => {
    const p = buildDiscussionPrompt("要轉換舊平台");
    expect(p).toContain("要轉換舊平台");
    expect(p).toContain('"approach"');
    expect(p).toContain("single|batch|generator");
  });
  it("tells the AI it may read the reference folders and must not claim it cannot", () => {
    expect(buildDiscussionPrompt("x")).toContain("先讀");
  });
});
```
保留 `formatDecision` 的既有測試。

- [ ] **Step 2: 跑測試確認紅。** `npx vitest run src/components/AgentCanvas/methodDecision.test.ts`

- [ ] **Step 3: 實作。** 刪 `buildDiscussionSystem`、`buildDiscussionMessage`、`DiscussionTurn`、`parseDecision`、`BLOCK`、不再用到的 `Reference` import。新增並匯出：

```ts
/** 後端會把它換成決定檔的真實路徑。 */
export const DECISION_PATH_PLACEHOLDER = "{{DECISION_PATH}}";

/** 驗證並整理決定；不合格式回 null。`parseDecisionJson` 只是它的 JSON 入口。 */
function toDecision(raw: unknown): MethodDecision | null {
  if (!raw || typeof raw !== "object") return null;
  const o = raw as Record<string, unknown>;
  const approach = o.approach;
  const requirement = typeof o.requirement === "string" ? o.requirement.trim() : "";
  if (typeof approach !== "string" || !APPROACHES.includes(approach as Approach) || !requirement) return null;
  return {
    approach: approach as Approach,
    reason: typeof o.reason === "string" ? o.reason.trim() : "",
    trial: typeof o.trial === "string" ? o.trial.trim() : "",
    requirement,
  };
}

/** 決定檔可能寫到一半或還沒有：任何讀不懂的內容都回 null（當作還在討論）。 */
export function parseDecisionJson(text: string): MethodDecision | null {
  if (!text.trim()) return null;
  try {
    return toDecision(JSON.parse(text));
  } catch {
    return null;
  }
}

/** 開給 Claude Code session 的第一則提示：角色、判斷標準、決定檔格式。 */
export function buildDiscussionPrompt(opening: string): string {
  return [
    "你在協助使用者決定 AITerm「Agent 協作」這批工作該怎麼做，先討論、不要動手改任何程式。",
    "你可以也應該直接讀程式庫與參考資料夾（先讀它們的 CLAUDE.md／README，再看實際檔案）、跑唯讀指令做統計，再提建議；不要說你讀不到。",
    "",
    "可選的做法：",
    ...APPROACHES.map((a) => `- ${a}：${APPROACH_LABEL[a]}`),
    "",
    "批次只在四項全符合時才建議：有明確清單且通常 20 項以上；每項用同一份 template；驗證方式相同且可機械判斷；項目之間沒有依賴。",
    "能被產生器涵蓋的部分，優先建議先做產生器；批次只處理產生器處理不了的例外項目。",
    "不確定就先看資料、再問使用者，一次問一到兩個問題。",
    "",
    `當你有把握提案時，用 Write 工具把下面格式的 JSON 寫到 ${DECISION_PATH_PLACEHOLDER}（整份覆寫），然後用使用者的語言簡短說明，並請使用者回到 AITerm 的 Agent 協作畫面按「確認」：`,
    '{"approach":"single|batch|generator","reason":"為什麼，要引用你看到的實際資料","trial":"試跑計畫與驗證方式，沒有就留空字串","requirement":"整理後的完整需求"}',
    "使用者想調整時繼續討論，之後覆寫同一個檔案；以最後一次寫入為準。",
    "",
    `使用者的開場說明：\n${opening.trim()}`,
  ].join("\n");
}
```

- [ ] **Step 4: 綠。** 同上指令全過；突變：把 `toDecision` 裡 `|| !requirement` 拿掉，確認「rejects an empty requirement」紅，還原。`npx tsc -b` 預期此時 `MethodDiscussion.tsx` 會因 import 已刪除的符號而報錯——這是預期的，Task 4 修；為了讓 commit 可獨立編譯，**本 Task 暫時保留** `MethodDiscussion.tsx` 對舊符號的使用不可行，所以改為：Task 1 與 Task 4 之間不跑 tsc，Task 1 commit 訊息註明「元件於 Task 4 改寫」。

- [ ] **Step 5: Commit** `git add` 兩個檔案；`refactor(agent-canvas): 方法討論改用 Claude Code session 的提示詞與決定檔解析`。

---

### Task 2: 後端 `method_discussion` 模組（純函式）

**Files:** Create `src-tauri/src/tasks/method_discussion.rs`；Modify `src-tauri/src/tasks/mod.rs`（`pub mod method_discussion;`，找到其他 `pub mod` 的位置照排）

- [ ] **Step 1: 先寫測試（模組內 `#[cfg(test)]`，檔尾）**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_path_lives_under_the_project_tasks_dir() {
        let p = decision_path(Path::new("/proj"), "abc-123");
        assert_eq!(p, Path::new("/proj").join("tasks").join("discussion-abc-123").join("decision.json"));
    }

    #[test]
    fn valid_id_accepts_uuid_and_rejects_path_tricks() {
        assert!(valid_id("0b9f6d2e-1c3a-4f55-9a7e-2d1e8c3b4a5f"));
        for bad in ["", "../x", "a/b", "a\\b", "a b", "x.json", &"a".repeat(65)] {
            assert!(!valid_id(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn fill_prompt_replaces_every_placeholder() {
        let out = fill_prompt("寫到 {{DECISION_PATH}} 再說 {{DECISION_PATH}}", Path::new("/p/d.json"));
        assert_eq!(out, "寫到 /p/d.json 再說 /p/d.json");
    }

    #[test]
    fn read_decision_is_none_until_the_file_exists_and_rejects_bad_ids() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_decision(dir.path(), "abc"), None);
        let path = decision_path(dir.path(), "abc");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{\"approach\":\"single\"}").unwrap();
        assert_eq!(read_decision(dir.path(), "abc").as_deref(), Some("{\"approach\":\"single\"}"));
        assert_eq!(read_decision(dir.path(), "../abc"), None);
    }

    #[test]
    fn settings_json_denies_edits_in_reference_dirs() {
        let refs = vec![crate::tasks::references::Reference { path: "/old/src".into(), note: String::new() }];
        let s = settings_json(&refs);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["permissions"]["deny"][0], "Edit(//old/src/**)");
    }
}
```

- [ ] **Step 2: 跑測試確認紅。** `cd src-tauri && cargo test -p app method_discussion --lib`（若 package 名不是 `app`，用 `grep -n '^name' Cargo.toml` 確認）。預期：編譯失敗（函式未定義）。若缺 `tauri-build` 的 sidecar 資源，先確認 `src-tauri/binaries/` symlink 在（worktree 已建）。

- [ ] **Step 3: 實作**

```rust
//! 建團隊前的「方法討論」：開一個真的 Claude Code session（工作目錄是程式庫、參考資料夾用 `--add-dir`
//! 掛進去），使用者在那裡跟 AI 討論；AI 把做法決定寫進 `<專案>/tasks/discussion-<id>/decision.json`，
//! 畫面讀到後顯示決定卡，使用者確認才建團隊。這裡只放路徑與提示詞的純函式，開分頁走 `dispatch::spawn_and_run`。

use std::path::{Path, PathBuf};

use crate::tasks::references::Reference;

/// 前端提示詞裡的占位（見 `methodDecision.ts` 的 `DECISION_PATH_PLACEHOLDER`），啟動時換成真實路徑。
pub const PATH_PLACEHOLDER: &str = "{{DECISION_PATH}}";

/// 討論用的資料夾：跟卡片的 `tasks/<id>/` 同一層，專案搬家時一起帶走。
pub fn discussion_dir(project_path: &Path, id: &str) -> PathBuf {
    crate::tasks::task_dir(project_path, &format!("discussion-{id}"))
}

pub fn decision_path(project_path: &Path, id: &str) -> PathBuf {
    discussion_dir(project_path, id).join("decision.json")
}

/// id 只能是字母數字與連字號：它會被拿來組路徑，不能讓呼叫端用 `../` 讀到專案外。
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn fill_prompt(template: &str, decision_path: &Path) -> String {
    template.replace(PATH_PLACEHOLDER, &decision_path.to_string_lossy())
}

/// 決定檔的文字；還沒有、或 id 不合法就是 `None`。解析與驗證交給前端（檔案可能寫到一半）。
pub fn read_decision(project_path: &Path, id: &str) -> Option<String> {
    if !valid_id(id) {
        return None;
    }
    std::fs::read_to_string(decision_path(project_path, id)).ok()
}

/// 討論 session 的設定檔：禁止用內建工具編輯參考資料夾（跟派工同一條規則）。
pub fn settings_json(refs: &[Reference]) -> String {
    crate::tasks::references::settings_with_deny("{}", &crate::tasks::references::deny_edit_rules(refs))
}
```

- [ ] **Step 4: 綠。** 同指令全過；突變：把 `valid_id` 的 `|| c == '-'` 拿掉，確認 UUID 那條紅，還原。

- [ ] **Step 5: Commit** `git add src-tauri/src/tasks/method_discussion.rs src-tauri/src/tasks/mod.rs`；`feat(agent-canvas): 方法討論的路徑與決定檔讀取（後端純函式）`。

---

### Task 3: 後端命令與前端 IPC

**Files:** Modify `src-tauri/src/commands/agents.rs`（命令）、`src-tauri/src/lib.rs`（兩處註冊：第 134 行的 `use` 清單與第 841 行附近的 `invoke_handler`）、`src/ipc/agents.ts`（包裝）

- [ ] **Step 1: 讀懂既有慣例。** 讀 `commands/agents.rs` 裡 `project(&reg, &id)` 與其他用到 `State<'_, Arc<PtyManager>>`、設定（`task_board.claude_command`，見 `tasks/scheduler.rs:883`）的命令，用同一種取得方式。回報你用的是哪個 State 型別。

- [ ] **Step 2: 實作命令**

```rust
#[derive(serde::Serialize)]
pub struct MethodDiscussionStarted {
    pub discussion_id: String,
    pub tab_id: String,
}

/// 建團隊前的方法討論：開一個真的 Claude Code 分頁（工作目錄是程式庫，參考資料夾唯讀掛進去）。
/// `prompt` 內的 `{{DECISION_PATH}}` 會換成決定檔的路徑；AI 寫了那個檔，`agents_method_discussion_read` 就讀得到。
#[tauri::command]
pub async fn agents_method_discussion_start(
    project_id: String,
    repo_dir: String,
    prompt: String,
    reg: State<'_, ProjectRegistry>,
    /* config、pty 的 State：照 Step 1 找到的慣例 */
    app: AppHandle,
) -> Result<MethodDiscussionStarted, String> {
    let p = project(&reg, &project_id)?;
    let claude_cmd = /* config.get().task_board.claude_command */;
    if !crate::tasks::dispatch::looks_like_claude(&claude_cmd) {
        return Err(format!("方法討論需要 Claude Code：目前的派工指令是 `{claude_cmd}`。請到設定把派工指令改成 claude。"));
    }
    let refs = crate::tasks::references::get(&p.pool).await.map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let dir = crate::tasks::method_discussion::discussion_dir(&p.path, &id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = crate::tasks::method_discussion::decision_path(&p.path, &id);
    let mut full = crate::tasks::method_discussion::fill_prompt(&prompt, &path);
    full.push_str(&crate::tasks::references::prompt_note(&refs));
    let settings = dir.join("claude-settings.json");
    std::fs::write(&settings, crate::tasks::method_discussion::settings_json(&refs)).map_err(|e| e.to_string())?;
    let extra = [
        crate::tasks::quota::settings_flag(&settings),
        crate::tasks::references::add_dir_args(&refs),
        // 決定檔所在資料夾也要能寫：它在程式庫之外
        crate::tasks::quota::path_flag("--add-dir", &dir),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let session_id = uuid::Uuid::new_v4().to_string();
    let (tab_id, _) = crate::tasks::dispatch::spawn_and_run(
        &app, &pty, &repo_dir, &claude_cmd, Some(&session_id), &full,
        false, None, Some(&extra), false, |_| {},
    )
    .await?;
    Ok(MethodDiscussionStarted { discussion_id: id, tab_id })
}

/// 決定檔的文字；AI 還沒寫就是 `None`。
#[tauri::command]
pub async fn agents_method_discussion_read(
    project_id: String,
    discussion_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Option<String>, String> {
    let p = project(&reg, &project_id)?;
    Ok(crate::tasks::method_discussion::read_decision(&p.path, &discussion_id))
}
```
（`/* … */` 處依 Step 1 的發現填成實際程式碼，不可留註解。`extra` 為空字串時傳 `None`。）

- [ ] **Step 3: 註冊兩個命令**（lib.rs 兩處，緊跟 `agents_get_references` 之後）。

- [ ] **Step 4: 前端包裝（`src/ipc/agents.ts`，緊跟 `getReferences` 之後）**

```ts
export interface MethodDiscussionStarted {
  discussion_id: string;
  tab_id: string;
}

/** 建團隊前的方法討論：開一個真的 Claude Code 分頁。prompt 內的 `{{DECISION_PATH}}` 由後端換成決定檔路徑。 */
export const startMethodDiscussion = (projectId: string, repoDir: string, prompt: string): Promise<MethodDiscussionStarted> =>
  invoke("agents_method_discussion_start", { projectId, repoDir, prompt });

/** 決定檔的文字；AI 還沒寫就是 null。 */
export const readMethodDecision = (projectId: string, discussionId: string): Promise<string | null> =>
  invoke("agents_method_discussion_read", { projectId, discussionId });
```
並在 `src/ipc/agents.test.ts`（若有，沒有就新建）補兩條：呼叫 `invoke` 的命令名稱與參數鍵（`projectId`、`repoDir`、`prompt`、`discussionId`）正確。

- [ ] **Step 5: 驗證。** `cd src-tauri && cargo check -p app` 與 `cargo test -p app method_discussion --lib`；`npx vitest run src/ipc`；`npx tsc -b` 此時仍會因 Task 4 未做而報 `MethodDiscussion` 的錯，只確認**沒有新增來自 ipc 檔的錯誤**。

- [ ] **Step 6: Commit** `feat(agent-canvas): 方法討論的後端命令（開 Claude Code 分頁、讀決定檔）`。

---

### Task 4: 前端元件改寫、版面與接線

**Files:** Modify `MethodDiscussion.tsx`、`MethodDiscussion.test.tsx`、`RequirementStart.tsx`、`RequirementStart.test.tsx`、`index.css`、`src/lib/i18n.ts`（zhTW＋enRaw）

**元件規格（取代 v1 的對話串）：**
- props：`start(opening: string): Promise<{ discussionId: string }>`、`read(discussionId: string): Promise<string | null>`、`busy`、`confirmLocked`、`onConfirm(d)`、`onBack()`、`pollMs?`（預設 2000，測試傳小值）。
- 三個狀態：
  1. **idle**：一個說明段落（「會開一個 Claude Code 分頁，AI 能讀你的程式庫與參考資料夾；在那裡討論，有結論後這裡會出現做法決定卡」）、開場說明 textarea（testid `ac-md-input`）、「開始討論」鈕（testid `ac-md-start`，空白時灰）、「改回直接送出」（`ac-md-back`）。
  2. **running**（呼叫 `start` 成功後）：狀態列「討論進行中：請到 Claude Code 分頁（Agent: claude）與 AI 討論」＋ 每 `pollMs` 呼叫 `read`，`parseDecisionJson` 通過就存 `decision`；「重新開始討論」鈕（回到 idle、停止輪詢，舊決定清掉）。
  3. **decision**：決定卡（`ac-md-decision`）＋「確認，建立團隊」（`ac-md-confirm`，`busy || confirmLocked` 時灰）。輪詢**繼續**（AI 可能再調整覆寫），卡片會更新。
- `start` 失敗：顯示 `t.ac_md_error(msg)`，留在 idle。
- 卸載時清掉輪詢（`mountedRef`／`clearInterval`），輪詢結果在卸載後不得 setState。
- **版面**：整個元件是一個垂直堆疊，沒有固定高度的捲動區，也沒有第二個 `<h3>`（標題只有 `RequirementStart` 的「你想做什麼？」）；決定卡在流程裡自然往下長，不覆蓋任何東西。`.ac-md-thread`、`.ac-md-turn*` 樣式整個刪除，改 `.ac-md`（`display:flex; flex-direction:column; gap`）、`.ac-md-status`、`.ac-md-decision`（邊框卡片，沿用 `.ac-req-*` 的變數與間距）。

- [ ] **Step 1: i18n** zhTW／enRaw 同步：新增 `ac_md_intro`、`ac_md_start`、`ac_md_starting`、`ac_md_running`、`ac_md_restart`；刪除不再用的 `ac_md_thinking`、`ac_md_send`；`ac_md_placeholder` 改成開場說明的 placeholder（「描述這批工作：有多大、每項是否同型；AI 會自己去讀程式庫與參考資料夾」，en 對應）。`ac_md_error` 保留。確認 zh/en key 集合相同（用 grep 自己比，因為 en 是合併出來的）。
- [ ] **Step 2: 測試先紅**（`MethodDiscussion.test.tsx` 整檔重寫，用假的 `start`／`read`，`pollMs: 10`）：idle 時「開始討論」在空白時灰；按下後呼叫 `start(opening)`、進入 running；`read` 先回 null 再回有效 JSON → 出現決定卡且確認可按；`read` 回壞 JSON → 不出現決定卡；決定卡出現後 `read` 回新內容 → 卡片更新；`confirmLocked` → 確認灰但卡片仍顯示；`start` 失敗 → 顯示錯誤、仍在 idle；卸載後不再呼叫 `read`；「重新開始討論」清掉決定卡並停止輪詢。
- [ ] **Step 3: 實作元件**，跑到綠；兩個突變（拿掉輪詢清理 → 卸載那條紅；拿掉 `|| confirmLocked` → 鎖住那條紅），各自還原。
- [ ] **Step 4: 接 `RequirementStart`**：討論模式傳 `start={(opening) => startMethodDiscussion(projectId, repo, buildDiscussionPrompt(opening)).then((r) => ({ discussionId: r.discussion_id }))}`（`repo` 為 null 時 `start` 直接丟錯「先選程式庫」，用 `t.ac_req_repo_pick`）、`read={(id) => readMethodDecision(projectId, id)}`；`confirmLocked={!repoOk}` 維持。`RequirementStart.test.tsx` 的 mock 改掛 `startMethodDiscussion`／`readMethodDecision`（不再需要 `invokeAiComplete` 的討論路徑）；「確認後建團隊、需求卡含做法決定」那條測試改為由 `read` 回傳決定。
- [ ] **Step 5: 清理。** 全專案 grep 確認 `buildDiscussionSystem`、`buildDiscussionMessage`、`parseDecision(`、`ac-md-thread`、`ac_md_thinking`、`ac_md_send` 都已無引用。
- [ ] **Step 6: 全面驗證。** `npx tsc -b`、`npx eslint`（本次改動檔）、`npx vitest run src/components/AgentCanvas src/ipc src/lib`、`cd src-tauri && cargo test --workspace --no-fail-fast` 看每一行 `test result:`（pty 的 openpty 競爭失敗是已知環境性問題，只要失敗的不是本次改動相關就在回報註明）。
- [ ] **Step 7: Commit** `feat(agent-canvas): 方法討論改用 Claude Code 分頁，畫面只顯示決定卡`。

---

### Task 5: 實機驗收（使用者）

1. 重新 `npm run tauri:dev`（Rust 有改，要重編）。
2. 專案已有參考資料夾（例如 HTBTW_Source）。勾「先討論做法再開始」，輸入開場說明，按「開始討論」。
3. **預期**：終端機多一個 Claude Code 分頁；它真的去讀參考資料夾（看得到它在讀檔），不會說「沒辦法讀取」。
4. 討論到它提案後，**預期**回到 Agent 協作畫面看到決定卡，引用的是它實際看到的資料；按確認建團隊，需求卡與簡介帶有「做法決定：」。
5. 要觀察並回報：分頁是否自動切過去或需要手動切；Claude Code 是否真的把 JSON 寫到 `決定檔` 並提醒你回畫面；是否有權限詢問卡住（`--add-dir` 與設定檔是否擋到寫決定檔）。

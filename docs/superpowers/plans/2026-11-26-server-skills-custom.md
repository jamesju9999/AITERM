# Server 自訂技能（M0＋M1）實作計畫

> 對應 spec：`docs/superpowers/specs/2026-11-26-server-skills-custom-design.md`。每個任務先寫會紅的測試，再實作。

**Goal:** 共用看板的 Agent 能在 server 上使用自訂技能（上傳、管理、勾選、派工附帶）。
**Architecture:** core 存技能 metadata 與 Agent 的 `custom_skills`；server 新增 `skills.rs`（驗證、落地、複製副本）與 `routes_skills.rs`；runner 呼叫處統一用 `SkillCtx` 取得 `--add-dir` 與提示詞；桌面版新增 Tauri 指令與「Server 技能庫」對話框。

## 任務

1. **core：欄位與表**（`aiterm-intake-core/src/board_store.rs`）
   - `board_agents.custom_skills`、`board_skills` 表；`BoardAgent.custom_skills: Option<String>`；`BoardSkill` 結構；
     `upsert_skill`（同名保留 id）、`list_skills`、`get_skill`、`delete_skill`、`set_agent_skills`（驗證 id 存在、≤12）。
   - 測試：同名取代保留 id、刪除、set_agent_skills 拒絕不存在 id／超過 12。
2. **server：`skills.rs`**
   - `validate_upload`（路徑、上限、必須有 SKILL.md、frontmatter）、`store`（暫存→換）、`copy_to_run`（副本）、`SkillCtx`（dirs＋note＋Drop 清理）、啟動清殘留暫存。
   - 測試：壞路徑、超限、取代保留 id、副本內容一致、Drop 清掉。
3. **server：`routes_skills.rs` 與路由**
   - GET／POST／DELETE `/v1/skills`、PUT `/v1/board/agents/{id}/skills`；POST 主體上限 16 MB；管理員檢查；稽核；`st.events.changed()`。
   - 測試（`tests/skills_api.rs`）：權限、超限、壞路徑、設定 Agent 技能、刪除後 Agent 讀到的清單被過濾。
4. **server：派工接線**（`orchestrator.rs`、`verifier.rs`）
   - work／fix／review／plan／verify 的 `exec_claude_*` 都傳 `SkillCtx.dirs`，提示詞附 `SkillCtx.note`。
   - 測試（`tests/skills_run.rs`，假 claude）：參數含 `--add-dir`、提示含清單、副本結束後刪除、改副本不影響技能庫。
5. **桌面版 Rust：`board_remote.rs`、`commands/agents.rs`、`lib.rs`**
   - 四個 Tauri 指令；`to_agent` 把 `custom_skills` 帶過來；資料夾讀取（略過 `.git` 等、上限）。
   - 測試：資料夾讀取與上限、上傳打到假 server。
6. **前端**
   - `ipc/serverSkills.ts`、`ServerSkillsDialog.tsx`（勾選／上傳／刪除／成員唯讀）、`index.tsx` 技能區在共用專案只顯示自訂技能、跳過 `missingSkills`、名稱對照。
   - i18n zh／en；測試。
7. **驗證與文件**
   - 全套測試（cargo `--workspace --no-fail-fast`、vitest、tsc、eslint）。
   - 真實 Claude Code：技能裡放暗號，Agent 照做；再用畫面上傳與勾選。
   - `docs/intake-server.md` 加「Server 技能」一節、CHANGELOG。

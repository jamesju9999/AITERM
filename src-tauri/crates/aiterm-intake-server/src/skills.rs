//! server 的技能庫：把桌面版上傳的技能資料夾落地、管理，並在派工時給 Agent 一份一次性的副本。
//!
//! 為什麼給副本而不是技能庫路徑：work 卡有 Edit 權限，直接把共用路徑交給 Claude，Agent 就能改到所有人共用的技能。
//! 副本在 `<data>/run/<card id>/skills/`，卡結束（或這個值被丟掉）就刪。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aiterm_intake_core::board_store::{self as bs, BoardAgent, BoardSkill};
use aiterm_intake_core::skills as rules;

use crate::state::AppState;

fn root(data_dir: &Path) -> PathBuf {
    data_dir.join("skills")
}

/// 這個技能的檔案資料夾。
pub fn skill_dir(data_dir: &Path, id: &str) -> PathBuf {
    root(data_dir).join(id)
}

/// 技能 id 是我們自己產生的 uuid，但路徑組合前還是驗證，免得 API 傳進奇怪的字串。
pub fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}

fn write_tree(dir: &Path, files: &[(String, Vec<u8>)]) -> std::io::Result<()> {
    for (rel, data) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)?;
    }
    Ok(())
}

/// 收下一個上傳的技能：檢查、寫到暫存資料夾、登記、再換進正式位置。同名＝取代內容、保留 id。
/// `files` 是（相對路徑, 內容）；`.git`、`node_modules` 之類會被略過。
pub async fn store_upload(st: &AppState, folder_name: &str, files: Vec<(String, Vec<u8>)>, by: &str) -> Result<BoardSkill, String> {
    let mut kept: Vec<(String, Vec<u8>)> = Vec::new();
    for (raw, data) in files {
        let path = rules::clean_path(&raw)?;
        if !rules::skippable(&path) {
            kept.push((path, data));
        }
    }
    let sizes: Vec<(String, usize)> = kept.iter().map(|(p, d)| (p.clone(), d.len())).collect();
    rules::check_files(&sizes)?;
    let skill_md = kept.iter().find(|(p, _)| p == rules::SKILL_FILE).map(|(_, d)| String::from_utf8_lossy(d).into_owned()).unwrap_or_default();
    let fallback = rules::safe_dir_name(folder_name);
    let (name, description) = rules::parse_frontmatter(&skill_md, &fallback);
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(|c| c.is_control()) {
        return Err("技能名稱不能是空的、超過 80 字或含控制字元".into());
    }
    let (files_n, bytes) = (kept.len() as i64, sizes.iter().map(|(_, n)| *n as i64).sum::<i64>());

    let skills_root = root(&st.data_dir);
    std::fs::create_dir_all(&skills_root).map_err(|e| format!("無法建立技能資料夾：{e}"))?;
    let tmp = skills_root.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    if let Err(e) = write_tree(&tmp, &kept) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("寫入技能檔案失敗：{e}"));
    }
    let existed = bs::skill_by_name(&st.pools.intake, &name).await.map_err(|e| e.to_string())?.is_some();
    let skill = match bs::upsert_skill(&st.pools.intake, &name, description.trim(), files_n, bytes, by).await {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e.to_string());
        }
    };
    let dest = skill_dir(&st.data_dir, &skill.id);
    let _ = std::fs::remove_dir_all(&dest);
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_dir_all(&tmp);
        if !existed {
            let _ = bs::delete_skill(&st.pools.intake, &skill.id).await;
        }
        return Err(format!("無法放進技能庫：{e}"));
    }
    Ok(skill)
}

/// 刪除技能（資料表一筆＋檔案）。不存在回 false。
pub async fn remove(st: &AppState, id: &str) -> Result<bool, String> {
    if !valid_id(id) {
        return Ok(false);
    }
    let gone = bs::delete_skill(&st.pools.intake, id).await.map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(skill_dir(&st.data_dir, id));
    Ok(gone)
}

/// 啟動時清掉上傳到一半留下的暫存資料夾與上次沒清掉的執行副本。
pub fn cleanup_stale(data_dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(root(data_dir)) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(".tmp-") {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let _ = std::fs::remove_dir_all(data_dir.join("run"));
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type()?.is_dir() {
            copy_tree(&src, &dst)?;
        } else if e.file_type()?.is_file() {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// 每張卡都有的「中途問人」說明。
const ASK_NOTE: &str = "\n\n遇到需要人決定的事（規格不清楚、有多種做法要選）時，可以用 ask_human 工具問人；它會等到有人回答才回來。能自己判斷的事不要問，一次只問一個問題。";

/// 一張卡這次執行用的技能：副本資料夾（`--add-dir`）與提示詞附註。被丟掉時副本一併刪除。
#[derive(Debug, Default)]
pub struct SkillCtx {
    dirs: Vec<String>,
    note: String,
    run_root: Option<PathBuf>,
    /// 資料庫與系統連線技能的 MCP 註冊：被丟掉時 token 作廢、瀏覽器收掉。
    db: Option<crate::dbmcp::McpGuard>,
    has_db: bool,
    has_sys: bool,
    /// 這張卡能不能用 `ask_human`（驗收卡不能：驗收是自動閘門，不該停下來等人）。
    has_ask: bool,
}

impl SkillCtx {
    /// 沒裝技能：什麼都不加。
    pub fn none() -> Self {
        Self::default()
    }

    pub fn dirs(&self) -> &[String] {
        &self.dirs
    }

    /// 給 `exec_claude_attached` 的附掛：技能資料夾，加上資料庫技能的 MCP 設定、允許的工具與逾時環境變數。
    pub fn attach(&self) -> crate::runner::Attach {
        let mut a = crate::runner::Attach::dirs(&self.dirs);
        if let Some(g) = &self.db {
            a.mcp_configs.push(g.config_path.to_string_lossy().into_owned());
            if self.has_ask {
                a.allow.push(format!("mcp__{}", crate::dbmcp::ASK_SERVER_NAME));
            }
            if self.has_db {
                a.allow.push(format!("mcp__{}", crate::dbmcp::SERVER_NAME));
            }
            if self.has_sys {
                a.allow.push(format!("mcp__{}", crate::dbmcp::SYSTEMS_SERVER_NAME));
            }
            a.env.push((crate::dbmcp::TOOL_TIMEOUT_ENV.0.to_string(), crate::dbmcp::TOOL_TIMEOUT_ENV.1.to_string()));
        }
        a
    }

    /// 要接在提示詞後面的說明；沒有技能就是空字串。
    pub fn note(&self) -> &str {
        &self.note
    }

    /// 依 Agent 的設定準備副本。技能庫裡讀不到的技能不會讓卡失敗，只在附註裡明講讀不到。
    pub async fn prepare(st: &Arc<AppState>, agent: &BoardAgent, card_id: &str) -> Self {
        Self::prepare_with(st, agent, card_id, true).await
    }

    /// 同 `prepare`；`ask` 為假時不開放（也不提）`ask_human`。
    pub async fn prepare_with(st: &Arc<AppState>, agent: &BoardAgent, card_id: &str, ask: bool) -> Self {
        let mut ctx = Self::prepare_custom(st, agent, card_id).await;
        // 資料庫與系統連線技能：發一個只屬於這張卡的 token，工具由 server 內建的 MCP 端點提供。
        let skill = aiterm_db::skill::parse(agent.db_skill.as_deref());
        let systems: Vec<aiterm_intake_core::board_store::BoardSystem> = bs::agent_systems(&st.pools.intake, agent).await.unwrap_or_default().into_iter().filter(|s| s.enabled).collect();
        {
            let run = st.data_dir.join("run").join(card_id);
            let mctx = crate::dbmcp::McpCtx { card_id: card_id.to_string(), agent_id: Some(agent.id.clone()), project_id: agent.project_id.clone(), skill: skill.clone(), systems: systems.iter().map(|s| s.id.clone()).collect() };
            if let Some(guard) = crate::dbmcp::register(st, mctx, &run) {
                if let Some(skill) = &skill {
                    ctx.note.push_str(&crate::dbmcp::prompt_note(st, skill).await);
                    ctx.has_db = true;
                }
                if !systems.is_empty() {
                    ctx.note.push_str(&systems_note(&systems));
                    ctx.has_sys = true;
                }
                if ask {
                    ctx.note.push_str(ASK_NOTE);
                    ctx.has_ask = true;
                }
                ctx.db = Some(guard);
                // 就算沒有自訂技能，副本資料夾（`run/<卡>`）也要在結束時清掉。
                ctx.run_root.get_or_insert_with(|| run.join("skills"));
            }
        }
        ctx
    }

    async fn prepare_custom(st: &AppState, agent: &BoardAgent, card_id: &str) -> Self {
        let skills = match bs::agent_skills(&st.pools.intake, agent).await {
            Ok(s) if !s.is_empty() => s,
            _ => return Self::none(),
        };
        let run_root = st.data_dir.join("run").join(card_id).join("skills");
        let data_dir = st.data_dir.clone();
        let root2 = run_root.clone();
        let copied = tokio::task::spawn_blocking(move || {
            let mut out: Vec<(BoardSkill, Result<PathBuf, String>)> = Vec::new();
            for s in skills {
                let dst = root2.join(rules::safe_dir_name(&s.name));
                let r = copy_tree(&skill_dir(&data_dir, &s.id), &dst).map(|_| dst).map_err(|e| e.to_string());
                out.push((s, r));
            }
            out
        })
        .await
        .unwrap_or_default();
        let mut ctx = Self { dirs: Vec::new(), note: String::new(), run_root: Some(run_root), db: None, has_db: false, has_sys: false, has_ask: false };
        ctx.note.push_str("\n\n你有這些技能可以用。需要時先用 Read 讀那個 SKILL.md 的全文並照做（不需要就不用讀）：\n");
        for (s, r) in &copied {
            match r {
                Ok(dir) => {
                    ctx.dirs.push(dir.to_string_lossy().into_owned());
                    let file = dir.join(rules::SKILL_FILE);
                    if s.description.is_empty() {
                        ctx.note.push_str(&format!("- {}：{}\n", s.name, file.display()));
                    } else {
                        ctx.note.push_str(&format!("- {}：{}（{}）\n", s.name, s.description, file.display()));
                    }
                }
                Err(_) => ctx.note.push_str(&format!("- {}：（這個技能目前讀不到，請略過）\n", s.name)),
            }
        }
        ctx.note.push_str("技能資料夾只能讀，不要修改裡面的檔案。");
        ctx
    }
}

/// 提示詞附註：有哪些系統可以操作、要遵守的規矩。
fn systems_note(systems: &[aiterm_intake_core::board_store::BoardSystem]) -> String {
    let list: Vec<String> = systems
        .iter()
        .map(|s| {
            let mode = match s.mode.as_str() {
                "approve_each" => "寫入要管理員核准",
                "autonomous" => "可寫入",
                _ => "只讀",
            };
            format!("{}（id：{}，{}）", s.name, s.id, mode)
        })
        .collect();
    format!(
        "\n\n系統連線：你可以用 sys_list 與其他 sys_* 工具操作這些網頁系統：{}。每個 sys_* 工具都要帶 system（系統 id）與 intent（一句話說明要做什麼，是讀還是寫）。\
         只讀的系統不能送出任何修改；需要核准的寫入要等管理員決定，被拒絕就不要硬做。登入是管理員事先分享好的：如果頁面變成登入頁，請停下來回報「需要重新分享 session」，不要自己填帳號密碼。",
        list.join("、")
    )
}

impl Drop for SkillCtx {
    fn drop(&mut self) {
        if let Some(root) = &self.run_root {
            // 刪到 `run/<card id>`：副本放在它底下的 skills/。
            let _ = std::fs::remove_dir_all(root.parent().unwrap_or(root));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::secrets::FileSecrets;
    use std::sync::Arc;

    async fn state() -> (Arc<AppState>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pools = db::open(dir.path()).await.unwrap();
        let secrets = Arc::new(FileSecrets::open(dir.path()).unwrap());
        (AppState::new(pools, secrets, "fp".into(), dir.path().to_path_buf()), dir)
    }

    fn files(skill_md: &str) -> Vec<(String, Vec<u8>)> {
        vec![("SKILL.md".into(), skill_md.as_bytes().to_vec()), ("scripts/run.sh".into(), b"echo hi".to_vec()), (".git/config".into(), b"x".to_vec())]
    }

    fn agent_with(skills: Option<&str>) -> BoardAgent {
        serde_json::from_value(serde_json::json!({
            "id": "a", "project_id": "P", "kind": "coder", "name": "甲", "role": "", "prompt_prefix": "", "canvas_x": 0.0, "canvas_y": 0.0,
            "produces_spec": false, "model": null, "cli": "claude", "paused_card_id": null, "custom_skills": skills, "created_by": "", "created_at": 0, "updated_at": 0, "version": 1
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn an_upload_is_stored_with_its_tree_and_noise_is_dropped() {
        let (st, dir) = state().await;
        let s = store_upload(&st, "資料夾", files("---\nname: 寫測試\ndescription: 先寫紅燈\n---\n內文"), "甲").await.unwrap();
        assert_eq!((s.name.as_str(), s.description.as_str(), s.files, s.uploaded_by.as_str()), ("寫測試", "先寫紅燈", 2, "甲"), ".git 不算");
        let d = skill_dir(dir.path(), &s.id);
        assert!(d.join("SKILL.md").is_file() && d.join("scripts/run.sh").is_file());
        assert!(!d.join(".git").exists());
        assert!(std::fs::read_dir(dir.path().join("skills")).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".tmp-")), "暫存資料夾要清掉");
    }

    #[tokio::test]
    async fn re_uploading_the_same_name_replaces_the_files_and_keeps_the_id() {
        let (st, dir) = state().await;
        let a = store_upload(&st, "x", files("---\nname: 技能\n---\n舊"), "甲").await.unwrap();
        let b = store_upload(&st, "x", vec![("SKILL.md".into(), b"---\nname: \xe6\x8a\x80\xe8\x83\xbd\n---\n\xe6\x96\xb0".to_vec())], "乙").await.unwrap();
        assert_eq!(a.id, b.id);
        let d = skill_dir(dir.path(), &b.id);
        assert!(std::fs::read_to_string(d.join("SKILL.md")).unwrap().contains("新"));
        assert!(!d.join("scripts").exists(), "取代是整份換掉，舊檔不留");
    }

    #[tokio::test]
    async fn bad_uploads_are_refused_and_leave_nothing_behind() {
        let (st, dir) = state().await;
        for (why, fs) in [
            ("路徑穿越", vec![("SKILL.md".to_string(), b"x".to_vec()), ("../evil".to_string(), b"x".to_vec())]),
            ("沒有 SKILL.md", vec![("a.md".to_string(), b"x".to_vec())]),
            ("單檔太大", vec![("SKILL.md".to_string(), vec![0u8; rules::MAX_FILE_BYTES + 1])]),
        ] {
            assert!(store_upload(&st, "x", fs, "甲").await.is_err(), "{why}");
        }
        assert!(bs::list_skills(&st.pools.intake).await.unwrap().is_empty());
        assert!(!dir.path().join("evil").exists() && !dir.path().join("skills/../evil").exists());
    }

    #[tokio::test]
    async fn a_run_copy_is_made_noted_and_removed_when_dropped() {
        let (st, dir) = state().await;
        let s = store_upload(&st, "x", files("---\nname: 寫測試\ndescription: 先寫紅燈\n---\n內文"), "甲").await.unwrap();
        let agent = agent_with(Some(&serde_json::to_string(&[&s.id]).unwrap()));
        let run = dir.path().join("run").join("card1");
        {
            let ctx = SkillCtx::prepare(&st, &agent, "card1").await;
            assert_eq!(ctx.dirs().len(), 1);
            let copy = Path::new(&ctx.dirs()[0]);
            assert!(copy.starts_with(&run) && copy.join("SKILL.md").is_file() && copy.join("scripts/run.sh").is_file());
            assert!(ctx.note().contains("寫測試") && ctx.note().contains("先寫紅燈") && ctx.note().contains("SKILL.md"));
            // 改副本不影響技能庫。
            std::fs::write(copy.join("SKILL.md"), "被改了").unwrap();
            assert!(std::fs::read_to_string(skill_dir(dir.path(), &s.id).join("SKILL.md")).unwrap().contains("內文"));
        }
        assert!(!run.exists(), "卡結束副本要刪掉");
    }

    #[tokio::test]
    async fn no_skills_means_no_dirs_and_no_note_and_a_missing_one_is_called_out() {
        let (st, dir) = state().await;
        let none = SkillCtx::prepare(&st, &agent_with(None), "c").await;
        assert!(none.dirs().is_empty() && none.note().is_empty());
        let s = store_upload(&st, "x", files("---\nname: 甲\n---\n"), "x").await.unwrap();
        std::fs::remove_dir_all(skill_dir(dir.path(), &s.id)).unwrap();
        let ctx = SkillCtx::prepare(&st, &agent_with(Some(&serde_json::to_string(&[&s.id]).unwrap())), "c2").await;
        assert!(ctx.dirs().is_empty());
        assert!(ctx.note().contains("讀不到"), "{}", ctx.note());
    }

    #[tokio::test]
    async fn removing_a_skill_deletes_row_and_files_and_startup_cleans_leftovers() {
        let (st, dir) = state().await;
        let s = store_upload(&st, "x", files("---\nname: 甲\n---\n"), "x").await.unwrap();
        assert!(remove(&st, &s.id).await.unwrap());
        assert!(!skill_dir(dir.path(), &s.id).exists());
        assert!(!remove(&st, &s.id).await.unwrap() && !remove(&st, "../etc").await.unwrap());
        std::fs::create_dir_all(dir.path().join("skills/.tmp-abc")).unwrap();
        std::fs::create_dir_all(dir.path().join("run/old")).unwrap();
        cleanup_stale(dir.path());
        assert!(!dir.path().join("skills/.tmp-abc").exists() && !dir.path().join("run").exists());
    }
}

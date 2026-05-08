//! Skill Sync: installs and removes skills pushed from Management Server.
//! Company skills take priority over local skills with the same skill_id.

use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

use crate::enterprise::types::{SkillUpdate, SkillUpdateAction};

/// Directory where company-pushed skills are stored.
/// Claude Code reads skills from ~/.claude/skills/ (or platform equivalent).
fn skills_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("skills").join("company"))
}

pub struct SkillSyncer;

impl SkillSyncer {
    pub async fn apply_updates(app: &AppHandle, updates: Vec<SkillUpdate>) {
        let Some(dir) = skills_dir() else {
            log::warn!("skill_sync: cannot determine home directory");
            return;
        };

        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::error!("skill_sync: failed to create skills dir: {}", e);
            return;
        }

        for update in updates {
            match update.action {
                SkillUpdateAction::Install => {
                    if let Some(content) = &update.content {
                        let path = dir.join(format!("{}.md", update.skill_id));
                        match std::fs::write(&path, content) {
                            Ok(_) => {
                                log::info!("skill_sync: installed {} v{}", update.skill_id, update.version);
                                app.emit("enterprise:skill-installed", serde_json::json!({
                                    "skill_id": update.skill_id,
                                    "version": update.version,
                                    "content": content,
                                })).ok();
                            }
                            Err(e) => log::error!("skill_sync: write failed: {}", e),
                        }
                    }
                }
                SkillUpdateAction::Remove => {
                    let path = dir.join(format!("{}.md", update.skill_id));
                    if path.exists() {
                        let _ = std::fs::remove_file(&path);
                        log::info!("skill_sync: removed {}", update.skill_id);
                        app.emit("enterprise:skill-removed", serde_json::json!({
                            "skill_id": update.skill_id,
                        })).ok();
                    }
                }
            }
        }
    }
}

//! 驗收者的禁止清單：全域預設（server 一份，管理員編輯）＋專案覆寫（移除某條預設／新增專屬規則）。
//! 不是沙箱：只擋最明顯的誤操作（見 spec §6.3）；Claude 若把命令寫進腳本再執行，字串比對攔不住。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub rule: String,
    pub note: String,
    pub enabled: bool,
}

/// 專案覆寫（對應 `board_store::DenyOverride`；放在這裡讓純函式不依賴資料庫型別）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyOverrideRow {
    pub rule: String,
    pub kind: String,
    pub enabled: bool,
}

pub const MAX_RULE_LEN: usize = 200;

/// 規則必須是單一的 `Bash(...)` 比對式（會原樣放進 `--disallowedTools`，以逗號分隔，所以不能含逗號與換行）。
pub fn validate_rule(r: &str) -> Result<(), String> {
    let t = r.trim();
    if !(t.starts_with("Bash(") && t.ends_with(')')) {
        return Err("規則要寫成 Bash(命令:*) 的形式".into());
    }
    if t.contains('\n') || t.contains('\r') {
        return Err("規則不能換行".into());
    }
    if t.contains(',') {
        return Err("規則不能含逗號（逗號是規則之間的分隔）".into());
    }
    if t.len() > MAX_RULE_LEN {
        return Err(format!("規則最長 {MAX_RULE_LEN} 字元"));
    }
    let inner = &t["Bash(".len()..t.len() - 1];
    if inner.trim().is_empty() {
        return Err("括號裡不能是空的".into());
    }
    if inner.contains('(') || inner.contains(')') {
        return Err("括號裡不能再有括號".into());
    }
    Ok(())
}

pub fn default_rules() -> Vec<Rule> {
    let r = |rule: &str, note: &str| Rule { rule: rule.into(), note: note.into(), enabled: true };
    vec![
        r("Bash(sudo:*)", "提權"),
        r("Bash(su:*)", "提權"),
        r("Bash(git push:*)", "對外發佈：推到遠端"),
        r("Bash(npm publish:*)", "對外發佈：發佈套件"),
        r("Bash(cargo publish:*)", "對外發佈：發佈套件"),
        r("Bash(gh pr create:*)", "對外發佈：建立 PR"),
        r("Bash(gh release create:*)", "對外發佈：建立 release"),
        r("Bash(docker push:*)", "對外發佈：推映像檔"),
        r("Bash(ssh:*)", "遠端連線"),
        r("Bash(scp:*)", "遠端連線"),
        r("Bash(sftp:*)", "遠端連線"),
        r("Bash(curl:* | sh)", "下載後直接執行"),
        r("Bash(curl:* | bash)", "下載後直接執行"),
        r("Bash(wget:* | sh)", "下載後直接執行"),
        r("Bash(wget:* | bash)", "下載後直接執行"),
        r("Bash(rm -rf /:*)", "破壞性刪除：根目錄"),
        r("Bash(rm -rf ~:*)", "破壞性刪除：家目錄"),
        r("Bash(rm -rf $HOME:*)", "破壞性刪除：家目錄"),
        r("Bash(rm -rf ..:*)", "破壞性刪除：上層目錄"),
        r("Bash(launchctl:*)", "改系統設定"),
        r("Bash(systemctl:*)", "改系統設定"),
        r("Bash(defaults write:*)", "改系統設定"),
        r("Bash(npm install -g:*)", "改全域環境"),
        r("Bash(npm i -g:*)", "改全域環境"),
        r("Bash(pip install --user:*)", "改全域環境"),
    ]
}

/// 有效的禁止規則：全域啟用的，扣掉專案移除的，加上專案啟用的新增。
pub fn effective(global: &[Rule], overrides: &[DenyOverrideRow]) -> Vec<String> {
    let removed: Vec<&str> = overrides.iter().filter(|o| o.kind == "remove" && o.enabled).map(|o| o.rule.as_str()).collect();
    let mut out: Vec<String> = global.iter().filter(|g| g.enabled && !removed.contains(&g.rule.as_str())).map(|g| g.rule.clone()).collect();
    for o in overrides.iter().filter(|o| o.kind == "add" && o.enabled) {
        if !out.contains(&o.rule) {
            out.push(o.rule.clone());
        }
    }
    out
}

/// 全域清單存在 intake 資料庫的設定表（與 `orchestrate:<pid>` 同處）。
pub const GLOBAL_KEY: &str = "verify_deny_global";

pub async fn load_global(pool: &sqlx::SqlitePool) -> Vec<Rule> {
    aiterm_intake_core::store::get_setting(pool, GLOBAL_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(default_rules)
}

pub async fn save_global(pool: &sqlx::SqlitePool, rules: &[Rule]) -> Result<(), String> {
    for r in rules {
        validate_rule(&r.rule).map_err(|e| format!("{}：{e}", r.rule))?;
    }
    aiterm_intake_core::store::set_setting(pool, GLOBAL_KEY, &serde_json::to_string(rules).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())
}

/// 這個專案這次驗收實際生效的規則（全域＋專案覆寫）。
pub async fn effective_for(pool: &sqlx::SqlitePool, project_id: &str) -> Vec<String> {
    let global = load_global(pool).await;
    let overrides: Vec<DenyOverrideRow> = aiterm_intake_core::board_store::list_deny_overrides(pool, project_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|o| DenyOverrideRow { rule: o.rule, kind: o.kind, enabled: o.enabled })
        .collect();
    effective(&global, &overrides)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_the_categories_and_every_rule_is_well_formed() {
        let d = default_rules();
        for needle in ["Bash(sudo:*)", "Bash(git push:*)", "Bash(npm publish:*)", "Bash(ssh:*)", "Bash(rm -rf /:*)", "Bash(rm -rf ~:*)", "Bash(npm install -g:*)"] {
            assert!(d.iter().any(|r| r.rule == needle), "預設缺：{needle}");
        }
        assert!(d.iter().all(|r| validate_rule(&r.rule).is_ok() && !r.note.is_empty()), "預設每一條都要合法並有說明");
    }

    #[test]
    fn rules_must_be_a_single_bash_pattern() {
        assert!(validate_rule("Bash(foo:*)").is_ok());
        let long = format!("Bash({}:*)", "x".repeat(300));
        for bad in ["", "foo", "Bash()", "Bash(a:*)\nBash(b:*)", "Read(x)", "Bash(a:*", "Bash(a,b:*)", "Bash(a(b):*)", long.as_str()] {
            assert!(validate_rule(bad).is_err(), "應該拒絕：{bad:?}");
        }
    }

    #[test]
    fn effective_rules_apply_global_then_project_overrides() {
        let global = vec![
            Rule { rule: "Bash(sudo:*)".into(), note: "x".into(), enabled: true },
            Rule { rule: "Bash(docker push:*)".into(), note: "x".into(), enabled: true },
            Rule { rule: "Bash(ssh:*)".into(), note: "x".into(), enabled: false },
        ];
        let overrides = vec![
            DenyOverrideRow { rule: "Bash(docker push:*)".into(), kind: "remove".into(), enabled: true },
            DenyOverrideRow { rule: "Bash(make deploy:*)".into(), kind: "add".into(), enabled: true },
            DenyOverrideRow { rule: "Bash(rm -rf build:*)".into(), kind: "add".into(), enabled: false },
        ];
        assert_eq!(effective(&global, &overrides), vec!["Bash(sudo:*)".to_string(), "Bash(make deploy:*)".to_string()], "停用的、被專案移除的、停用的新增都不算");
    }
}

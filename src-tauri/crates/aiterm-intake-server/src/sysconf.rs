//! server 上的系統連線設定與登入 session：輸入檢查、cookie 篩選（只留白名單網域的）、加密保存。
//! session 是使用者在桌面版登入後匯出的 cookie；帳號密碼不經過 server。

use aiterm_intake_core::board_store::{self as bs, BoardSystem, NewSystem};
use aiterm_intake_core::host::Secrets;
use aiterm_systems::gate::host_of;
use serde_json::{json, Value};

use crate::state::AppState;

/// server 版支援的寫入權限模式（`rules` 模式先不做）。
pub const MODES: [&str; 3] = ["read_only", "approve_each", "autonomous"];
/// 一份 session 最多幾個 cookie、序列化後最大多少位元組。
pub const MAX_COOKIES: usize = 500;
pub const MAX_SESSION_BYTES: usize = 1024 * 1024;

pub fn session_key(id: &str) -> String {
    format!("syssession:{id}")
}

/// 設定輸入。
pub struct SystemInput<'a> {
    pub name: &'a str,
    pub start_url: &'a str,
    pub allowed_hosts: &'a [String],
    pub mode: &'a str,
    pub enabled: bool,
}

/// 檢查並整理輸入；回傳整理後的白名單（一定包含起始網址的網域，小寫、去重、去掉空白）。
pub fn validate(c: &SystemInput<'_>) -> Result<Vec<String>, String> {
    if c.name.trim().is_empty() || c.name.chars().count() > 80 || c.name.chars().any(|ch| ch.is_control()) {
        return Err("系統名稱不能是空的、超過 80 字或含控制字元".into());
    }
    if !c.start_url.starts_with("https://") && !c.start_url.starts_with("http://") {
        return Err("起始網址要以 http:// 或 https:// 開頭".into());
    }
    let Some(start_host) = host_of(c.start_url) else { return Err("起始網址不合法".into()) };
    if !MODES.contains(&c.mode) {
        return Err(format!("server 支援的權限模式：{}（規則模式只在桌面版）", MODES.join("、")));
    }
    let mut hosts: Vec<String> = vec![start_host];
    for h in c.allowed_hosts {
        let h = h.trim().trim_start_matches("*.").to_ascii_lowercase();
        if h.is_empty() || h.contains('/') || h.contains(' ') || h.chars().any(|ch| ch.is_control()) {
            return Err(format!("白名單網域不合法：{h:?}"));
        }
        if !hosts.contains(&h) {
            hosts.push(h);
        }
    }
    Ok(hosts)
}

pub async fn upsert(st: &AppState, c: &SystemInput<'_>, by: &str) -> Result<BoardSystem, String> {
    let hosts = validate(c)?;
    bs::upsert_system(&st.pools.intake, NewSystem { name: c.name.trim(), start_url: c.start_url, allowed_hosts: &hosts, mode: c.mode, enabled: c.enabled, created_by: by })
        .await
        .map_err(|e| e.to_string())
}

/// 刪除系統、它的 session 與（還在跑的）瀏覽器。
pub async fn remove(st: &std::sync::Arc<AppState>, id: &str) -> Result<bool, String> {
    let gone = bs::delete_system(&st.pools.intake, id).await.map_err(|e| e.to_string())?;
    let _ = st.secrets.delete(&session_key(id));
    crate::syscore::close_system(st, id).await;
    Ok(gone)
}

/// cookie 的網域跟允許的網域有沒有關係：瀏覽器會把這個 cookie 送給白名單裡的某個網域。
fn cookie_domain_allowed(domain: &str, hosts: &[String]) -> bool {
    let d = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    !d.is_empty() && hosts.iter().any(|h| *h == d || h.ends_with(&format!(".{d}")) || d.ends_with(&format!(".{h}")))
}

/// 把桌面版從 Chrome 取出的 cookie（CDP `Network.getAllCookies` 的格式）整理成可以載入的樣子：
/// 只留網域跟白名單有關的，只保留載入需要的欄位。
pub fn filter_cookies(cookies: &[Value], hosts: &[String]) -> Vec<Value> {
    cookies
        .iter()
        .filter(|c| c["name"].is_string() && c["value"].is_string() && c["domain"].as_str().is_some_and(|d| cookie_domain_allowed(d, hosts)))
        .map(|c| {
            let mut out = json!({
                "name": c["name"], "value": c["value"], "domain": c["domain"],
                "path": c["path"].as_str().unwrap_or("/"),
                "httpOnly": c["httpOnly"].as_bool().unwrap_or(false),
                "secure": c["secure"].as_bool().unwrap_or(false),
            });
            // 沒有到期時間（-1 或沒有）是 session cookie，不帶 expires。
            if let Some(e) = c["expires"].as_f64().filter(|e| *e > 0.0) {
                out["expires"] = json!(e);
            }
            if let Some(s) = c["sameSite"].as_str().filter(|s| matches!(*s, "Strict" | "Lax" | "None")) {
                out["sameSite"] = json!(s);
            }
            out
        })
        .collect()
}

/// 收下一份 session：篩選、檢查上限、加密保存。回傳實際保存的 cookie 數。
pub async fn store_session(st: &AppState, system: &BoardSystem, cookies: &[Value], by: &str) -> Result<usize, String> {
    let kept = filter_cookies(cookies, &system.hosts());
    if kept.is_empty() {
        return Err("沒有任何 cookie 屬於這個系統的白名單網域——請先在桌面版登入這個系統再分享".into());
    }
    if kept.len() > MAX_COOKIES {
        return Err(format!("cookie 太多：{} 個（上限 {MAX_COOKIES}）", kept.len()));
    }
    let raw = serde_json::to_string(&kept).map_err(|e| e.to_string())?;
    if raw.len() > MAX_SESSION_BYTES {
        return Err(format!("session 太大：{} KB（上限 {} KB）", raw.len() / 1024, MAX_SESSION_BYTES / 1024));
    }
    st.secrets.set(&session_key(&system.id), &raw)?;
    bs::put_system_session(&st.pools.intake, &system.id, by, kept.len() as i64).await.map_err(|e| e.to_string())?;
    Ok(kept.len())
}

/// 讀出保存的 session（沒有或壞掉回 `None`）。
pub fn load_session(st: &AppState, system_id: &str) -> Option<Vec<Value>> {
    let raw = st.secrets.get(&session_key(system_id))?;
    serde_json::from_str(&raw).ok()
}

pub async fn remove_session(st: &AppState, system_id: &str) -> Result<bool, String> {
    let _ = st.secrets.delete(&session_key(system_id));
    bs::delete_system_session(&st.pools.intake, system_id).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(name: &'a str, url: &'a str, hosts: &'a [String], mode: &'a str) -> SystemInput<'a> {
        SystemInput { name, start_url: url, allowed_hosts: hosts, mode, enabled: true }
    }

    #[test]
    fn inputs_are_validated_and_the_start_host_is_always_allowed() {
        let h = vec!["SSO.Example.com".to_string(), "*.cdn.example.com".to_string(), "sso.example.com".to_string()];
        let hosts = validate(&input("ERP", "https://erp.example.com/home", &h, "approve_each")).unwrap();
        assert_eq!(hosts, vec!["erp.example.com", "sso.example.com", "cdn.example.com"], "起始網域在最前、小寫、去重、去掉 *.");
        assert!(validate(&input("ERP", "ftp://x/", &[], "read_only")).unwrap_err().contains("http"));
        assert!(validate(&input("ERP", "https://x/", &[], "rules")).unwrap_err().contains("規則模式"));
        assert!(validate(&input("  ", "https://x/", &[], "read_only")).is_err());
        assert!(validate(&input("ERP", "https://x/", &["a b".to_string()], "read_only")).unwrap_err().contains("白名單"));
        assert!(validate(&input("ERP", "https://x/", &["a/b".to_string()], "read_only")).is_err());
    }

    #[test]
    fn only_cookies_that_would_be_sent_to_the_allowed_hosts_are_kept_and_trimmed() {
        let hosts = vec!["erp.example.com".to_string(), "sso.example.com".to_string()];
        let c = |name: &str, domain: &str| json!({"name": name, "value": "v", "domain": domain, "path": "/", "httpOnly": true, "secure": true, "expires": 1900000000.5, "sameSite": "Lax", "size": 9, "session": false, "priority": "Medium"});
        let all = vec![c("a", "erp.example.com"), c("b", ".example.com"), c("c", ".sso.example.com"), c("d", "evil.com"), c("e", ".other.com"), c("f", "notexample.com")];
        let kept = filter_cookies(&all, &hosts);
        assert_eq!(kept.iter().map(|k| k["name"].as_str().unwrap()).collect::<Vec<_>>(), vec!["a", "b", "c"], "父網域與子網域都算，不相干的（含只是結尾字串相同的）都不留");
        assert!(kept[0].get("size").is_none() && kept[0].get("priority").is_none(), "只保留載入需要的欄位");
        assert_eq!((kept[0]["expires"].clone(), kept[0]["sameSite"].clone()), (json!(1900000000.5), json!("Lax")));
        let session = json!({"name": "s", "value": "v", "domain": "erp.example.com", "expires": -1, "sameSite": "Unspecified"});
        let k = &filter_cookies(&[session], &hosts)[0];
        assert!(k.get("expires").is_none() && k.get("sameSite").is_none(), "session cookie 不帶 expires，不認得的 sameSite 不帶");
        assert!(filter_cookies(&[json!({"name": 1, "value": "v", "domain": "erp.example.com"}), json!({"value": "v"})], &hosts).is_empty(), "格式不對的丟掉");
    }
}

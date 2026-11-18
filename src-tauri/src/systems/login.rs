//! 自動登入：帳號密碼存在作業系統鑰匙圈，由 AITerm 自己用 CDP 填進登入頁並送出。
//! 密碼只存在 Rust 記憶體與目標網頁之間，不會進 AI 的提示、工具回傳或日誌。

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::cdp::Cdp;
use super::state::looks_logged_out;
use super::types::SystemConnection;
use crate::secret::SecretStore;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Creds {
    pub username: String,
    pub password: String,
}

// 不實作 Debug，免得不小心被 `{:?}` 印進日誌。

fn key(system_id: &str) -> String {
    format!("system-login:{system_id}")
}

pub fn load(secrets: &SecretStore, system_id: &str) -> Option<Creds> {
    let raw = secrets.get(&key(system_id)).ok().flatten()?;
    let c: Creds = serde_json::from_str(&raw).ok()?;
    (!c.username.is_empty() && !c.password.is_empty()).then_some(c)
}

/// 存帳號；`password` 為 `None` 代表密碼不變（沒存過就只存帳號，登入時會缺密碼而略過）。
/// 帳號為空＝清除。
pub fn save(secrets: &SecretStore, system_id: &str, username: &str, password: Option<&str>) -> Result<(), String> {
    let username = username.trim();
    if username.is_empty() {
        return clear(secrets, system_id);
    }
    let old = secrets.get(&key(system_id)).ok().flatten().and_then(|r| serde_json::from_str::<Creds>(&r).ok());
    let password = match password {
        Some(p) => p.to_string(),
        None => old.map(|c| c.password).unwrap_or_default(),
    };
    let raw = serde_json::to_string(&Creds { username: username.into(), password }).map_err(|e| e.to_string())?;
    secrets.set(&key(system_id), &raw).map_err(|e| e.to_string())
}

pub fn clear(secrets: &SecretStore, system_id: &str) -> Result<(), String> {
    secrets.delete(&key(system_id)).map_err(|e| e.to_string())
}

/// 給畫面：帳號與「有沒有存密碼」，密碼本身不回傳。
pub fn describe(secrets: &SecretStore, system_id: &str) -> (String, bool) {
    match secrets.get(&key(system_id)).ok().flatten().and_then(|r| serde_json::from_str::<Creds>(&r).ok()) {
        Some(c) => (c.username, !c.password.is_empty()),
        None => (String::new(), false),
    }
}

/// 在頁面內執行：找可見的密碼欄位與它前面最近的文字欄位，填值（走原生 setter 並送事件，
/// 讓 Angular／React 的表單狀態同步），再按送出。回傳狀態字串，不含任何輸入內容。
const FILL_JS: &str = r#"(async (u, p) => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const visible = (e) => !!(e.offsetWidth || e.offsetHeight);
  let pw = null;
  for (let i = 0; i < 40; i++) {
    pw = [...document.querySelectorAll('input[type=password]')].find(visible);
    if (pw) break;
    await sleep(200);
  }
  if (!pw) return 'no-password-field';
  const inputs = [...document.querySelectorAll('input')].filter((e) => visible(e) && !['hidden', 'checkbox', 'radio', 'submit', 'button'].includes(e.type));
  const user = inputs.slice(0, inputs.indexOf(pw)).reverse().find((e) => ['text', 'email', 'tel', ''].includes(e.type));
  if (!user) return 'no-username-field';
  const set = (el, v) => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, v);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    el.dispatchEvent(new Event('blur', { bubbles: true }));
  };
  user.focus(); set(user, u);
  pw.focus(); set(pw, p);
  await sleep(250);
  const scope = pw.closest('form') || document;
  const btn = scope.querySelector('button[type=submit],input[type=submit]') || [...scope.querySelectorAll('button')].find((b) => /登入|login|sign ?in/i.test(b.innerText));
  if (btn) btn.click();
  else if (pw.form) pw.form.requestSubmit();
  else return 'no-submit';
  return 'submitted';
})"#;

async fn pages(cdp: &Cdp) -> Vec<(String, String)> {
    let Ok(res) = cdp.call("Target.getTargets", json!({}), None).await else { return Vec::new() };
    res["targetInfos"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|t| t["type"] == "page")
                .filter_map(|t| Some((t["targetId"].as_str()?.to_string(), t["url"].as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn is_blank(url: &str) -> bool {
    url.is_empty() || url.starts_with("about:") || url.starts_with("chrome://")
}

/// 瀏覽器開在登入頁就自動登入。回傳：`Ok(true)` 剛登入成功、`Ok(false)` 不需要登入、
/// `Err` 登入沒成功（帳密錯、頁面結構不認得、逾時）。最多只試一次，免得帳號被鎖。
pub async fn auto_login(browser_ws: &str, conn: &SystemConnection, creds: &Creds) -> Result<bool, String> {
    let (cdp, _ev) = Cdp::connect(browser_ws).await?;
    let as_text = |url: &str| format!("Page URL: {url}");

    // 剛啟動時頁面還在載入：等到出現非空白頁（最多 10 秒）。
    let deadline = Instant::now() + Duration::from_secs(10);
    let target = loop {
        let list = pages(&cdp).await;
        if let Some((id, url)) = list.iter().find(|(_, u)| !is_blank(u)) {
            break (id.clone(), url.clone());
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    if !looks_logged_out(conn, &as_text(&target.1)) {
        return Ok(false);
    }

    let session = cdp.call("Target.attachToTarget", json!({ "targetId": target.0, "flatten": true }), None).await?["sessionId"]
        .as_str()
        .ok_or("無法連到登入頁")?
        .to_string();
    let expr = format!("({FILL_JS})({}, {})", Value::String(creds.username.clone()), Value::String(creds.password.clone()));
    let res = cdp
        .call("Runtime.evaluate", json!({ "expression": expr, "awaitPromise": true, "returnByValue": true }), Some(&session))
        .await?;
    let status = res["result"]["value"].as_str().unwrap_or("").to_string();
    let why = match status.as_str() {
        "submitted" => None,
        "no-password-field" => Some("頁面上找不到密碼欄位"),
        "no-username-field" => Some("頁面上找不到帳號欄位"),
        _ => Some("找不到登入按鈕"),
    };
    if let Some(w) = why {
        return Err(format!("自動登入失敗：{w}"));
    }

    // 送出後等離開登入頁（最多 20 秒）。
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let list = pages(&cdp).await;
        if let Some((_, url)) = list.iter().find(|(id, _)| *id == target.0).or_else(|| list.iter().find(|(_, u)| !is_blank(u))) {
            if !looks_logged_out(conn, &as_text(url)) {
                return Ok(true);
            }
        }
    }
    Err("自動登入失敗：送出後仍停在登入頁，帳號或密碼可能不正確".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_pages_are_not_pages_to_judge() {
        assert!(is_blank("about:blank") && is_blank("") && is_blank("chrome://newtab/"));
        assert!(!is_blank("https://x/login"));
    }

    #[test]
    fn the_fill_script_never_returns_what_was_typed() {
        // 回傳值只有固定的狀態字串；密碼不會經由 Runtime.evaluate 的結果流回 Rust 日誌。
        assert!(FILL_JS.contains("return 'submitted'") && !FILL_JS.contains("return p") && !FILL_JS.contains("return u"));
    }

    #[test]
    fn creds_round_trip_through_json() {
        let c = Creds { username: "a".into(), password: "b".into() };
        let back: Creds = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert!(back == c);
    }
}

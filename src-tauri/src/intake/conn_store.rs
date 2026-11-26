//! 已記錄的 server 連線清單（多組）。只管「記錄」：清單與哪一組是目前連線；token 在系統鑰匙圈，
//! 每組各一個項目（`token_secret_id`）。連上／切換／忘記的流程在 `remote.rs`。
//!
//! 舊版只記一組：`remote_connection`（連線資訊）＋ `intake_remote_token`（token）。
//! 第一次讀取時搬成清單的第一筆並設為目前連線（`migrate_legacy`）。
use super::remote::Saved;
use aiterm_intake_core::store;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// 舊版單一連線的設定鍵與 token 鑰匙圈項目。
pub const LEGACY_CONNECTION_KEY: &str = "remote_connection";
pub const LEGACY_TOKEN_ID: &str = "intake_remote_token";
const LIST_KEY: &str = "remote_connections";
const ACTIVE_KEY: &str = "remote_active";

/// 清單中的一筆：穩定的 id ＋ 連線資訊。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ConnEntry {
    pub id: String,
    #[serde(flatten)]
    pub saved: Saved,
}

/// 給畫面顯示的一筆（沒有指紋與 token）。
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ConnView {
    pub id: String,
    pub addr: String,
    pub name: String,
    pub role: aiterm_intake_core::api::Role,
    /// 現在連著的是不是這一筆。
    pub active: bool,
}

pub fn token_secret_id(id: &str) -> String {
    format!("{LEGACY_TOKEN_ID}:{id}")
}

pub async fn list(pool: &SqlitePool) -> Vec<ConnEntry> {
    store::get_setting(pool, LIST_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

async fn write_list(pool: &SqlitePool, list: &[ConnEntry]) -> Result<(), String> {
    store::set_setting(pool, LIST_KEY, &serde_json::to_string(list).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())
}

pub async fn get(pool: &SqlitePool, id: &str) -> Option<ConnEntry> {
    list(pool).await.into_iter().find(|e| e.id == id)
}

/// 目前連線的那一筆（沒有＝本機模式）。清單裡已經找不到的 id 視同沒有。
pub async fn active_id(pool: &SqlitePool) -> Option<String> {
    let id = store::get_setting(pool, ACTIVE_KEY).await.ok().flatten().filter(|s| !s.is_empty())?;
    get(pool, &id).await.map(|e| e.id)
}

pub async fn set_active(pool: &SqlitePool, id: Option<&str>) -> Result<(), String> {
    store::set_setting(pool, ACTIVE_KEY, id.unwrap_or("")).await.map_err(|e| e.to_string())
}

/// 新增一筆；同一個位址已經記錄過就更新那一筆（重新配對換了名稱、角色、指紋），回傳它的 id。
pub async fn upsert(pool: &SqlitePool, saved: &Saved) -> Result<String, String> {
    let mut all = list(pool).await;
    let id = match all.iter_mut().find(|e| e.saved.addr == saved.addr) {
        Some(e) => {
            e.saved = saved.clone();
            e.id.clone()
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            all.push(ConnEntry { id: id.clone(), saved: saved.clone() });
            id
        }
    };
    write_list(pool, &all).await?;
    Ok(id)
}

/// 從清單拿掉；目前連線的就一併清掉「目前連線」。
pub async fn remove(pool: &SqlitePool, id: &str) -> Result<bool, String> {
    let mut all = list(pool).await;
    let before = all.len();
    all.retain(|e| e.id != id);
    if all.len() == before {
        return Ok(false);
    }
    write_list(pool, &all).await?;
    if active_id(pool).await.is_none() {
        set_active(pool, None).await?;
    }
    Ok(true)
}

pub async fn views(pool: &SqlitePool) -> Vec<ConnView> {
    let active = active_id(pool).await;
    list(pool).await.into_iter().map(|e| ConnView { active: active.as_deref() == Some(e.id.as_str()), id: e.id, addr: e.saved.addr, name: e.saved.name, role: e.saved.role }).collect()
}

/// 舊版單一連線搬進清單。已經有清單就不動；回傳搬進來的那一筆（呼叫端要把舊 token 複製到新項目）。
pub async fn migrate_legacy(pool: &SqlitePool) -> Option<ConnEntry> {
    if store::get_setting(pool, LIST_KEY).await.ok().flatten().is_some() {
        return None;
    }
    let saved: Saved = store::get_setting(pool, LEGACY_CONNECTION_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok())?;
    let id = upsert(pool, &saved).await.ok()?;
    set_active(pool, Some(&id)).await.ok()?;
    sqlx::query("DELETE FROM intake_settings WHERE key = ?").bind(LEGACY_CONNECTION_KEY).execute(pool).await.ok();
    get(pool, &id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiterm_intake_core::api::Role;

    async fn pool() -> SqlitePool {
        let p = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&p).await.unwrap();
        p
    }
    fn saved(addr: &str, name: &str) -> Saved {
        Saved { addr: addr.into(), fingerprint: Some(format!("fp-{addr}")), name: name.into(), role: Role::Member }
    }

    #[tokio::test]
    async fn several_servers_are_remembered_and_only_one_is_active() {
        let p = pool().await;
        assert!(list(&p).await.is_empty() && active_id(&p).await.is_none());
        let a = upsert(&p, &saved("10.0.0.1:8443", "小明")).await.unwrap();
        let b = upsert(&p, &saved("10.0.0.2:8443", "小華")).await.unwrap();
        assert_ne!(a, b);
        set_active(&p, Some(&b)).await.unwrap();
        let v = views(&p).await;
        assert_eq!(v.iter().map(|x| (x.addr.as_str(), x.active)).collect::<Vec<_>>(), vec![("10.0.0.1:8443", false), ("10.0.0.2:8443", true)]);
        assert_eq!(get(&p, &a).await.unwrap().saved.fingerprint.as_deref(), Some("fp-10.0.0.1:8443"), "指紋跟著各自那一筆");
    }

    #[tokio::test]
    async fn pairing_the_same_address_again_updates_that_entry_instead_of_adding_one() {
        let p = pool().await;
        let a = upsert(&p, &saved("10.0.0.1:8443", "舊名")).await.unwrap();
        let again = upsert(&p, &Saved { role: Role::Admin, ..saved("10.0.0.1:8443", "新名") }).await.unwrap();
        assert_eq!(a, again);
        let all = list(&p).await;
        assert_eq!((all.len(), all[0].saved.name.as_str(), all[0].saved.role), (1, "新名", Role::Admin));
    }

    #[tokio::test]
    async fn forgetting_the_active_one_leaves_local_mode_and_keeps_the_others() {
        let p = pool().await;
        let a = upsert(&p, &saved("a:1", "甲")).await.unwrap();
        let b = upsert(&p, &saved("b:1", "乙")).await.unwrap();
        set_active(&p, Some(&a)).await.unwrap();
        assert!(remove(&p, &a).await.unwrap());
        assert!(active_id(&p).await.is_none(), "目前連線的被忘記後回到本機模式");
        assert_eq!(list(&p).await.iter().map(|e| e.id.clone()).collect::<Vec<_>>(), vec![b]);
        assert!(!remove(&p, "不存在").await.unwrap());
    }

    #[tokio::test]
    async fn an_active_id_that_is_no_longer_in_the_list_counts_as_none() {
        let p = pool().await;
        set_active(&p, Some("ghost")).await.unwrap();
        assert!(active_id(&p).await.is_none());
    }

    #[tokio::test]
    async fn the_old_single_connection_becomes_the_first_entry_and_stays_active_once() {
        let p = pool().await;
        let old = saved("192.168.1.20:8443", "舊的");
        store::set_setting(&p, LEGACY_CONNECTION_KEY, &serde_json::to_string(&old).unwrap()).await.unwrap();
        let e = migrate_legacy(&p).await.expect("搬進清單");
        assert_eq!(e.saved, old);
        assert_eq!(active_id(&p).await.as_deref(), Some(e.id.as_str()));
        assert!(store::get_setting(&p, LEGACY_CONNECTION_KEY).await.unwrap().is_none(), "舊的鍵拿掉，不會再搬第二次");
        assert!(migrate_legacy(&p).await.is_none());
        assert_eq!(list(&p).await.len(), 1);
    }

    #[tokio::test]
    async fn without_an_old_connection_nothing_is_migrated() {
        let p = pool().await;
        assert!(migrate_legacy(&p).await.is_none());
    }
}

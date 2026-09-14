//! 遠端終端機地址簿。
//!
//! 紀錄存設定檔、金鑰存 OS keychain（key 為 `remote:{id}`），形狀照
//! `commands/vcs.rs`。**金鑰不跨 IPC**：前端永遠拿不到已存的金鑰，
//! 連線時只送 `saved_host_id`，由 `share_viewer_connect` 自己去 keychain 取。

/// 前端用來辨識「這台的金鑰不在這台電腦上」的錯誤字串。
///
/// 走字串是因為整個 command 層都是 `Result<_, String>`。Task 5 之後前端會有
/// 一份同名的字串常數，**兩邊只能靠人工保持一致，沒有任何編譯期檢查**——
/// 這個 repo 既有的 `no_remote:`（commands/vcs.rs）與 AiError 的 kind 標籤
/// 也是同樣的手抄關係。
pub const ERR_SAVED_KEY_MISSING: &str = "remote_host_key_missing";

/// keychain 本身讀不到（鎖住、權限被拒、資料損毀），跟「這台電腦沒有這把金鑰」
/// 是不同的問題——後者要使用者重貼金鑰，前者重貼幾次都沒用。合併成同一個錯誤
/// 就會叫使用者去修一個沒有壞的東西。
pub const ERR_KEYCHAIN_UNAVAILABLE: &str = "remote_host_keychain_unavailable";

/// 這一筆地址簿條目的金鑰在 keychain 裡的 key。
fn remote_host_secret_key(id: &str) -> String {
    format!("remote:{id}")
}

/// 決定這次連線要用哪個金鑰。
///
/// **`saved_host_id` 有值卻查不到金鑰時一定要回 `Err`，絕對不能回 `Ok(None)`。**
/// 回 `Ok(None)` 的話後端會把這次連線當成短碼模式，握手失敗的訊息會指向短碼
/// 不符——使用者看到的原因跟真正的原因毫無關係。`ConnectDialog` 現有註解記錄
/// 的 `Some("")` 陷阱就是同一類問題。
///
/// `lookup` 的回傳分三種，不能互相合併：
/// - `Ok(Some(key))`：找到了。
/// - `Ok(None)`：這台電腦真的沒有這把金鑰——使用者要重貼金鑰。
/// - `Err(detail)`：keychain 本身讀不到——重貼金鑰沒有用，是別的問題。
///
/// `lookup` 由呼叫端注入，測試才能在沒有 keychain 的環境跑。
pub fn resolve_connect_key(
    saved_host_id: Option<&str>,
    typed_key: Option<String>,
    lookup: impl FnOnce(&str) -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    match saved_host_id {
        Some(id) => match lookup(&remote_host_secret_key(id)) {
            Ok(Some(key)) => Ok(Some(key)),
            Ok(None) => Err(ERR_SAVED_KEY_MISSING.to_string()),
            // 保留底層訊息：前端用前綴比對辨識類別，訊息尾巴給人看。
            Err(detail) => Err(format!("{ERR_KEYCHAIN_UNAVAILABLE}: {detail}")),
        },
        None => Ok(typed_key),
    }
}

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::config::{ConfigStore, RemoteHost};
use crate::secret::SecretStore;

/// 前端送進來的整筆資料。`id` 為 `None` 代表新增。
#[derive(Debug, Deserialize)]
pub struct RemoteHostInput {
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: u16,
    /// 預共享金鑰。**只往這個方向走**——列出時永遠不回傳它。
    pub secret: Option<String>,
}

/// 回給前端的資料，**不含金鑰**。
#[derive(Debug, Serialize)]
pub struct RemoteHostInfo {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    /// 這台電腦的 keychain 裡有沒有這一筆的金鑰。設定檔被同步到另一台電腦時
    /// 會是 false——UI 靠它先提醒，而不是等連線失敗。
    pub has_key: bool,
}

/// 寫入設定檔（不碰 keychain），回傳這一筆的 id。
///
/// 跟兩個 command 共用，也讓設定檔那半邊在沒有 keychain 的環境測得到。
fn insert_host(
    config: &ConfigStore,
    id: Option<String>,
    name: String,
    host: String,
    port: u16,
) -> Result<String, String> {
    match id {
        Some(id) => {
            let record = RemoteHost { id: id.clone(), name, host, port };
            config.update_remote_host(record).map_err(|e| e.to_string())?;
            Ok(id)
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            let record = RemoteHost { id: id.clone(), name, host, port };
            config.add_remote_host(record).map_err(|e| e.to_string())?;
            Ok(id)
        }
    }
}

/// 空字串代表「這次沒有要改金鑰」，不是「把金鑰清空」。編輯對話框不會把既有
/// 金鑰回填（前端根本拿不到），所以送空值必須是無操作，否則使用者只改個別名
/// 就會把金鑰弄丟——語意照抄 `commands/vcs.rs` 的 add/update connection。
///
/// 抽成自由函式（不吃 `State`），這樣 `#[ignore]` 的 keychain 測試才能直接呼叫
/// 到這一段真正的邏輯，而不是在測試裡把同一段 `if let` 複製一份。
fn set_secret_if_present(
    secrets: &SecretStore,
    id: &str,
    secret: &Option<String>,
) -> Result<(), String> {
    if let Some(s) = secret {
        if !s.is_empty() {
            secrets
                .set(&remote_host_secret_key(id), s)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 刪除這一筆在 keychain 裡的金鑰。條目可能本來就沒存過金鑰，所以是 best-effort
/// ——呼叫端不該因為這裡失敗就回報整個刪除操作失敗。
fn delete_secret_best_effort(secrets: &SecretStore, id: &str) {
    let _ = secrets.delete(&remote_host_secret_key(id));
}

#[tauri::command]
pub async fn remote_hosts_list(
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<RemoteHostInfo>, String> {
    Ok(config
        .get()
        .remote_hosts
        .into_iter()
        .map(|h| RemoteHostInfo {
            has_key: secrets.has(&remote_host_secret_key(&h.id)),
            id: h.id,
            name: h.name,
            host: h.host,
            port: h.port,
        })
        .collect())
}

#[tauri::command]
pub async fn remote_hosts_add(
    input: RemoteHostInput,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let id = insert_host(&config, None, input.name, input.host, input.port)?;
    set_secret_if_present(&secrets, &id, &input.secret)?;
    Ok(id)
}

#[tauri::command]
pub async fn remote_hosts_update(
    input: RemoteHostInput,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let id = input.id.clone().ok_or("missing id")?;
    insert_host(&config, Some(id.clone()), input.name, input.host, input.port)?;
    set_secret_if_present(&secrets, &id, &input.secret)?;
    Ok(())
}

#[tauri::command]
pub async fn remote_hosts_remove(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    config.remove_remote_host(&id).map_err(|e| e.to_string())?;
    // 設定檔刪了金鑰卻留著＝keychain 裡累積永遠不會再被用到的條目。
    // 刪失敗不該讓整個操作失敗（條目可能本來就不存在）。
    delete_secret_best_effort(&secrets, &id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_secret_key_is_namespaced() {
        // 不加前綴的話會跟 provider id 撞在同一個 keychain 命名空間裡。
        assert_eq!(remote_host_secret_key("abc"), "remote:abc");
    }

    #[test]
    fn a_saved_host_uses_the_key_from_the_keychain() {
        let got = resolve_connect_key(Some("abc"), None, |k| {
            assert_eq!(k, "remote:abc");
            Ok(Some("deadbeef".into()))
        })
        .unwrap();
        assert_eq!(got, Some("deadbeef".into()));
    }

    #[test]
    fn a_saved_host_with_no_key_in_the_keychain_is_an_error() {
        // **這是這支函式存在的理由。** 回 Ok(None) 的話會被當成短碼模式，
        // 錯誤訊息指向短碼不符，跟真正的原因無關。
        let got = resolve_connect_key(Some("abc"), None, |_| Ok(None));
        assert_eq!(got, Err(ERR_SAVED_KEY_MISSING.to_string()));
    }

    #[test]
    fn a_keychain_that_cannot_be_read_is_a_different_error_from_a_missing_key() {
        // 這兩件事使用者要做的處置不同：金鑰不在這台要重貼，keychain 讀不到
        // 重貼幾次都沒用。合併成同一個錯誤等於叫使用者去修一個沒壞的東西。
        let got = resolve_connect_key(Some("abc"), None, |_| Err("keychain locked".into()));
        let msg = got.unwrap_err();
        assert!(msg.starts_with(ERR_KEYCHAIN_UNAVAILABLE), "got {msg}");
        assert!(msg.contains("keychain locked"), "底層原因要留著給人看，got {msg}");
        assert!(
            !msg.starts_with(ERR_SAVED_KEY_MISSING),
            "不能跟『這台沒有這把金鑰』撞在一起"
        );
    }

    #[test]
    fn a_saved_host_ignores_whatever_the_frontend_sent() {
        // 前端在這條路上不該填 key；就算填了也不採用，避免出現兩個來源。
        let got = resolve_connect_key(Some("abc"), Some("from-frontend".into()), |_| {
            Ok(Some("from-keychain".into()))
        })
        .unwrap();
        assert_eq!(got, Some("from-keychain".into()));
    }

    #[test]
    fn a_manual_connection_uses_the_typed_key() {
        let got = resolve_connect_key(None, Some("typed".into()), |_| {
            panic!("沒有 saved_host_id 時不該去查 keychain");
        })
        .unwrap();
        assert_eq!(got, Some("typed".into()));
    }

    #[test]
    fn a_short_code_connection_has_no_key_at_all() {
        let got = resolve_connect_key(None, None, |_| {
            panic!("沒有 saved_host_id 時不該去查 keychain");
        })
        .unwrap();
        assert_eq!(got, None);
    }

    use crate::config::ConfigStore;
    use tempfile::tempdir;

    #[test]
    fn adding_a_host_puts_it_in_the_config() {
        let dir = tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        let id = insert_host(&config, None, "辦公室".into(), "192.168.1.50".into(), 8022).unwrap();
        let hosts = config.get().remote_hosts;
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].id, id);
        assert_eq!(hosts[0].name, "辦公室");
        assert_eq!(hosts[0].port, 8022);
    }

    #[test]
    fn updating_a_host_keeps_its_id_and_does_not_add_a_second_row() {
        let dir = tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        let id = insert_host(&config, None, "舊名".into(), "1.2.3.4".into(), 8022).unwrap();
        insert_host(&config, Some(id.clone()), "新名".into(), "1.2.3.4".into(), 9000).unwrap();
        let hosts = config.get().remote_hosts;
        assert_eq!(hosts.len(), 1, "更新不該再新增一列");
        assert_eq!(hosts[0].id, id, "更新不該換掉 id——keychain 的金鑰是綁 id 的");
        assert_eq!(hosts[0].name, "新名");
        assert_eq!(hosts[0].port, 9000);
    }

    #[test]
    fn removing_a_host_takes_it_out_of_the_config() {
        let dir = tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        let id = insert_host(&config, None, "要刪的".into(), "1.2.3.4".into(), 8022).unwrap();
        let keep = insert_host(&config, None, "留著".into(), "5.6.7.8".into(), 8022).unwrap();
        config.remove_remote_host(&id).unwrap();
        let hosts = config.get().remote_hosts;
        assert_eq!(hosts.len(), 1, "只該刪掉指定的那一列");
        assert_eq!(hosts[0].id, keep);
    }

    /// 這三條碰真實 OS keychain，照 secret/mod.rs 的慣例標 #[ignore]。
    /// 手動跑：`cargo test --workspace remote_hosts_keychain -- --ignored`
    ///
    /// 都直接呼叫 production 用的自由函式（`set_secret_if_present` /
    /// `delete_secret_best_effort`），不是在測試裡另外抄一份同樣的邏輯——
    /// `#[tauri::command]` 本身因為簽章要吃 `State<'_, Arc<...>>`，在這個
    /// repo（`tauri` 依賴沒開 `test` feature，見 `tasks/dispatch.rs` 測試模組
    /// 裡的說明）沒有乾淨的辦法直接呼叫，所以測試打到兩個 command 共用、
    /// 真正做事的那一層，而不是 command 本身那層瘦的 orchestration。
    #[test]
    #[ignore]
    fn remote_hosts_keychain_update_without_a_secret_keeps_the_existing_key() {
        let secrets = SecretStore::new();
        let id = format!("test-{}", uuid::Uuid::new_v4());
        let key = remote_host_secret_key(&id);
        secrets.set(&key, "original").unwrap();

        // 模擬 remote_hosts_update 收到 secret: None（使用者只改了別名）——
        // 呼叫的是真正的 production 函式，不是複製它的邏輯。
        set_secret_if_present(&secrets, &id, &None).unwrap();

        assert_eq!(secrets.get(&key).unwrap(), Some("original".into()));
        secrets.delete(&key).unwrap();
    }

    /// **這條比 None 那條重要。** 前端拿不到已存的金鑰（刻意的設計），所以
    /// 使用者只改別名時，金鑰欄位是空的，送出來的是空字串而不是 None。
    /// 空字串若被當成「清空金鑰」，改個名字就會把金鑰弄丟，而且要等下一次
    /// 連線才會發現。
    #[test]
    #[ignore]
    fn remote_hosts_keychain_update_with_an_empty_secret_keeps_the_existing_key() {
        let secrets = SecretStore::new();
        let id = format!("test-{}", uuid::Uuid::new_v4());
        let key = remote_host_secret_key(&id);
        secrets.set(&key, "original").unwrap();

        set_secret_if_present(&secrets, &id, &Some(String::new())).unwrap();

        assert_eq!(secrets.get(&key).unwrap(), Some("original".into()));
        secrets.delete(&key).unwrap();
    }

    #[test]
    #[ignore]
    fn remote_hosts_keychain_remove_deletes_the_key() {
        let dir = tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        let secrets = SecretStore::new();
        let id = insert_host(&config, None, "要刪的".into(), "1.2.3.4".into(), 8022).unwrap();
        let key = remote_host_secret_key(&id);
        secrets.set(&key, "deadbeef").unwrap();
        assert!(secrets.has(&key), "前置條件：金鑰要先真的存進去");

        config.remove_remote_host(&id).unwrap();
        delete_secret_best_effort(&secrets, &id);

        assert!(!secrets.has(&key), "刪掉條目之後 keychain 不該還留著金鑰");
    }
}

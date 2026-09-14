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
}

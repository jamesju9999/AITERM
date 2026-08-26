//! 遠端終端機共享的 Tauri commands。
//!
//! 形狀比照 `commands/mcp_server.rs`（同樣是一個可啟停的本機 server）。
//!
//! **一條貫穿整個模組的規則：4 位驗證碼不離開 Rust。** 回傳給前端的結構
//! 沒有 sas 欄位；同意時使用者輸入的碼送進來，比對在這裡做。理由見
//! `decide` 的說明。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::pty::PtyManager;
use crate::share::registry::{AccessMode, ShareRegistry};
use crate::share::ShareServerState;

/// 某個分頁的分享狀態，給分享面板顯示。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareStatus {
    /// 這個分頁正在分享嗎。
    pub sharing: bool,
    /// 6 位短碼。沒在分享時是 `None`。
    pub code: Option<String>,
    /// server 的 port。沒在分享時是 `None`。
    pub port: Option<u16>,
}

/// 待審請求，給同意視窗顯示。
///
/// **刻意沒有 `sas` 欄位**——見 `decide` 的說明。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRequestView {
    pub request_id: String,
    pub tab_id: String,
    /// 對方自報的名字，**未經驗證**。
    pub display_name: String,
}

/// 一位觀看者，給分享面板的清單顯示。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerView {
    pub viewer_id: String,
    /// 對方自報的名字，**未經驗證**。
    pub display_name: String,
    /// `"read_only"` 或 `"control"`。
    pub mode: String,
}

/// `share_approve` 的裁決結果。
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Decision {
    /// 碼對了，觀看者已加入。
    ///
    /// 變體上這個 `rename_all` **不能省**：enum 容器層的 `rename_all` 只轉
    /// 變體名稱，不會轉變體裡的欄位。少了它，這裡會送出 `viewer_id` 而前端
    /// 型別寫的是 `viewerId`，兩邊在執行期才會對不上。
    #[serde(rename_all = "camelCase")]
    Approved { viewer_id: String },
    /// 輸入的碼跟這條連線的不符——請求已被拒絕，**不給重試**。
    CodeMismatch,
    /// 要控制權但已經有人持有。請求還在，可以改用唯讀重新裁決。
    ControlTaken,
    /// 請求已經不在了（對方斷線、或分享被停掉）。
    RequestGone,
}

/// 核准或拒絕一筆待審請求。
///
/// **驗證碼的比對在這裡做，不在前端。** 主控端的 4 位碼存在
/// `PendingRequest::sas`，從來不會被送到 JS 端——`PendingRequestView` 與
/// `PendingRequestEvent` 都沒有那個欄位。使用者必須跟對方口頭核對、把聽到
/// 的數字打進來，這裡才比對。
///
/// 若碼送到了前端，使用者會照抄畫面上的數字而不問對方，人工核對變成自欺，
/// 而整個防中間人保證的最後一哩就是那次口頭核對。
///
/// **碼不符時直接拒絕，不給重試**：攻擊者只有 1/10000 的一發機會，給重試
/// 等於送他一萬次。
///
/// 抽成自由函式（而不是寫在 `#[tauri::command]` 裡）是為了能在不起 Tauri
/// app 的情況下測試——command 本身只是很薄的轉接。
pub fn decide(
    registry: &ShareRegistry,
    request_id: &str,
    mode: AccessMode,
    typed_code: &str,
) -> Decision {
    let Some(expected) = registry.sas_for_request(request_id) else {
        return Decision::RequestGone;
    };
    if typed_code != expected {
        registry.deny(request_id);
        return Decision::CodeMismatch;
    }
    match registry.approve(request_id, mode) {
        Some(viewer_id) => Decision::Approved { viewer_id },
        // `approve` 回 None 有兩種可能：控制權被占用（請求被放回待審），
        // 或分享在裁決前被停掉（請求消失）。用請求還在不在來分辨。
        None => {
            if registry.sas_for_request(request_id).is_some() {
                Decision::ControlTaken
            } else {
                Decision::RequestGone
            }
        }
    }
}

#[tauri::command]
pub async fn share_start(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
    pty_manager: State<'_, Arc<PtyManager>>,
    app: AppHandle,
) -> Result<ShareStatus, String> {
    let port = server
        .start_if_needed(pty_manager.inner().clone(), Some(app))
        .await
        .map_err(|e| format!("啟動共享服務失敗：{e}"))?;
    let code = server.registry.start_share(tab_id);
    Ok(ShareStatus { sharing: true, code: Some(code), port: Some(port) })
}

#[tauri::command]
pub async fn share_stop(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<ShareStatus, String> {
    server.registry.stop_share(&tab_id);
    server.stop_if_idle();
    Ok(ShareStatus { sharing: false, code: None, port: None })
}

#[tauri::command]
pub async fn share_status(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<ShareStatus, String> {
    let code = server.registry.code_for_tab(&tab_id);
    let port = server.port();
    Ok(ShareStatus { sharing: code.is_some(), code, port })
}

#[tauri::command]
pub async fn share_pending(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<Vec<PendingRequestView>, String> {
    Ok(server
        .registry
        .pending(&tab_id)
        .into_iter()
        .map(|p| PendingRequestView {
            request_id: p.request_id,
            tab_id: p.tab_id,
            display_name: p.display_name,
        })
        .collect())
}

#[tauri::command]
pub async fn share_approve(
    request_id: String,
    mode: String,
    typed_code: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<Decision, String> {
    let mode = match mode.as_str() {
        "read_only" => AccessMode::ReadOnly,
        "control" => AccessMode::Control,
        other => return Err(format!("未知的存取模式：{other}")),
    };
    Ok(decide(&server.registry, &request_id, mode, &typed_code))
}

#[tauri::command]
pub async fn share_deny(
    request_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<(), String> {
    server.registry.deny(&request_id);
    Ok(())
}

#[tauri::command]
pub async fn share_viewers(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<Vec<ViewerView>, String> {
    Ok(server
        .registry
        .viewers(&tab_id)
        .into_iter()
        .map(|v| ViewerView {
            viewer_id: v.viewer_id,
            display_name: v.display_name,
            mode: match v.mode {
                AccessMode::ReadOnly => "read_only".to_string(),
                AccessMode::Control => "control".to_string(),
            },
        })
        .collect())
}

#[tauri::command]
pub async fn share_kick(
    tab_id: String,
    viewer_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<(), String> {
    server.registry.remove_viewer(&tab_id, &viewer_id);
    Ok(())
}

#[tauri::command]
pub async fn share_revoke_control(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<(), String> {
    server.registry.revoke_control(&tab_id);
    Ok(())
}

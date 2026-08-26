//! 共享連線的線上格式。
//!
//! 分工：**文字 frame 走這裡的 JSON 控制訊息，二進位 frame 就是原始位元組**
//! ——server→viewer 的二進位是 PTY 輸出，viewer→server 的二進位是按鍵。
//! PTY 位元組不套 JSON/base64，因為那會讓每個 chunk 膨脹並多一次配置。
//!
//! 計畫②的前端要照這份契約實作，改動要同步兩邊。

use serde::{Deserialize, Serialize};

use super::registry::AccessMode;

/// `AccessMode` 的線上表示。刻意與內部型別分開：內部列舉改名不該悄悄變成
/// 破壞相容性的線上格式變更。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireAccessMode {
    ReadOnly,
    Control,
}

impl From<AccessMode> for WireAccessMode {
    fn from(m: AccessMode) -> Self {
        match m {
            AccessMode::ReadOnly => WireAccessMode::ReadOnly,
            AccessMode::Control => WireAccessMode::Control,
        }
    }
}

/// 連線結束的原因。前端靠這個決定顯示哪一句話，所以是機器可讀的列舉而不是
/// 自由文字。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// 主控端按了拒絕。
    Denied,
    /// 主控端停止分享。
    HostStoppedSharing,
    /// 被分享的終端機自己結束了（shell 退出）。
    SessionClosed,
    /// 主控端單獨踢掉這位觀看者。
    KickedByHost,
    /// 短碼不存在或已作廢。
    InvalidCode,
    /// 兩端的協定版本不相容。
    ///
    /// 同區網分享的真實情境就是「同事的 AITerm 跟你的版本不同」——其中一台
    /// 還沒更新是常態，不是邊角案例。沒有這個變體的話，新版主控端送出舊版
    /// 認不得的訊息時，舊版只會在 `serde_json::from_str` 硬性失敗（實測確認
    /// serde 對未知 tag 回 `Err` 而非忽略），使用者看到的是無法解釋的斷線。
    VersionMismatch,
}

/// 這份線上格式的版本。**新增或修改任何訊息的形狀時都要往上加。**
///
/// 之所以現在就加而不是等需要時再說：事後補版本欄位本身就是一次破壞相容性
/// 的線上格式變更，而且補的時候舊版早就裝在別人機器上了。現在的成本是一個
/// 欄位，之後的成本是沒有乾淨的升級路徑。
pub const PROTOCOL_VERSION: u32 = 1;

/// 一條連線導出的 4 位 SAS。**永遠不會出現在線上格式裡**，只透過 axum 的
/// request extension 交給 ws handler，再存進 `PendingRequest` 給主控端 UI。
///
/// 每條 TLS 連線一組——這正是它能當身分保證的原因：中間人必須維持兩條獨立的
/// TLS 連線，兩邊導出的值必然不同，所以口頭核對時對不起來。
///
/// **為什麼不送給觀看端**：觀看端必須從自己那條 TLS 連線獨立算出自己那一份。
/// 如果它顯示的是主控端送來的值，中間人只要原封轉發就能讓兩邊看起來一致，
/// 整個防冒充保證歸零。讓觀看端**拿不到**這個值，比讓它拿得到卻叮嚀不要用
/// 安全得多。
///
/// 欄位是 `pub` 是為了讓 Task 6 的整合測試（住在獨立的 `tests/` crate，只碰
/// 得到 `pub` API）能注入佔位值。正式路徑只由 Task 8 的 TLS accept 迴圈建構，
/// 不要在業務邏輯裡自己 `ConnectionSas(...)` 生一個出來。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionSas(pub String);

/// 觀看端 → 主控端。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// 連上 ws 後的第一則訊息。在收到 `Granted` 之前，觀看端不會收到任何
    /// PTY 位元組。
    ///
    /// `protocol_version` 讓主控端能在握手第一步就發現版本落差，送出乾淨的
    /// `Ended { VersionMismatch }`，而不是讓後續訊息解析失敗變成無法解釋的
    /// 斷線。
    Join { protocol_version: u32, code: String, display_name: String },
}

/// 主控端 → 觀看端。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// 請求已送達，正在等主控端裁決。
    ///
    /// **刻意不帶任何 SAS。** 觀看端要顯示的 4 位驗證碼必須由它自己那條 TLS
    /// 連線導出——見 `ConnectionSas` 的說明。主控端那一份存在 `PendingRequest`
    /// 裡，走 Tauri 給主控端自己的 UI，不上線路。
    AwaitingApproval,
    /// 已獲准。`cols`/`rows` 是主控端的終端機尺寸——觀看端必須照這個建立
    /// xterm，不能用自己的視窗大小。緊接著會來一個二進位 frame 作為重播。
    Granted { mode: WireAccessMode, cols: u16, rows: u16 },
    /// 主控端 resize 了，觀看端重新 fit。
    Resize { cols: u16, rows: u16 },
    /// 控制權變動（被授予或被收回）。
    ///
    /// 由 Task 6 的 ws 迴圈在 `share_watch` 輪詢時偵測 mode 變化後送出——跟
    /// `KickedByHost` 同一個機制。主控端呼叫 `ShareRegistry` 的
    /// `grant_control`/`revoke_control` 之後，觀看端靠這則訊息才知道自己現在
    /// 能不能打字。
    ControlChanged { mode: WireAccessMode },
    /// 觀看端落後太多，接下來的二進位 frame 是全量重播；收到這個要先清空
    /// 畫面再套用。漏掉的位元組可能截斷 ANSI 逃脫序列，帶著壞掉的畫面繼續
    /// 是不會自己好的。
    Resync,
    /// 連線結束。送出後 server 立即關閉這條 ws。
    Ended { reason: EndReason },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_round_trips_through_json() {
        let msg = ClientMessage::Join {
            protocol_version: PROTOCOL_VERSION,
            code: "384719".to_string(),
            display_name: "Alice".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"join\""), "got {json}");
        assert!(json.contains("\"protocol_version\":1"), "got {json}");
        let back: ClientMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn an_unknown_server_message_fails_to_parse_rather_than_being_ignored() {
        // 這不是我們想要的行為，是我們必須知道的行為：serde 對未知 tag 回
        // `Err`，所以新版主控端送出舊版認不得的變體時，舊版觀看端會硬性解析
        // 失敗。這正是 `PROTOCOL_VERSION` 與 `EndReason::VersionMismatch`
        // 存在的理由——在握手第一步就擋掉，而不是讓它變成無法解釋的斷線。
        let unknown = r#"{"type":"some_future_variant","payload":1}"#;
        let parsed: Result<ServerMessage, _> = serde_json::from_str(unknown);
        assert!(
            parsed.is_err(),
            "serde silently accepted an unknown variant; the version-guard \
             reasoning in PROTOCOL_VERSION's doc comment is built on it erroring"
        );
    }

    #[test]
    fn wire_access_mode_does_not_invert_the_internal_one() {
        // 對調這兩個分支不會被任何序列化測試抓到，但會讓 `Granted.mode` 回報
        // 相反的存取層級：真正拿到控制權的人前端顯示唯讀而打不了字，唯讀的人
        // 以為自己能控制、白打一堆字進黑洞。
        assert_eq!(
            WireAccessMode::from(AccessMode::ReadOnly),
            WireAccessMode::ReadOnly
        );
        assert_eq!(
            WireAccessMode::from(AccessMode::Control),
            WireAccessMode::Control
        );
    }

    #[test]
    fn granted_carries_the_access_mode_and_the_host_screen_size() {
        let msg = ServerMessage::Granted {
            mode: WireAccessMode::ReadOnly,
            cols: 120,
            rows: 40,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"granted\""), "got {json}");
        assert!(json.contains("\"mode\":\"read_only\""), "got {json}");
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn ended_carries_a_machine_readable_reason() {
        let msg = ServerMessage::Ended { reason: EndReason::HostStoppedSharing };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"reason\":\"host_stopped_sharing\""), "got {json}");
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn awaiting_approval_never_carries_a_sas() {
        // 這則訊息是送給觀看端的，而觀看端絕不該從線上取得任何 SAS——它必須
        // 從自己那條 TLS 連線獨立導出。若哪天有人「順手」把主控端的 sas 加
        // 回這則訊息，中間人原封轉發就能讓兩邊看起來一致，防冒充保證歸零。
        let msg = ServerMessage::AwaitingApproval;
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"awaiting_approval\""), "got {json}");
        assert!(
            !json.contains("sas"),
            "AwaitingApproval must not carry a SAS onto the wire; got {json}"
        );
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn every_end_reason_survives_a_round_trip() {
        // 前端要靠 reason 決定顯示哪一句話，漏掉任何一個都會變成「未知錯誤」。
        //
        // 用沒有 `_` 萬用分支的 match 而不是手寫陣列：陣列漏掉新變體時測試
        // 照樣綠燈（實測確認過），而這個 match 會讓**編譯失敗**，強迫新增
        // 變體的人回來這裡加一行。名字承諾了「every」，就要有東西撐住它。
        fn round_trip(reason: EndReason) {
            let msg = ServerMessage::Ended { reason };
            let json = serde_json::to_string(&msg).unwrap();
            let back: ServerMessage = serde_json::from_str(&json).unwrap();
            assert_eq!(back, msg, "round trip failed for {reason:?}");
        }

        let all = [
            EndReason::Denied,
            EndReason::HostStoppedSharing,
            EndReason::SessionClosed,
            EndReason::KickedByHost,
            EndReason::InvalidCode,
            EndReason::VersionMismatch,
        ];
        for reason in all {
            round_trip(reason);
            // 這個 match 沒有 `_` 分支，所以新增 EndReason 變體時這裡會編譯
            // 失敗——那是提醒你把它加進上面的 `all` 陣列。
            match reason {
                EndReason::Denied
                | EndReason::HostStoppedSharing
                | EndReason::SessionClosed
                | EndReason::KickedByHost
                | EndReason::InvalidCode
                | EndReason::VersionMismatch => {}
            }
        }
    }
}

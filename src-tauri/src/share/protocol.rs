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
}

/// 一條連線導出的 4 位 SAS，透過 axum 的 request extension 交給 ws handler。
///
/// 每條 TLS 連線一組——這正是它能當身分保證的原因：中間人必須維持兩條獨立的
/// TLS 連線，兩邊導出的值必然不同。Task 8 的 TLS accept 迴圈負責填入真值；
/// Task 6 的明文 server 只在測試裡注入佔位值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionSas(pub String);

/// 觀看端 → 主控端。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// 連上 ws 後的第一則訊息。在收到 `Granted` 之前，觀看端不會收到任何
    /// PTY 位元組。
    Join { code: String, display_name: String },
}

/// 主控端 → 觀看端。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// 請求已送達，正在等主控端裁決。
    ///
    /// `sas` 是**主控端從自己這條 TLS 連線導出**的 4 位驗證碼，僅供主控端 UI
    /// 顯示。觀看端必須算自己那一份、跟這個值口頭核對——**絕對不要**直接顯示
    /// 這裡收到的值當作自己的 SAS：中間人只要原封轉發就能讓兩邊看起來一致，
    /// 整個防冒充保證就沒了。見 `share::tls`。
    ///
    /// 型別在 Task 5 就定成帶欄位的形狀，即使 TLS 要到 Task 8 才接上——線上
    /// 契約不該因為實作順序而中途變形。Task 6 的明文 server 送的是注入的
    /// 佔位值。
    AwaitingApproval { sas: String },
    /// 已獲准。`cols`/`rows` 是主控端的終端機尺寸——觀看端必須照這個建立
    /// xterm，不能用自己的視窗大小。緊接著會來一個二進位 frame 作為重播。
    Granted { mode: WireAccessMode, cols: u16, rows: u16 },
    /// 主控端 resize 了，觀看端重新 fit。
    Resize { cols: u16, rows: u16 },
    /// 控制權變動（被授予或被收回）。
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
            code: "384719".to_string(),
            display_name: "Alice".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"join\""), "got {json}");
        let back: ClientMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
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
    fn awaiting_approval_carries_the_hosts_sas() {
        let msg = ServerMessage::AwaitingApproval { sas: "4917".to_string() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"awaiting_approval\""), "got {json}");
        assert!(json.contains("\"sas\":\"4917\""), "got {json}");
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn every_end_reason_survives_a_round_trip() {
        // 前端要靠 reason 決定顯示哪一句話，漏掉任何一個都會變成「未知錯誤」。
        for reason in [
            EndReason::Denied,
            EndReason::HostStoppedSharing,
            EndReason::SessionClosed,
            EndReason::KickedByHost,
            EndReason::InvalidCode,
        ] {
            let msg = ServerMessage::Ended { reason };
            let json = serde_json::to_string(&msg).unwrap();
            let back: ServerMessage = serde_json::from_str(&json).unwrap();
            assert_eq!(back, msg, "round trip failed for {reason:?}");
        }
    }
}

use serde::Serialize;

pub fn data_event_name(session_id: &str) -> String {
    format!("pty://data/{session_id}")
}

pub fn closed_event_name(session_id: &str) -> String {
    format!("pty://closed/{session_id}")
}

pub fn elevation_state_event_name(session_id: &str) -> String {
    format!("pty://elevation-state/{session_id}")
}

pub fn elevation_suggested_event_name(session_id: &str) -> String {
    format!("pty://elevation-suggested/{session_id}")
}

#[derive(Debug, Clone, Serialize)]
pub struct PtyDataPayload {
    /// Base64-encoded bytes. xterm.js expects a binary stream, but Tauri events
    /// marshal JSON so we base64 on the wire and decode on the frontend.
    pub base64: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PtyClosedPayload {
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ElevationStatePayload {
    pub elevated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ElevationSuggestedPayload {
    /// 被判定為權限不足而失敗的那條指令文字，讓前端 banner 顯示，也讓
    /// `pty_elevate` 就緒後可以自動重送。
    pub failed_command: String,
}

use serde::Serialize;

pub fn data_event_name(session_id: &str) -> String {
    format!("pty://data/{session_id}")
}

pub fn closed_event_name(session_id: &str) -> String {
    format!("pty://closed/{session_id}")
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

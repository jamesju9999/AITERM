use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum PtyError {
    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("failed to spawn shell: {0}")]
    SpawnFailed(String),

    #[error("no suitable shell found on this system")]
    NoShellAvailable,

    #[error("pty io error: {0}")]
    Io(String),

    #[error("pty internal error: {0}")]
    Internal(String),
}

impl From<std::io::Error> for PtyError {
    fn from(e: std::io::Error) -> Self {
        PtyError::Io(e.to_string())
    }
}

impl From<anyhow::Error> for PtyError {
    fn from(e: anyhow::Error) -> Self {
        PtyError::Internal(e.to_string())
    }
}

pub type PtyResult<T> = Result<T, PtyError>;

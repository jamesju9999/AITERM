// src-tauri/src/mail/client.rs
use tokio::net::TcpStream;
use futures_util::TryStreamExt;

#[derive(Debug, thiserror::Error)]
pub enum MailClientError {
    #[error("connection error: {0}")]
    Connect(String),
    #[error("login failed: {0}")]
    Login(String),
    #[error("IMAP command failed: {0}")]
    Command(String),
}

pub struct RawMessage {
    pub uid: i64,
    pub raw: Vec<u8>,
}

/// Fetch every message with UID greater than `since_uid` (or the whole
/// mailbox if `since_uid` is `None`, i.e. first poll) from INBOX, using
/// `BODY.PEEK[]` so the server's `\Seen` flag is never touched — AITerm
/// tracks read/unread locally (see `db/mail.rs::mark_read_locally`) so it
/// doesn't clobber the read state the user sees in their phone's mail app.
pub async fn fetch_new_messages(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    since_uid: Option<i64>,
) -> Result<Vec<RawMessage>, MailClientError> {
    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|e| MailClientError::Connect(e.to_string()))?;

    let native_connector = native_tls::TlsConnector::new()
        .map_err(|e| MailClientError::Connect(e.to_string()))?;
    let connector = tokio_native_tls::TlsConnector::from(native_connector);
    let tls_stream = connector
        .connect(host, tcp)
        .await
        .map_err(|e| MailClientError::Connect(e.to_string()))?;

    let client = async_imap::Client::new(tls_stream);
    let mut session = client
        .login(username, password)
        .await
        .map_err(|(e, _client)| MailClientError::Login(e.to_string()))?;

    session
        .select("INBOX")
        .await
        .map_err(|e| MailClientError::Command(e.to_string()))?;

    let search_query = match since_uid {
        Some(uid) => format!("UID {}:*", uid + 1),
        None => "1:*".to_string(),
    };
    let uids = session
        .uid_search(&search_query)
        .await
        .map_err(|e| MailClientError::Command(e.to_string()))?;

    let mut new_uids: Vec<u32> = uids
        .into_iter()
        .filter(|uid| since_uid.map_or(true, |since| (*uid as i64) > since))
        .collect();
    new_uids.sort_unstable();

    if new_uids.is_empty() {
        session.logout().await.ok();
        return Ok(Vec::new());
    }

    let uid_set = new_uids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
    let fetches = session
        .uid_fetch(&uid_set, "(UID BODY.PEEK[])")
        .await
        .map_err(|e| MailClientError::Command(e.to_string()))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| MailClientError::Command(e.to_string()))?;

    let mut messages = Vec::new();
    for fetch in fetches {
        if let (Some(uid), Some(body)) = (fetch.uid, fetch.body()) {
            messages.push(RawMessage { uid: uid as i64, raw: body.to_vec() });
        }
    }

    session.logout().await.ok();
    Ok(messages)
}

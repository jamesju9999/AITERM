//! 成員、邀請與 token。
//!
//! 邀請碼 = 6 字元公開編號 + 16 字元祕密（約 80 bit），一次性、15 分鐘內有效。
//! 祕密**不上線**：配對時雙方各對「這條 TLS 連線的 exporter」做 HMAC 互證（見 `pairing`），
//! 所以中間人沒有邀請碼就既偽造不了證明，也無法冒充 server。
//! 長期 token 只存雜湊；比對靠雜湊後查表（256 bit 隨機值）。
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

pub const INVITE_TTL_SECS: i64 = 15 * 60;
pub const MAX_INVITE_ATTEMPTS: i64 = 5;
/// 去掉容易混淆的字元（I、L、O、0、1）的 base32。
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const ID_LEN: usize = 6;
const SECRET_LEN: usize = 16;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Member,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Member => "member",
            Role::Admin => "admin",
        }
    }
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "member" => Some(Role::Member),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Me {
    pub id: String,
    pub name: String,
    pub role: Role,
}

fn random_chars(n: usize) -> String {
    let mut rng = rand::rng();
    (0..n)
        .map(|_| {
            let mut b = [0u8; 1];
            loop {
                rng.fill_bytes(&mut b);
                // 拒絕取樣，避免 modulo 偏差。
                if (b[0] as usize) < ALPHABET.len() * (256 / ALPHABET.len()) {
                    return ALPHABET[b[0] as usize % ALPHABET.len()] as char;
                }
            }
        })
        .collect()
}

/// 每 4 個字元以 `-` 分組，方便抄寫。
pub fn format_code(id: &str, secret: &str) -> String {
    let all = format!("{id}{secret}");
    all.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
}

/// 使用者貼上的邀請碼 → `(公開編號, 祕密)`。忽略大小寫、空白與 `-`；長度不對回 `None`。
pub fn parse_code(input: &str) -> Option<(String, String)> {
    let clean: String = input.chars().filter(|c| !c.is_whitespace() && *c != '-').map(|c| c.to_ascii_uppercase()).collect();
    if clean.len() != ID_LEN + SECRET_LEN || !clean.bytes().all(|b| ALPHABET.contains(&b)) {
        return None;
    }
    Some((clean[..ID_LEN].to_string(), clean[ID_LEN..].to_string()))
}

/// 配對互證用的金鑰：由祕密導出，與公開編號無關。
pub fn invite_key(secret: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"aiterm-intake-invite-v1");
    h.update(secret.as_bytes());
    h.finalize().into()
}

pub struct NewInvite {
    pub code: String,
    pub expires_at: i64,
}

pub async fn create_invite(pool: &SqlitePool, name: &str, role: Role, now: i64) -> Result<NewInvite, sqlx::Error> {
    let id = random_chars(ID_LEN);
    let secret = random_chars(SECRET_LEN);
    let expires_at = now + INVITE_TTL_SECS;
    sqlx::query("INSERT INTO invites(id, key_hex, name, role, expires_at) VALUES (?,?,?,?,?)")
        .bind(&id)
        .bind(aiterm_core::share::tls::hex_of(&invite_key(&secret)))
        .bind(name)
        .bind(role.as_str())
        .bind(expires_at)
        .execute(pool)
        .await?;
    Ok(NewInvite { code: format_code(&id, &secret), expires_at })
}

#[derive(Debug, Clone)]
pub struct InviteRow {
    pub id: String,
    pub key: Vec<u8>,
    pub name: String,
    pub role: Role,
}

/// 還有效（未用、未過期、錯誤次數未超過）的邀請。
pub async fn find_invite(pool: &SqlitePool, id: &str, now: i64) -> Option<InviteRow> {
    let row: Option<(String, String, String, String)> =
        sqlx::query_as("SELECT id, key_hex, name, role FROM invites WHERE id = ? AND used_at IS NULL AND expires_at > ? AND attempts < ?")
            .bind(id)
            .bind(now)
            .bind(MAX_INVITE_ATTEMPTS)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let (id, key_hex, name, role) = row?;
    Some(InviteRow { id, key: aiterm_core::share::tls::decode_hex(&key_hex)?, name, role: Role::parse(&role)? })
}

/// 記一次失敗；超過上限邀請就作廢（`find_invite` 不再回傳）。
pub async fn record_invite_failure(pool: &SqlitePool, id: &str) {
    let _ = sqlx::query("UPDATE invites SET attempts = attempts + 1 WHERE id = ?").bind(id).execute(pool).await;
}

/// 邀請已兌換：標記已用，發一組新的長期 token（只回傳這一次）。已用過回 `None`。
pub async fn redeem(pool: &SqlitePool, invite_id: &str, now: i64) -> Result<Option<(Me, String)>, sqlx::Error> {
    let done = sqlx::query("UPDATE invites SET used_at = ? WHERE id = ? AND used_at IS NULL AND expires_at > ?").bind(now).bind(invite_id).bind(now).execute(pool).await?;
    if done.rows_affected() != 1 {
        return Ok(None);
    }
    let (name, role): (String, String) = sqlx::query_as("SELECT name, role FROM invites WHERE id = ?").bind(invite_id).fetch_one(pool).await?;
    let role = Role::parse(&role).unwrap_or(Role::Member);
    Ok(Some(issue_token(pool, &name, role, now).await?))
}

fn token_hash(token: &str) -> String {
    aiterm_core::share::tls::hex_of(&Sha256::digest(token.as_bytes()))
}

pub async fn issue_token(pool: &SqlitePool, name: &str, role: Role, now: i64) -> Result<(Me, String), sqlx::Error> {
    let mut raw = [0u8; 32];
    rand::rng().fill_bytes(&mut raw);
    let token = aiterm_core::share::tls::hex_of(&raw);
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO members(id, name, role, token_hash, created_at) VALUES (?,?,?,?,?)")
        .bind(&id)
        .bind(name)
        .bind(role.as_str())
        .bind(token_hash(&token))
        .bind(now)
        .execute(pool)
        .await?;
    Ok((Me { id, name: name.to_string(), role }, token))
}

/// 驗證 token；撤銷過、不存在、格式錯誤一律 `None`（呼叫端不可區分原因）。
pub async fn verify_token(pool: &SqlitePool, token: &str) -> Option<Me> {
    if token.len() != 64 {
        return None;
    }
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT id, name, role FROM members WHERE token_hash = ? AND revoked_at IS NULL").bind(token_hash(token)).fetch_optional(pool).await.ok().flatten();
    let (id, name, role) = row?;
    Some(Me { id, name, role: Role::parse(&role)? })
}

#[derive(Serialize, Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct MemberRow {
    pub id: String,
    pub name: String,
    pub role: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

pub async fn list(pool: &SqlitePool) -> Vec<MemberRow> {
    sqlx::query_as("SELECT id, name, role, created_at, revoked_at FROM members ORDER BY created_at, name").fetch_all(pool).await.unwrap_or_default()
}

/// 以 id 或名稱撤銷（名稱重複時全部撤銷）。回傳撤銷人數。
pub async fn revoke(pool: &SqlitePool, id_or_name: &str, now: i64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE members SET revoked_at = ? WHERE (id = ? OR name = ?) AND revoked_at IS NULL").bind(now).bind(id_or_name).bind(id_or_name).execute(pool).await?.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        crate::db::open_memory().await.unwrap().server
    }

    #[test]
    fn a_code_round_trips_through_formatting_and_forgiving_parsing() {
        let c = format_code("ABCDEF", "GHJKMNPQRSTUVWXY");
        assert_eq!(c, "ABCD-EFGH-JKMN-PQRS-TUVW-XY");
        assert_eq!(parse_code(&c), Some(("ABCDEF".into(), "GHJKMNPQRSTUVWXY".into())));
        assert_eq!(parse_code("  abcd-efgh-jkmn-pqrs-tuvw-xy \n"), parse_code(&c), "忽略大小寫與空白");
        assert_eq!(parse_code("ABCD-EFGH"), None, "太短");
        assert_eq!(parse_code("ABCD-EFGH-JKMN-PQRS-TUVW-X0"), None, "0 不在字母表");
    }

    #[tokio::test]
    async fn an_invite_can_be_redeemed_once_for_a_token_that_verifies() {
        let p = pool().await;
        let inv = create_invite(&p, "小明", Role::Member, 1000).await.unwrap();
        let (id, _) = parse_code(&inv.code).unwrap();
        assert!(find_invite(&p, &id, 1001).await.is_some());
        let (me, token) = redeem(&p, &id, 1001).await.unwrap().expect("第一次兌換成功");
        assert_eq!((me.name.as_str(), me.role), ("小明", Role::Member));
        assert_eq!(verify_token(&p, &token).await, Some(me));
        assert!(redeem(&p, &id, 1002).await.unwrap().is_none(), "邀請只能用一次");
        assert!(find_invite(&p, &id, 1002).await.is_none());
    }

    #[tokio::test]
    async fn an_expired_invite_is_not_found_or_redeemable() {
        let p = pool().await;
        let inv = create_invite(&p, "小明", Role::Admin, 1000).await.unwrap();
        let (id, _) = parse_code(&inv.code).unwrap();
        let after = 1000 + INVITE_TTL_SECS + 1;
        assert!(find_invite(&p, &id, after).await.is_none());
        assert!(redeem(&p, &id, after).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn too_many_wrong_attempts_burn_the_invite() {
        let p = pool().await;
        let inv = create_invite(&p, "小明", Role::Member, 1000).await.unwrap();
        let (id, _) = parse_code(&inv.code).unwrap();
        for _ in 0..MAX_INVITE_ATTEMPTS {
            assert!(find_invite(&p, &id, 1001).await.is_some());
            record_invite_failure(&p, &id).await;
        }
        assert!(find_invite(&p, &id, 1001).await.is_none(), "猜太多次就作廢，即使還沒過期");
    }

    #[tokio::test]
    async fn tokens_are_stored_hashed_and_revocation_takes_effect_immediately() {
        let p = pool().await;
        let (me, token) = issue_token(&p, "小華", Role::Admin, 5).await.unwrap();
        let stored: String = sqlx::query_scalar("SELECT token_hash FROM members WHERE id = ?").bind(&me.id).fetch_one(&p).await.unwrap();
        assert!(!stored.contains(&token) && stored != token);
        assert_eq!(verify_token(&p, &token).await.map(|m| m.role), Some(Role::Admin));
        assert_eq!(revoke(&p, "小華", 9).await.unwrap(), 1);
        assert_eq!(verify_token(&p, &token).await, None);
        assert_eq!(revoke(&p, "小華", 10).await.unwrap(), 0, "重複撤銷不再計數");
    }

    #[tokio::test]
    async fn junk_tokens_are_rejected_without_error() {
        let p = pool().await;
        for t in ["", "abc", &"0".repeat(64), &"z".repeat(64)] {
            assert_eq!(verify_token(&p, t).await, None);
        }
    }
}

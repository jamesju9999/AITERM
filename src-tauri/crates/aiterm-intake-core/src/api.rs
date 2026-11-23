//! Server 與 Client 共用的線上型別（`/v1`）。
use serde::{Deserialize, Serialize};

/// 協定主版本。Client 看到不認識的版本就拒絕並說明要更新哪一邊。
pub const PROTOCOL: u32 = 1;

/// 配對時從 TLS 連線導出互證金鑰用的 label（兩端必須相同）。
pub const PAIR_EXPORTER_LABEL: &[u8] = b"EXPERIMENTAL aiterm intake pair v1";
/// 配對請求中 Client 的證明（hex）所在的 header。
pub const HEADER_CLIENT_PROOF: &str = "x-aiterm-proof";
/// 配對回應中 Server 的證明（hex）所在的 header。
pub const HEADER_HOST_PROOF: &str = "x-aiterm-host-proof";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Hello {
    pub server_version: String,
    pub protocol: u32,
}

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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PairRequest {
    pub invite_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PairResponse {
    pub token: String,
    pub me: Me,
    /// Server 憑證的 SHA-256 指紋（hex）；Client 釘住它。
    pub fingerprint: String,
}

/// WebSocket 推播：`{"type":"changed","seq":N}`。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Event {
    pub r#type: String,
    pub seq: u64,
}

impl Event {
    pub fn changed(seq: u64) -> Self {
        Self { r#type: "changed".into(), seq }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ApiError {
    pub error: String,
}


/// 邀請碼規則：6 字元公開編號 + 16 字元祕密（約 80 bit），兩端共用同一份。
pub mod invite {
    use sha2::{Digest, Sha256};

    /// 去掉容易混淆的字元（I、L、O、0、1）的 base32。
    pub const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    pub const ID_LEN: usize = 6;
    pub const SECRET_LEN: usize = 16;

    /// 每 4 個字元以 `-` 分組，方便抄寫。
    pub fn format_code(id: &str, secret: &str) -> String {
        let all = format!("{id}{secret}");
        all.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
    }

    /// 使用者貼上的邀請碼 → `(公開編號, 祕密)`。忽略大小寫、空白與 `-`；長度或字元不對回 `None`。
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_code_round_trips_through_formatting_and_forgiving_parsing() {
            let c = format_code("ABCDEF", "GHJKMNPQRSTUVWXY");
            assert_eq!(c, "ABCD-EFGH-JKMN-PQRS-TUVW-XY");
            assert_eq!(parse_code(&c), Some(("ABCDEF".into(), "GHJKMNPQRSTUVWXY".into())));
            assert_eq!(parse_code("  abcd-efgh-jkmn-pqrs-tuvw-xy \n"), parse_code(&c), "忽略大小寫與空白");
            assert_eq!(parse_code("ABCD-EFGH"), None, "太短");
            assert_eq!(parse_code("ABCD-EFGH-JKMN-PQRS-TUVW-X0"), None, "0 不在字母表");
        }

        #[test]
        fn the_key_depends_only_on_the_secret() {
            assert_eq!(invite_key("S"), invite_key("S"));
            assert_ne!(invite_key("S"), invite_key("T"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_shapes_are_stable() {
        assert_eq!(serde_json::to_string(&Event::changed(3)).unwrap(), r#"{"type":"changed","seq":3}"#);
        assert_eq!(serde_json::to_string(&Role::Admin).unwrap(), r#""admin""#);
        assert_eq!(Role::parse("member"), Some(Role::Member));
        assert_eq!(Role::parse("root"), None);
        assert_eq!(PROTOCOL, 1);
    }
}

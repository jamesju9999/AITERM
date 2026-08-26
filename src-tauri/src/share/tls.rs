//! 共享連線的傳輸安全。
//!
//! 兩件事：
//! 1. **加密**——每次分享產生一組臨時自簽憑證，連線走 TLS 1.3。
//! 2. **防冒充**——自簽憑證本身擋不住中間人（觀看端沒有任何先驗資訊可判斷
//!    該信任哪張憑證），所以身分保證來自 **SAS 人工核對**：雙方各自從 TLS
//!    連線導出（RFC 5705）同一份金鑰 material，混合 host/viewer 各自的
//!    nonce 算成 4 位數顯示在畫面上，由使用者口頭核對。
//!
//! **為什麼不能只靠「TLS 連線不同、material 就不同」**：TLS 1.3 裡 server
//! 是後手——它看到 ClientHello 的 key share 之後才選自己的。中間人對觀看端
//! 扮演 server 時，可以先跟真的主控端跑完一條連線拿到目標值，這時**還沒送出
//! ServerHello**，能在本機反覆換 ephemeral key、推導候選 SAS，湊中目標才真的
//! 送出去——4 位數的搜尋空間在一秒內就能窮舉，換更強的雜湊也救不了（輸出空間
//! 沒變）。真正的修法是消掉這個後手優勢：見 `commit_for`/`sas_from_parts` 與
//! `share::server` 的承諾流程（ZRTP／RFC 6189 的結構）。
//!
//! 為什麼不用 PAKE（SPAKE2）：那樣可以省掉人工核對，但 `spake2` crate 目前
//! 只有 pre-release 版（0.5.0-pre.0），不適合放在安全關鍵路徑上。rustls 已
//! 經在依賴樹裡。見設計文件的「安全契約」。

use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// RFC 5705 匯出用的標籤。**連線雙方必須完全一致**——不一致會算出不同的
/// SAS，使用者會看到「對不上」而誤以為遭到攻擊。版本號在字串裡，未來要改
/// 演算法時連同標籤一起換。
///
/// `EXPERIMENTAL` 前綴照 RFC 5705 慣例：該 RFC 允許此前綴不必註冊。
pub const SAS_EXPORTER_LABEL: &[u8] = b"EXPERIMENTAL aiterm share sas v1";

/// 從匯出的金鑰 material 取幾個位元組來算 SAS。
pub const SAS_MATERIAL_LEN: usize = 32;

/// nonce 的長度。32 byte 遠超過需要——重點是攻擊者猜不到，不是熵要剛好。
pub const NONCE_LEN: usize = 32;

/// 一次分享用的臨時 TLS 身分。分享停止就丟棄——重用會讓不同場次的連線可以
/// 被外部關聯起來。
pub struct ShareIdentity {
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
}

impl ShareIdentity {
    /// 產生一組新的自簽憑證。CN 沒有意義（觀看端不驗證它，身分來自 SAS），
    /// 但仍填一個可辨識的值方便除錯。
    pub fn generate() -> anyhow::Result<Self> {
        let cert = rcgen::generate_simple_self_signed(vec!["aiterm-share".to_string()])?;
        Ok(Self {
            cert_der: CertificateDer::from(cert.cert.der().to_vec()),
            key_der: PrivateKeyDer::try_from(cert.key_pair.serialize_der())
                .map_err(|e| anyhow::anyhow!("serialise share key: {e}"))?,
        })
    }
}

/// 從承諾流程的三份材料算出 4 位驗證碼。
///
/// 用 SHA-256 而不是自製折疊：`sha2` 本來就在依賴樹裡（`Cargo.toml` 的
/// `sha2 = "0.10"`），成本只有幾行。**但要清楚知道這不是安全性的來源**——
/// 輸出空間就是 10⁴，換任何雜湊都一樣。真正擋住中間人的是 `host_nonce` 的
/// 承諾流程（見這個 task 的說明），不是雜湊強度。
pub fn sas_from_parts(host_nonce: &[u8], viewer_nonce: &[u8], exporter: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    // 長度前綴，避免 (a‖b) 與 (a'‖b') 在不同切分下碰撞。
    for part in [host_nonce, viewer_nonce, exporter] {
        h.update((part.len() as u32).to_be_bytes());
        h.update(part);
    }
    let digest = h.finalize();
    let n = u32::from_be_bytes(digest[..4].try_into().expect("sha256 has 32 bytes"));
    format!("{:04}", n % 10_000)
}

/// 產生一組 32 byte 的 nonce。
pub fn fresh_nonce() -> [u8; NONCE_LEN] {
    use rand::RngCore;
    let mut n = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut n);
    n
}

/// 主控端對自己 nonce 的承諾。
pub fn commit_for(nonce: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(nonce);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// 從一條已完成握手的 TLS 連線匯出金鑰 material。
///
/// 吃 `&ConnectionCommon<Data>`（`ServerConnection`/`ClientConnection` 都 deref
/// 到它）而不是吃 closure：closure 版在型別上允許呼叫端塞任何 `[u8; 32]` 進來，
/// 包括接線時暫時放的假資料。label 與 context 也鎖死在這裡，呼叫端改不到。
pub fn exporter_material<Data>(
    conn: &rustls::ConnectionCommon<Data>,
) -> anyhow::Result<[u8; SAS_MATERIAL_LEN]> {
    let material = conn
        .export_keying_material([0u8; SAS_MATERIAL_LEN], SAS_EXPORTER_LABEL, None)
        .map_err(|e| anyhow::anyhow!("export keying material: {e}"))?;
    Ok(material)
}

/// 把位元組編成小寫 hex 字串，上線用（`SasCommit`/`SasNonce`/
/// `AwaitingApproval` 都走 hex，不走 base64——省一個依賴，且 nonce/commit
/// 不需要 base64 的密度優勢）。
pub fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `hex_of` 的反向操作。格式錯誤（非偶數長度、非 hex 字元）回 `None`——呼叫端
/// 要把這個當成「對方送了壞資料」處理，而不是 panic。
pub fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_cert_has_a_private_key_and_a_der_chain() {
        let ident = ShareIdentity::generate().expect("generate");
        assert!(!ident.cert_der.is_empty());
        assert!(!ident.key_der.secret_der().is_empty());
    }

    #[test]
    fn two_identities_are_different() {
        // 每次分享產生一組新的臨時憑證——重用會讓不同場次的連線可以被關聯。
        let a = ShareIdentity::generate().expect("a");
        let b = ShareIdentity::generate().expect("b");
        assert_ne!(a.cert_der, b.cert_der);
    }

    #[test]
    fn the_same_three_parts_give_the_same_four_digit_code() {
        let a = sas_from_parts(b"host", b"viewer", b"exporter");
        let b = sas_from_parts(b"host", b"viewer", b"exporter");
        assert_eq!(a, b);
        assert_eq!(a.len(), 4, "SAS should be 4 chars, got {a:?}");
        assert!(a.chars().all(|c| c.is_ascii_digit()), "got {a:?}");
    }

    #[test]
    fn different_exporter_bytes_give_a_different_code() {
        let a = sas_from_parts(b"host", b"viewer", &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        let b = sas_from_parts(b"host", b"viewer", &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x89]);
        assert_ne!(a, b);
    }

    #[test]
    fn the_sas_label_is_pinned() {
        // 標籤是連線雙方必須一致的常數。改動它會讓新舊版本算出不同的 SAS，
        // 使用者會看到「對不上」而以為被攻擊——所以固定住並讓改動很明顯。
        assert_eq!(SAS_EXPORTER_LABEL, b"EXPERIMENTAL aiterm share sas v1");
    }

    #[test]
    fn a_sas_below_a_thousand_is_still_four_digits() {
        // 補零那條分支要真的被走到。用搜尋找一組會折出 <1000 的輸入，而不是
        // 賭固定測資剛好落在那裡——原本的測試就是因為測資都 ≥1000，
        // 把 `{:04}` 改成 `{}` 也照樣綠燈（已實測）。
        let mut found = None;
        for i in 0u32..100_000 {
            let s = sas_from_parts(&i.to_be_bytes(), b"v", b"e");
            if s.starts_with('0') {
                found = Some(s);
                break;
            }
        }
        let s = found.expect("no input in 100k produced a SAS below 1000");
        assert_eq!(s.len(), 4, "zero padding was dropped: {s}");
    }

    #[test]
    fn swapping_the_two_nonces_changes_the_sas() {
        // 承諾流程的兩個 nonce 角色不同，SAS 必須把它們分開。若哪天有人把
        // 組合方式改成順序無關（例如 XOR），這個測試會抓到。
        let a = [0xAAu8; NONCE_LEN];
        let b = [0xBBu8; NONCE_LEN];
        assert_ne!(
            sas_from_parts(&a, &b, b"exporter"),
            sas_from_parts(&b, &a, b"exporter")
        );
    }

    #[test]
    fn a_commit_matches_only_its_own_nonce() {
        let n1 = fresh_nonce();
        let n2 = fresh_nonce();
        assert_eq!(commit_for(&n1), commit_for(&n1));
        assert_ne!(commit_for(&n1), commit_for(&n2));
        assert_eq!(commit_for(&n1).len(), 64, "hex of sha256 is 64 chars");
    }

    #[test]
    fn hex_round_trips() {
        let bytes = [0x00u8, 0x01, 0xab, 0xff, 0x10];
        let s = hex_of(&bytes);
        assert_eq!(s, "0001abff10");
        assert_eq!(decode_hex(&s).expect("valid hex"), bytes.to_vec());
    }

    #[test]
    fn decode_hex_rejects_malformed_input() {
        assert_eq!(decode_hex("abc"), None, "odd length must be rejected");
        assert_eq!(decode_hex("zz"), None, "non-hex chars must be rejected");
    }
}

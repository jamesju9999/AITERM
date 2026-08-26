//! 共享連線的傳輸安全。
//!
//! 兩件事：
//! 1. **加密**——每次分享產生一組臨時自簽憑證，連線走 TLS 1.3。
//! 2. **防冒充**——自簽憑證本身擋不住中間人（觀看端沒有任何先驗資訊可判斷
//!    該信任哪張憑證），所以身分保證來自 **SAS 人工核對**：雙方各自從 TLS
//!    連線導出（RFC 5705）同一份金鑰 material，算成 4 位數顯示在畫面上，由
//!    使用者口頭核對。中間人必須維持兩條獨立的 TLS 連線，導出的 material 必
//!    然不同，數字就對不起來。
//!
//! 為什麼不用 PAKE（SPAKE2）：那樣可以省掉人工核對，但 `spake2` crate 目前
//! 只有 pre-release 版（0.5.0-pre.0），不適合放在安全關鍵路徑上。rustls 已
//! 經在依賴樹裡。見設計文件的「安全契約」。

use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// RFC 5705 匯出用的標籤。**連線雙方必須完全一致**——不一致會算出不同的
/// SAS，使用者會看到「對不上」而誤以為遭到攻擊。版本號在字串裡，未來要改
/// 演算法時連同標籤一起換。
pub const SAS_EXPORTER_LABEL: &[u8] = b"aiterm share sas v1";

/// 從匯出的金鑰 material 取幾個位元組來算 SAS。
const SAS_MATERIAL_LEN: usize = 32;

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

/// 把 TLS 匯出的金鑰 material 折成 4 位數字。
///
/// 4 位數（1/10000）不是用來抵擋離線暴力破解的——它只需要讓「即時的中間人
/// 攻擊」在人工核對這一關被抓到，而攻擊者沒有重試的機會：核對失敗使用者就
/// 不會按同意。
pub fn sas_from_exporter(material: &[u8]) -> String {
    // 折疊整段 material，讓任何一個位元組的差異都會影響結果。
    let mut acc: u32 = 0;
    for (i, b) in material.iter().enumerate() {
        acc = acc
            .wrapping_mul(31)
            .wrapping_add((*b as u32).wrapping_add(i as u32));
    }
    format!("{:04}", acc % 10_000)
}

/// 從一條已完成握手的 TLS 連線導出這一端的 SAS。
///
/// `conn` 是 `rustls::ServerConnection` 或 `rustls::ClientConnection`——兩者
/// 都 deref 到 `ConnectionCommon`，`export_keying_material` 就在上面
/// （rustls 0.23 `src/conn.rs:460`）。握手完成前呼叫會失敗，所以呼叫端必須
/// 在握手之後才叫。
pub fn sas_for_connection(
    export: impl FnOnce([u8; SAS_MATERIAL_LEN], &[u8], Option<&[u8]>) -> Result<[u8; SAS_MATERIAL_LEN], rustls::Error>,
) -> anyhow::Result<String> {
    let material = export([0u8; SAS_MATERIAL_LEN], SAS_EXPORTER_LABEL, None)
        .map_err(|e| anyhow::anyhow!("export keying material: {e}"))?;
    Ok(sas_from_exporter(&material))
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
    fn the_same_exporter_bytes_give_the_same_four_digit_code() {
        let material = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let a = sas_from_exporter(&material);
        let b = sas_from_exporter(&material);
        assert_eq!(a, b);
        assert_eq!(a.len(), 4, "SAS should be 4 chars, got {a:?}");
        assert!(a.chars().all(|c| c.is_ascii_digit()), "got {a:?}");
    }

    #[test]
    fn different_exporter_bytes_give_a_different_code() {
        // 這是整個防中間人設計的支點：中間人的兩條 TLS 連線導出不同的
        // material，所以兩邊畫面上的數字對不起來。
        let a = sas_from_exporter(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        let b = sas_from_exporter(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x89]);
        assert_ne!(a, b);
    }

    #[test]
    fn the_sas_label_is_pinned() {
        // 標籤是連線雙方必須一致的常數。改動它會讓新舊版本算出不同的 SAS，
        // 使用者會看到「對不上」而以為被攻擊——所以固定住並讓改動很明顯。
        assert_eq!(SAS_EXPORTER_LABEL, b"aiterm share sas v1");
    }
}

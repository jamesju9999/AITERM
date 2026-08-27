//! mDNS 區網自動發現：主控端廣播短碼，觀看端瀏覽比對。
//!
//! 設計文件：`docs/superpowers/specs/2026-08-27-remote-terminal-mdns-discovery-design.md`

/// 廣播/瀏覽用的服務類型。
pub const SERVICE_TYPE: &str = "_aiterm._tcp.local.";

/// 查詢一個能拿去註冊 mDNS 服務的本機 IPv4。
///
/// `mdns_sd::ServiceInfo::new` 要求呼叫端提供明確的 IP，不支援「自動偵測」
/// 的空值/占位符。這個 repo 既有的 `commands::share::lan_address()` 是給
/// 面板顯示用的，在 Windows 上刻意回 `None`（查不到就讓使用者自己輸入）
/// ——那個退化路徑在這裡不能用，mDNS 註冊需要保證拿得到的值。
///
/// 用 UDP「connect」到一個公開位址的技巧：`connect()` 只是讓作業系統依照
/// 路由表決定要用哪張網卡送出封包、把來源位址記在 socket 上，**不會真的
/// 送出任何封包**，所以離線也能用，且是三個平台共通的標準函式庫寫法，
/// 不需要另外拉一個列舉網卡的 crate。
///
/// # 已知限制
///
/// 如果 VPN 佔據了預設路由，本函式會回傳 VPN 的虛擬 IP 而非實際的區網介面——
/// 這會導致 mDNS 註冊無聲地失敗（服務會廣播一個區網上的任何機器都連不上的位址）。
/// 修正這個問題的成本高（需要列舉每個實體網卡的位址），超出本任務的範圍；
/// 這個限制需要被記住以防日後帶來意外。
pub fn local_ipv4_for_mdns() -> Option<std::net::Ipv4Addr> {
    use std::net::{IpAddr, UdpSocket};

    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            log::warn!("local_ipv4_for_mdns: UDP socket bind failed: {}", e);
            return None;
        }
    };

    if let Err(e) = socket.connect("8.8.8.8:80") {
        log::warn!("local_ipv4_for_mdns: UDP socket connect failed: {}", e);
        return None;
    }

    let local_addr = match socket.local_addr() {
        Ok(addr) => addr,
        Err(e) => {
            log::warn!("local_ipv4_for_mdns: failed to get local address: {}", e);
            return None;
        }
    };

    match local_addr.ip() {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(_) => {
            log::warn!("local_ipv4_for_mdns: resolved address was IPv6-only, no IPv4 available");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_some_local_ipv4_on_a_normal_machine() {
        // 不需要真的連得上網路——UDP connect 只是問路由表，離線也該查得到
        // 一個位址（至少是 loopback 以外的介面）。這條測試不碰 multicast，
        // 不用標 ignore。
        let ip = local_ipv4_for_mdns();
        assert!(ip.is_some(), "expected to find a local IPv4 address");
        assert!(!ip.unwrap().is_loopback(), "expected non-loopback IPv4 address");
    }
}

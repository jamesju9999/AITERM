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

use std::collections::HashMap;

use mdns_sd::{ServiceDaemon, ServiceInfo};

/// 主控端這一側：持有 mDNS daemon，記著「哪個分頁註冊成哪個 mDNS 服務全名」
/// 好在停止分享時精準取消註冊。
///
/// 跟 `ShareRegistry` 刻意分開（`ShareRegistry` 的文件講得很清楚：它不碰
/// 網路）——這個結構才是真的碰 mDNS 網路呼叫的地方。
///
/// **同步合約**：此結構**不自行同步**，與 `ShareRegistry`/`ViewerManager` 不同。
/// 呼叫端必須從外部序列化存取（例如在一個 Mutex 後面）。一個後續任務將把
/// `MdnsAdvertiser` 放進 `ShareServerState` 既有的 `Mutex<Option<Running>>`，
/// 那個外層鎖就會提供同步；內層額外的 Mutex 會造成冗餘的雙重鎖定。
pub struct MdnsAdvertiser {
    daemon: ServiceDaemon,
    /// tab_id → 這個分頁註冊出去的 mDNS 服務全名（`ServiceInfo::get_fullname()`）。
    registrations: HashMap<String, String>,
}

impl MdnsAdvertiser {
    /// 啟動一個 mDNS daemon（背景執行緒）。失敗就回錯誤——呼叫端把它當成
    /// 「這台機器上 mDNS 不能用」，不能讓分享功能本身也跟著失敗。
    pub fn start() -> anyhow::Result<Self> {
        let daemon = ServiceDaemon::new()?;
        Ok(Self { daemon, registrations: HashMap::new() })
    }

    /// 幫一個分頁的短碼註冊一筆 mDNS 服務。
    ///
    /// **冪等**：這個分頁已經註冊過就直接回，不重新註冊。`share_start` 本身
    /// 對「重複分享同一分頁」也是冪等的（回傳既有短碼），呼叫端因此可以每次
    /// 都無腦呼叫這支，不用自己判斷「這次是不是真的新短碼」。
    ///
    /// 服務名稱刻意用隨機值而不是短碼本身——見 `mdns.rs` 模組文件開頭的
    /// 背景說明：短碼撞名時 `ServiceDaemon` 會自動改名，觀看端永遠看不到
    /// 「兩個回應」，所以短碼只能放 TXT，讓觀看端自己判斷有幾筆比對上。
    pub fn register(&mut self, tab_id: &str, code: &str, port: u16) {
        if self.registrations.contains_key(tab_id) {
            return;
        }
        let Some(ip) = local_ipv4_for_mdns() else {
            log::warn!("mDNS 註冊失敗：查不到本機區網位址，短碼 {code} 不會被自動發現");
            return;
        };
        let host_name = format!("{ip}.local.");
        let instance_name = uuid::Uuid::new_v4().to_string();
        let mut props = HashMap::new();
        props.insert("code".to_string(), code.to_string());

        let info = match ServiceInfo::new(SERVICE_TYPE, &instance_name, &host_name, std::net::IpAddr::V4(ip), port, props)
        {
            Ok(i) => i,
            Err(e) => {
                log::warn!("mDNS ServiceInfo 建立失敗：{e}");
                return;
            }
        };
        let fullname = info.get_fullname().to_string();
        match self.daemon.register(info) {
            Ok(()) => {
                self.registrations.insert(tab_id.to_string(), fullname);
            }
            Err(e) => log::warn!("mDNS 註冊失敗：{e}"),
        }
    }

    /// 取消一個分頁的 mDNS 註冊。沒註冊過就是安全的空操作。
    pub fn unregister(&mut self, tab_id: &str) {
        if let Some(fullname) = self.registrations.remove(tab_id) {
            if let Err(e) = self.daemon.unregister(&fullname) {
                log::warn!("mDNS 取消註冊失敗：tab_id={}, fullname={}, error={}", tab_id, fullname, e);
            }
        }
    }

    /// 關掉整個 daemon。跟這個專案既有的 WS server 關閉方式一樣走
    /// fire-and-forget——沒有人需要等它真的關完才能繼續。
    pub fn shutdown(&self) {
        let _ = self.daemon.shutdown();
    }
}

#[cfg(test)]
mod advertiser_tests {
    use super::*;

    // 這兩個測試需要真的開 multicast socket，部分 CI 沙箱環境不允許（這個
    // repo 的 `cargo test` 已經有 3 個因為同類原因被標 ignore 的既有測試）。
    // 本機執行：`cargo test --lib share::mdns -- --ignored`

    #[test]
    #[ignore = "需要真的 multicast socket，CI 沙箱可能不允許"]
    fn registering_twice_for_the_same_tab_is_a_no_op() {
        let mut adv = MdnsAdvertiser::start().expect("daemon starts");
        adv.register("tab-1", "123456", 4000);
        let first = adv.registrations.get("tab-1").cloned();
        adv.register("tab-1", "123456", 4000);
        assert_eq!(adv.registrations.get("tab-1").cloned(), first, "re-registering minted a new entry");
        adv.shutdown();
    }

    #[test]
    #[ignore = "需要真的 multicast socket，CI 沙箱可能不允許"]
    fn unregistering_an_unknown_tab_does_not_panic() {
        let mut adv = MdnsAdvertiser::start().expect("daemon starts");
        adv.unregister("no-such-tab");
        adv.shutdown();
    }

    #[test]
    #[ignore = "需要真的 multicast socket，CI 沙箱可能不允許"]
    fn unregister_then_register_again_works() {
        let mut adv = MdnsAdvertiser::start().expect("daemon starts");
        adv.register("tab-1", "123456", 4000);
        assert!(adv.registrations.contains_key("tab-1"), "tab-1 should be registered");
        adv.unregister("tab-1");
        assert!(!adv.registrations.contains_key("tab-1"), "tab-1 should be unregistered");
        adv.register("tab-1", "654321", 4001);
        assert!(adv.registrations.contains_key("tab-1"), "tab-1 should be registered again");
        adv.shutdown();
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

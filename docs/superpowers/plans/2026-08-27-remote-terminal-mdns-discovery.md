# 遠端終端機共享 計畫②C：mDNS 區網自動發現 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓觀看端只打 6 位短碼就能連上同區網的主控端，不用再手動輸入 `host:port`。

**Architecture:** 主控端用 `mdns-sd`（純 Rust mDNS/DNS-SD 實作）廣播一筆服務，服務名稱是隨機值（避免撞名被協定悄悄改名），短碼放進 TXT 記錄。觀看端瀏覽同一個服務類型，用 TXT 裡的碼比對使用者輸入，在一個收集時間窗內累積結果，分類成「找到一個／零個／多個」三種結果，只有「找到一個」時才直接連線，其餘退回既有的手動位址輸入路徑。

**Tech Stack:** Rust（`mdns-sd` 0.21）、Tauri command、React/TypeScript（`ConnectDialog`）。

---

## 背景事實（寫這份計畫時查證過，不要重查一次）

- `mdns-sd` 的 `ServiceDaemon::register` 遇到同名衝突時會照 RFC 6762 自動幫其中一台改名（`DaemonEvent::NameChange`），不會讓兩台同時廣播成功——所以服務名稱**不能**是 6 位短碼本身，要用不會撞名的隨機值，短碼放 TXT。這是 `2026-08-27-remote-terminal-mdns-discovery-design.md` 已經定案的設計。
- `ServiceInfo::new` 要求呼叫端提供一個明確的本機 IP（不支援「自動偵測」的空值/占位符）。這個 repo 既有的 `lan_address()`（`src-tauri/src/commands/share.rs:40`）在 Windows 上永遠回 `None`，是設計成「查不到就讓面板退化成只顯示 port」的展示用途，不能借來用——mDNS 註冊需要的是一個保證拿得到、跨三平台都能動的 IP，所以這份計畫寫一個新的、專門給 mDNS 用的查詢函式，不碰 `lan_address()`。
- `ServiceDaemon` 實作 `Clone + Send + Sync`，內部就是一個指到背景執行緒的 channel handle，複製成本低，可以安心放進 Tauri 的 async command 裡用。
- `ServiceEvent` 是 `#[non_exhaustive]`，`match` 一定要有 wildcard 分支。
- `ResolvedService`（`ServiceEvent::ServiceResolved(Box<ResolvedService>)` 裡的型別）的欄位：`fullname: String`、`host: String`、`port: u16`、`addresses: HashSet<ScopedIp>`、`txt_properties: TxtProperties`。`TxtProperties::get_property_val_str(key: &str) -> Option<&str>`。`ScopedIp::to_ip_addr() -> IpAddr`。
- `daemon.unregister(fullname: &str)`、`daemon.stop_browse(ty_domain: &str)`、`daemon.shutdown()` 都回 `Result<...>`，這個專案既有的關閉模式是 fire-and-forget（`let _ = ...`），這裡延用同樣的風格。

## 檔案總覽

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src-tauri/Cargo.toml` | 修改 | 新增 `mdns-sd` 依賴 |
| `src-tauri/src/share/mdns.rs` | 新建 | 本機 IP 查詢、`MdnsAdvertiser`（註冊/取消註冊）、`discover()`（瀏覽＋分類） |
| `src-tauri/src/share/mod.rs` | 修改 | `Running` 多帶一個 `Option<MdnsAdvertiser>`；`start_if_needed_on_port`／`stop_if_idle` 跟著啟停；新增 `mdns_register`/`mdns_unregister` 方法 |
| `src-tauri/src/commands/share.rs` | 修改 | 新增 `DiscoverResult`（Serialize DTO）與 `share_discover` command；`share_start`/`share_stop` 各加一行呼叫 |
| `src-tauri/src/lib.rs` | 修改 | 註冊 `share_discover` |
| `src/ipc/share.ts` | 修改 | 新增 `DiscoverResult` 型別與 `shareDiscover()` |
| `src/lib/i18n.ts` | 修改 | 新增 `connect_searching`、`connect_ambiguous`（zh-TW + en） |
| `src/components/ConnectDialog/index.tsx` | 修改 | `submit()` 改成先查 mDNS，依結果分流 |
| `src/components/ConnectDialog/index.test.tsx` | 修改 | 新增/調整測試涵蓋三種 `DiscoverResult` |

---

### Task 1: `mdns-sd` 依賴與本機 IP 查詢

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/src/share/mdns.rs`
- Modify: `src-tauri/src/share/mod.rs`（加一行 `pub mod mdns;`）

- [ ] **Step 1: 加依賴**

在 `src-tauri/Cargo.toml` 的 `[dependencies]` 區塊，`rand = "0.9"` 那行附近加：

```toml
# mDNS 區網自動發現（計畫②C）。純 Rust 實作，macOS/Linux/Windows 都不需要
# 額外安裝 Bonjour 或 Avahi。
mdns-sd = "0.21"
```

- [ ] **Step 2: 建立 `mdns.rs`，寫本機 IP 查詢的失敗測試**

```rust
// src-tauri/src/share/mdns.rs
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
pub fn local_ipv4_for_mdns() -> Option<std::net::Ipv4Addr> {
    use std::net::{IpAddr, UdpSocket};
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(_) => None,
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
    }
}
```

在 `src-tauri/src/share/mod.rs` 檔案開頭的 `pub mod` 列表裡加一行：

```rust
pub mod mdns;
```

- [ ] **Step 3: 執行測試確認過**

Run: `cd src-tauri && cargo test --lib share::mdns -- --nocapture`
Expected: `finds_some_local_ipv4_on_a_normal_machine ... ok`

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/share/mdns.rs src-tauri/src/share/mod.rs
git commit -m "feat(share): add mdns-sd dependency and a cross-platform local-IP lookup"
```

---

### Task 2: `MdnsAdvertiser`（註冊／取消註冊，冪等）

**Files:**
- Modify: `src-tauri/src/share/mdns.rs`

- [ ] **Step 1: 寫結構與方法**

在 `mdns.rs` 的 `local_ipv4_for_mdns` 函式後面加：

```rust
use std::collections::HashMap;

use mdns_sd::{ServiceDaemon, ServiceInfo};

/// 主控端這一側：持有 mDNS daemon，記著「哪個分頁註冊成哪個 mDNS 服務全名」
/// 好在停止分享時精準取消註冊。
///
/// 跟 `ShareRegistry` 刻意分開（`ShareRegistry` 的文件講得很清楚：它不碰
/// 網路）——這個結構才是真的碰 mDNS 網路呼叫的地方。
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

        let info = match ServiceInfo::new(SERVICE_TYPE, &instance_name, &host_name, ip, port, props)
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
            let _ = self.daemon.unregister(&fullname);
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
}
```

- [ ] **Step 2: 本機手動確認（會用到真的 multicast，不是自動測試）**

Run: `cd src-tauri && cargo test --lib share::mdns -- --ignored --nocapture`
Expected: 兩個測試都 `ok`（在允許 multicast 的機器上）

- [ ] **Step 3: 一般測試套件仍然全過**

Run: `cd src-tauri && cargo test --lib share::mdns`
Expected: `finds_some_local_ipv4_on_a_normal_machine ... ok`（其餘 2 個 ignored）

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/share/mdns.rs
git commit -m "feat(share): add MdnsAdvertiser for idempotent per-tab mDNS registration"
```

---

### Task 3: `discover()`——瀏覽並分類成一個／零個／多個

**Files:**
- Modify: `src-tauri/src/share/mdns.rs`

- [ ] **Step 1: 寫三個情境的失敗測試**

在 `mdns.rs` 加：

```rust
use std::collections::HashSet;
use std::net::IpAddr;
use std::time::Duration;

use mdns_sd::ServiceEvent;

/// `discover` 的分類結果。跟 `commands::share::DiscoverResult` 刻意分開
/// （這支不依賴 Tauri/serde，能在不起 app 的情況下測試——比照
/// `commands::share::decide`/`Decision` 的既有分工）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverOutcome {
    Found { host: String, port: u16 },
    NotFound,
    Ambiguous,
}

fn pick_ipv4(addresses: &HashSet<mdns_sd::ScopedIp>) -> Option<std::net::Ipv4Addr> {
    addresses.iter().find_map(|a| match a.to_ip_addr() {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(_) => None,
    })
}

/// 瀏覽 `SERVICE_TYPE`，在 `window` 時間內收集所有 TXT `code` 欄位等於
/// `code` 的回應，依收集到幾個不同的 `(host, port)` 分類。
///
/// 用一個獨立的、只在這次呼叫存活的 daemon（不是主控端廣播用的那個）——
/// 觀看端可能根本沒有在分享任何東西，`ShareServerState.running` 這時是
/// `None`，沒有既有 daemon 可以借用；反過來，一台機器同時分享與查詢別人
/// 也該是兩件互不相干的事。
pub async fn discover(code: &str, window: Duration) -> DiscoverOutcome {
    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("mDNS daemon 啟動失敗：{e}");
            return DiscoverOutcome::NotFound;
        }
    };
    let Ok(receiver) = daemon.browse(SERVICE_TYPE) else {
        let _ = daemon.shutdown();
        return DiscoverOutcome::NotFound;
    };

    let mut matches: HashSet<(String, u16)> = HashSet::new();
    let deadline = tokio::time::Instant::now() + window;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, receiver.recv_async()).await {
            Ok(Ok(ServiceEvent::ServiceResolved(resolved))) => {
                if resolved.txt_properties.get_property_val_str("code") == Some(code) {
                    if let Some(addr) = pick_ipv4(&resolved.addresses) {
                        matches.insert((addr.to_string(), resolved.port));
                    }
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => break, // channel 關了
            Err(_) => break,     // 這次 recv 逾時，外層迴圈會發現 deadline 到了
        }
    }

    let _ = daemon.stop_browse(SERVICE_TYPE);
    let _ = daemon.shutdown();

    let mut it = matches.into_iter();
    match (it.next(), it.next()) {
        (None, _) => DiscoverOutcome::NotFound,
        (Some((host, port)), None) => DiscoverOutcome::Found { host, port },
        (Some(_), Some(_)) => DiscoverOutcome::Ambiguous,
    }
}

#[cfg(test)]
mod discover_tests {
    use super::*;

    // 三個情境都需要真的 multicast——見 Task 2 同樣的 ignore 理由。
    const TEST_WINDOW: Duration = Duration::from_millis(800);

    #[tokio::test]
    #[ignore = "需要真的 multicast socket，CI 沙箱可能不允許"]
    async fn finds_nothing_when_nobody_is_advertising() {
        let outcome = discover("999999", TEST_WINDOW).await;
        assert_eq!(outcome, DiscoverOutcome::NotFound);
    }

    #[tokio::test]
    #[ignore = "需要真的 multicast socket，CI 沙箱可能不允許"]
    async fn finds_the_one_host_advertising_that_code() {
        let mut adv = MdnsAdvertiser::start().expect("daemon starts");
        adv.register("tab-1", "111222", 5000);

        let outcome = discover("111222", TEST_WINDOW).await;
        match outcome {
            DiscoverOutcome::Found { port, .. } => assert_eq!(port, 5000),
            other => panic!("expected Found, got {other:?}"),
        }
        adv.shutdown();
    }

    #[tokio::test]
    #[ignore = "需要真的 multicast socket，CI 沙箱可能不允許"]
    async fn is_ambiguous_when_two_hosts_advertise_the_same_code() {
        // 同一個 daemon 註冊兩筆不同 port、相同 code——模擬兩台不同機器剛好
        // 選到同一組短碼的情境（觀看端看到的只是「兩個不同的 host:port
        // 都回報這個 code」，不管它們是不是同一台機器發的）。
        let mut adv = MdnsAdvertiser::start().expect("daemon starts");
        adv.register("tab-1", "333444", 5001);
        adv.register("tab-2", "333444", 5002);

        let outcome = discover("333444", TEST_WINDOW).await;
        assert_eq!(outcome, DiscoverOutcome::Ambiguous);
        adv.shutdown();
    }
}
```

- [ ] **Step 2: 本機手動確認**

Run: `cd src-tauri && cargo test --lib share::mdns -- --ignored --nocapture`
Expected: 全部（含 Task 2 的兩個）都 `ok`

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/share/mdns.rs
git commit -m "feat(share): add mDNS discover() with found/not-found/ambiguous classification"
```

---

### Task 4: 接上 `ShareServerState` 的生命週期

**Files:**
- Modify: `src-tauri/src/share/mod.rs`

- [ ] **Step 1: 修改 `Running`，把 mDNS daemon 一起裝進去**

`src-tauri/src/share/mod.rs` 目前是：

```rust
struct Running {
    port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
}
```

改成：

```rust
struct Running {
    port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
    /// `None` 代表這台機器上 mDNS daemon 啟動失敗（例如環境不允許
    /// multicast）——分享功能本身**不能**因為這樣就失敗，只是不會被自動
    /// 發現，使用者退回手動位址一樣能連。
    mdns: Option<mdns::MdnsAdvertiser>,
}
```

- [ ] **Step 2: `start_if_needed_on_port` 啟動 daemon**

找到：

```rust
        let app_router = server::router(pty, Arc::clone(&self.registry), app);
        let identity = tls::ShareIdentity::generate()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        // 自己的 accept 迴圈而不是 axum::serve——見下方「TLS 的接線」。
        tokio::spawn(serve_tls(listener, app_router, identity, rx));
        *self.running.lock() = Some(Running { port, shutdown: tx });
        Ok(port)
```

改成：

```rust
        let app_router = server::router(pty, Arc::clone(&self.registry), app);
        let identity = tls::ShareIdentity::generate()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        // 自己的 accept 迴圈而不是 axum::serve——見下方「TLS 的接線」。
        tokio::spawn(serve_tls(listener, app_router, identity, rx));
        let mdns = match mdns::MdnsAdvertiser::start() {
            Ok(a) => Some(a),
            Err(e) => {
                log::warn!("mDNS daemon 啟動失敗，這次分享不會被自動發現：{e}");
                None
            }
        };
        *self.running.lock() = Some(Running { port, shutdown: tx, mdns });
        Ok(port)
```

- [ ] **Step 3: `stop_if_idle` 關掉 daemon**

找到：

```rust
    pub fn stop_if_idle(&self) {
        if self.registry.any_active() {
            return;
        }
        if let Some(r) = self.running.lock().take() {
            let _ = r.shutdown.send(());
        }
    }
```

改成：

```rust
    pub fn stop_if_idle(&self) {
        if self.registry.any_active() {
            return;
        }
        if let Some(r) = self.running.lock().take() {
            if let Some(mdns) = &r.mdns {
                mdns.shutdown();
            }
            let _ = r.shutdown.send(());
        }
    }
```

- [ ] **Step 4: 新增 `mdns_register`/`mdns_unregister` 方法**

在 `impl ShareServerState` 裡，`stop_if_idle` 後面加：

```rust
    /// 幫某個分頁的短碼註冊 mDNS 廣播。server 還沒啟動（`running` 是
    /// `None`）或這次啟動時 mDNS daemon 沒能起來時，安靜地什麼都不做——
    /// 呼叫端（`commands::share::share_start`）不需要關心這兩種情況。
    pub fn mdns_register(&self, tab_id: &str, code: &str) {
        let mut running = self.running.lock();
        let Some(r) = running.as_mut() else { return };
        let port = r.port;
        if let Some(mdns) = r.mdns.as_mut() {
            mdns.register(tab_id, code, port);
        }
    }

    /// 取消某個分頁的 mDNS 廣播。
    pub fn mdns_unregister(&self, tab_id: &str) {
        let mut running = self.running.lock();
        let Some(r) = running.as_mut() else { return };
        if let Some(mdns) = r.mdns.as_mut() {
            mdns.unregister(tab_id);
        }
    }
```

- [ ] **Step 5: 編譯確認**

Run: `cd src-tauri && cargo check`
Expected: 編譯成功（這個 task 沒有新增自動測試——`Running`/`ShareServerState` 的既有測試都在 `commands/share.rs`/`share_commands.rs`/`share_viewer.rs` 那幾個檔案裡，不直接碰 `Running` 的內部欄位，這裡只是接線，行為由 Task 5 的 command 層與既有整合測試覆蓋）

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/share/mod.rs
git commit -m "feat(share): wire MdnsAdvertiser lifecycle into ShareServerState"
```

---

### Task 5: `share_discover` command

**Files:**
- Modify: `src-tauri/src/commands/share.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/share_commands.rs`

- [ ] **Step 1: 寫一個不用真的跑 mDNS 就能測的轉換測試**

`share_discover` 本身要跑真的網路查詢（跟 Task 3 一樣需要 multicast），但 `DiscoverResult` 跟 `mdns::DiscoverOutcome` 之間的轉換是純資料轉換，不用碰網路，直接測轉換邏輯。在 `src-tauri/tests/share_commands.rs` 加：

```rust
use aiterm_lib::commands::share::DiscoverResult;
use aiterm_lib::share::mdns::DiscoverOutcome;

#[test]
fn discover_result_mirrors_the_outcome_kind() {
    assert!(matches!(
        DiscoverResult::from(DiscoverOutcome::Found { host: "1.2.3.4".into(), port: 9 }),
        DiscoverResult::Found { host, port } if host == "1.2.3.4" && port == 9
    ));
    assert!(matches!(DiscoverResult::from(DiscoverOutcome::NotFound), DiscoverResult::NotFound));
    assert!(matches!(DiscoverResult::from(DiscoverOutcome::Ambiguous), DiscoverResult::Ambiguous));
}
```

- [ ] **Step 2: 執行確認失敗**

Run: `cd src-tauri && cargo test --test share_commands discover_result_mirrors_the_outcome_kind`
Expected: FAIL（`DiscoverResult` 不存在，或沒有 `From<DiscoverOutcome>`）

- [ ] **Step 3: 接上 `share_start`/`share_stop`——沒有這步，Task 4 加的方法永遠不會被呼叫**

`src-tauri/src/commands/share.rs` 的 `share_start`：

```rust
#[tauri::command]
pub async fn share_start(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
    pty_manager: State<'_, Arc<PtyManager>>,
    app: AppHandle,
) -> Result<ShareStatus, String> {
    let port = server
        .start_if_needed(pty_manager.inner().clone(), Some(app))
        .await
        .map_err(|e| format!("啟動共享服務失敗：{e}"))?;
    let code = server.registry.start_share(tab_id);
    Ok(ShareStatus { sharing: true, code: Some(code), port: Some(port), lan_address: lan_address() })
}
```

改成（`tab_id` 要留到呼叫 `mdns_register` 時用，`start_share` 那行原本會把它 move 掉，所以先 clone 一份）：

```rust
#[tauri::command]
pub async fn share_start(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
    pty_manager: State<'_, Arc<PtyManager>>,
    app: AppHandle,
) -> Result<ShareStatus, String> {
    let port = server
        .start_if_needed(pty_manager.inner().clone(), Some(app))
        .await
        .map_err(|e| format!("啟動共享服務失敗：{e}"))?;
    let code = server.registry.start_share(tab_id.clone());
    // 冪等：`MdnsAdvertiser::register` 自己會擋掉重複註冊，這裡不用判斷
    // 「這次是不是真的新短碼」。
    server.mdns_register(&tab_id, &code);
    Ok(ShareStatus { sharing: true, code: Some(code), port: Some(port), lan_address: lan_address() })
}
```

`share_stop`：

```rust
#[tauri::command]
pub async fn share_stop(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<ShareStatus, String> {
    server.registry.stop_share(&tab_id);
    server.stop_if_idle();
    Ok(ShareStatus { sharing: false, code: None, port: None, lan_address: None })
}
```

改成（取消 mDNS 註冊要在 `stop_if_idle` 之前——一旦 `stop_if_idle` 把整個 `Running`／daemon 關掉，`mdns_unregister` 就找不到東西可以取消了）：

```rust
#[tauri::command]
pub async fn share_stop(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<ShareStatus, String> {
    server.mdns_unregister(&tab_id);
    server.registry.stop_share(&tab_id);
    server.stop_if_idle();
    Ok(ShareStatus { sharing: false, code: None, port: None, lan_address: None })
}
```

- [ ] **Step 4: 在 `commands/share.rs` 加 DTO、`From` 轉換與 command**

在檔案最後（`share_revoke_control` 後面）加：

```rust
/// `share_discover` 的結果，給 `ConnectDialog` 分流用。
///
/// 跟 `mdns::DiscoverOutcome` 分開（這個帶 serde，那個不帶）——比照
/// `decide`/`Decision` 既有的分工：內部邏輯不依賴 Tauri，command 層只是
/// 薄薄一層轉接。
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DiscoverResult {
    Found { host: String, port: u16 },
    NotFound,
    Ambiguous,
}

impl From<crate::share::mdns::DiscoverOutcome> for DiscoverResult {
    fn from(o: crate::share::mdns::DiscoverOutcome) -> Self {
        match o {
            crate::share::mdns::DiscoverOutcome::Found { host, port } => {
                DiscoverResult::Found { host, port }
            }
            crate::share::mdns::DiscoverOutcome::NotFound => DiscoverResult::NotFound,
            crate::share::mdns::DiscoverOutcome::Ambiguous => DiscoverResult::Ambiguous,
        }
    }
}

/// 用短碼在區網上找主控端。收集窗固定 1.5 秒——短到使用者不會覺得卡住，
/// 長到足夠讓同網段的回應趕上（見設計文件的「觀看端查找流程」）。
#[tauri::command]
pub async fn share_discover(code: String) -> Result<DiscoverResult, String> {
    let outcome = crate::share::mdns::discover(&code, std::time::Duration::from_millis(1500)).await;
    Ok(outcome.into())
}
```

- [ ] **Step 5: 執行確認 Step 1 的測試過**

Run: `cd src-tauri && cargo test --test share_commands discover_result_mirrors_the_outcome_kind`
Expected: PASS

- [ ] **Step 6: 在 `lib.rs` 註冊新 command**

`src-tauri/src/lib.rs` 裡的 `use` 列表（約第 99-101 行）：

```rust
    share::{
        share_approve, share_deny, share_kick, share_pending, share_revoke_control, share_start,
        share_status, share_stop, share_viewers,
    },
```

改成：

```rust
    share::{
        share_approve, share_deny, share_discover, share_kick, share_pending,
        share_revoke_control, share_start, share_status, share_stop, share_viewers,
    },
```

`invoke_handler!` 列表（約第 386-395 行）的 `share_viewers,` 後面加一行：

```rust
            share_viewers,
            share_discover,
            share_kick,
```

- [ ] **Step 7: 完整編譯與既有測試套件確認**

Run: `cd src-tauri && cargo test --lib && cargo test --test share_commands`
Expected: 全部 PASS（新增的 1 個 PASS，既有的都不受影響）

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/commands/share.rs src-tauri/src/lib.rs src-tauri/tests/share_commands.rs
git commit -m "feat(share): add share_discover Tauri command"
```

---

### Task 6: 前端 IPC 封裝

**Files:**
- Modify: `src/ipc/share.ts`

- [ ] **Step 1: 加型別與函式**

在 `src/ipc/share.ts` 的 `Decision` 型別後面（`shareStart` 之前）加：

```typescript
export type DiscoverResult =
  | { kind: "found"; host: string; port: number }
  | { kind: "notFound" }
  | { kind: "ambiguous" };
```

在檔案最後加：

```typescript
/** 用短碼在區網上找主控端。找到一個就回 `Found`；找不到或找到多個時前端
 *  要退回手動位址輸入（分別對應「查無」與「有歧義」兩種不同文案）。 */
export function shareDiscover(code: string): Promise<DiscoverResult> {
  return invoke<DiscoverResult>("share_discover", { code });
}
```

- [ ] **Step 2: 型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤（這個 task 只加型別跟一個函式，還沒有任何呼叫端，`tsc` 應該乾淨過）

- [ ] **Step 3: Commit**

```bash
git add src/ipc/share.ts
git commit -m "feat(share): add shareDiscover IPC wrapper"
```

---

### Task 7: i18n 新增文案

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 加兩把新 key**

`connect_not_found` 與 `connect_manual_prompt` 這兩把 key 在 zh-TW（第 843-844 行）跟 en（第 2194-2195 行）已經存在但目前沒被用到——這個 task 沿用它們，只新增「搜尋中」跟「找到多個」這兩把還沒有的。

zh-TW 區塊（第 846 行 `connect_failed` 後面）加：

```typescript
    connect_searching: "搜尋中…",
    connect_ambiguous: "這個網路上有不只一台機器在用這組編號，請改用手動位址確認是哪一台。",
```

en 區塊（第 2197 行 `connect_failed` 後面）加：

```typescript
    connect_searching: "Searching…",
    connect_ambiguous: "More than one machine on this network is using that code. Use the manual address to pick the right one.",
```

- [ ] **Step 2: 型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤（`Translations` 型別是從 zh-TW 物件自動推導的，兩個區塊都加了同樣的 key，不會有型別缺漏）

- [ ] **Step 3: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(i18n): add strings for mDNS searching/ambiguous states"
```

---

### Task 8: `ConnectDialog` 改用 mDNS 優先查找

**Files:**
- Modify: `src/components/ConnectDialog/index.tsx`
- Test: `src/components/ConnectDialog/index.test.tsx`

- [ ] **Step 1: 寫新流程的失敗測試**

在 `src/components/ConnectDialog/index.test.tsx`，先在檔案頂端的 mock 區塊加 `shareDiscover` 的 mock：

```typescript
const discoverMock = vi.fn();
vi.mock("../../ipc/share", () => ({
  shareDiscover: (...a: unknown[]) => discoverMock(...a),
}));
```

在 `beforeEach` 裡加一行預設值：

```typescript
  discoverMock.mockReset().mockResolvedValue({ kind: "notFound" });
```

（完整 `beforeEach` 因此變成：)

```typescript
beforeEach(() => {
  connectMock.mockReset().mockResolvedValue({ connId: "conn-1", sas: "4917" });
  discoverMock.mockReset().mockResolvedValue({ kind: "notFound" });
  onConnected.mockReset();
  onCancel.mockReset();
});
```

在 `describe("ConnectDialog", ...)` 區塊裡加三個新測試（既有測試不動，但注意：既有的「connects with a manually entered host and port」等測試會先展開手動欄位再送出，那條路徑本來就不該呼叫 `shareDiscover`，Step 3 的實作要保證這點）：

```typescript
  it("connects straight through when mDNS finds exactly one match", async () => {
    discoverMock.mockResolvedValue({ kind: "found", host: "192.168.1.50", port: 9000 });
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(discoverMock).toHaveBeenCalledWith("632706");
    expect(connectMock).toHaveBeenCalledWith("192.168.1.50", 9000, "632706", "AITerm");
    expect(onConnected).toHaveBeenCalledWith("conn-1", "4917", "192.168.1.50:9000");
  });

  it("falls back to the manual address field when mDNS finds nothing", async () => {
    discoverMock.mockResolvedValue({ kind: "notFound" });
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/找不到這組編號/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/192\.168/)).toBeInTheDocument();
    expect(connectMock).not.toHaveBeenCalled();
  });

  it("shows a distinct message when mDNS finds more than one match", async () => {
    discoverMock.mockResolvedValue({ kind: "ambiguous" });
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/不只一台機器/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/192\.168/)).toBeInTheDocument();
    expect(connectMock).not.toHaveBeenCalled();
  });

  it("skips mDNS entirely once the manual address field has something in it", async () => {
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "192.168.1.33:47823");
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(discoverMock).not.toHaveBeenCalled();
    expect(connectMock).toHaveBeenCalledWith("192.168.1.33", 47823, "632706", "AITerm");
  });
```

- [ ] **Step 2: 執行確認新測試失敗**

Run: `npx vitest run src/components/ConnectDialog`
Expected: 四個新測試 FAIL（`shareDiscover` 還沒被 `submit()` 呼叫；既有 5 個測試繼續 PASS）

- [ ] **Step 3: 改寫 `submit()`**

把 `src/components/ConnectDialog/index.tsx` 的 import 區塊：

```typescript
import { useState } from "react";
import { shareViewerConnect } from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";
```

改成：

```typescript
import { useState } from "react";
import { shareDiscover } from "../../ipc/share";
import { shareViewerConnect } from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";
```

把 `submit()` 函式：

```typescript
  async function submit() {
    setError(null);
    const parsed = parseAddress(address);
    if (!parsed) {
      setError(t.connect_bad_address);
      return;
    }
    setBusy(true);
    try {
      const { connId, sas } = await shareViewerConnect(
        parsed.host,
        parsed.port,
        code,
        name || "AITerm",
      );
      onConnected(connId, sas, address);
    } catch (e) {
      // 連不上要說原因，不要靜默關閉——使用者才知道下一步該做什麼。
      setError(t.connect_failed.replace("{error}", String(e)));
    } finally {
      setBusy(false);
    }
  }
```

改成：

```typescript
  async function connectTo(host: string, port: number, addressLabel: string) {
    try {
      const { connId, sas } = await shareViewerConnect(host, port, code, name || "AITerm");
      onConnected(connId, sas, addressLabel);
    } catch (e) {
      // 連不上要說原因，不要靜默關閉——使用者才知道下一步該做什麼。
      setError(t.connect_failed.replace("{error}", String(e)));
    }
  }

  async function submit() {
    setError(null);

    // 手動位址欄位已經展開且有填：永遠優先，完全不跑 mDNS 查找。使用者
    // 已經知道要連哪裡，不該被搜尋卡住或蓋掉他輸入的內容。
    if (manualOpen && address.trim()) {
      const parsed = parseAddress(address);
      if (!parsed) {
        setError(t.connect_bad_address);
        return;
      }
      setBusy(true);
      await connectTo(parsed.host, parsed.port, address);
      setBusy(false);
      return;
    }

    setBusy(true);
    setSearching(true);
    try {
      const result = await shareDiscover(code);
      if (result.kind === "found") {
        const label = `${result.host}:${result.port}`;
        await connectTo(result.host, result.port, label);
        return;
      }
      setManualOpen(true);
      setError(result.kind === "ambiguous" ? t.connect_ambiguous : t.connect_not_found);
    } finally {
      setSearching(false);
      setBusy(false);
    }
  }
```

加一個新的 state（跟其他 `useState` 放一起）：

```typescript
  const [searching, setSearching] = useState(false);
```

在送出按鈕那段（`{error && ...}` 前面）加一個搜尋中的提示：

```typescript
        {searching && <div className="aiterm-connect__searching">{t.connect_searching}</div>}

        {error && <div className="aiterm-connect__error">{error}</div>}
```

- [ ] **Step 4: 執行確認全部測試過**

Run: `npx vitest run src/components/ConnectDialog`
Expected: 全部 PASS（既有 5 個 ＋ 新增 4 個 = 9 個）

- [ ] **Step 5: 型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤

- [ ] **Step 6: 順手把過時的 doc comment 改掉**

`ConnectDialog` 檔案開頭的 doc comment 提到「這個階段 mDNS 還沒上線」，現在已經上線了，改成：

```typescript
/**
 * 觀看端的連線入口。
 *
 * **手動位址永遠是主路徑**（見 spec 的決策紀錄）：mDNS 在公司網路／跨
 * VLAN／訪客 Wi-Fi 常常失效，所以手動那條路必須一直走得通——`submit()`
 * 一旦偵測到手動欄位有內容就完全跳過 mDNS，直接用它連。
 *
 * 平常把手動欄位收起來，只有 mDNS 查無結果或結果有歧義（多台機器用了
 * 同一組短碼）時才自動展開，並依情境顯示不同文案。
 */
```

- [ ] **Step 7: Commit**

```bash
git add src/components/ConnectDialog/index.tsx src/components/ConnectDialog/index.test.tsx
git commit -m "feat(share): try mDNS discovery before falling back to manual address"
```

---

### Task 9: 全套驗證

**Files:** 無新增/修改——這個 task 只跑驗證。

- [ ] **Step 1: 前端全套**

Run: `npx tsc -b && npx vitest run`
Expected: `tsc` 無錯誤；vitest 全部 PASS（含 Task 8 新增的 4 個）

- [ ] **Step 2: Rust 全套（不含 ignored）**

Run: `cd src-tauri && cargo test`
Expected: 全部 PASS，ignored 數量比這份計畫開始前多 5 個（Task 2 的 2 個 + Task 3 的 3 個）

- [ ] **Step 3: ESLint 沒有新增問題**

Run: `npm run lint 2>&1 | tail -5`
Expected: 錯誤/警告總數跟這份計畫開始前一致（不能比之前多）

- [ ] **Step 4: 本機手動驗證 mDNS 的三個情境（自動化測不到的部分）**

在允許 multicast 的機器上：

```bash
cd src-tauri && cargo test --lib share::mdns -- --ignored --nocapture
```

Expected: 5 個 ignored 測試全部 PASS。

再用兩台實機（或同一台機器的兩個視窗）驗證：主控端分享、觀看端只打短碼不碰手動欄位，確認能直接連上；把其中一台的 Wi-Fi 關掉多播（若環境允許測試）確認會展開手動欄位並顯示「找不到這組編號」。

- [ ] **Step 5: Commit（若前面步驟有任何修正）**

若 Step 1-3 過程中有修正任何檔案：

```bash
git add -A
git commit -m "fix: address issues found during final verification pass"
```

若都一次過，這個 task 不需要 commit。

---

## Spec 覆蓋檢查

| Spec 章節 | 對應 Task |
|---|---|
| 套件選擇（`mdns-sd`） | Task 1 |
| 廣播內容與生命週期（隨機服務名稱＋TXT 放短碼、`start_share`/`stop_share` 跟著走、daemon 跟 WS server 一起啟停） | Task 2、Task 4 |
| 觀看端查找流程（`share_discover` command、三種結果分流、手動優先） | Task 3、Task 5、Task 8 |
| 錯誤處理（找到一台／查無／找到多台／multicast 被擋時的行為） | Task 8（找到多台的獨立文案）、Task 3（分類邏輯） |
| 測試（Rust 三情境、前端三種 `DiscoverResult` 分支、手動驗證） | Task 2、Task 3（Rust）、Task 8（前端）、Task 9（手動） |
| 已知限制（CI 沙箱 multicast、mDNS 失效退回手動、防火牆沿用計畫②） | Task 2/3 的 `#[ignore]` 標記；不需要額外程式碼，已經是既有行為 |

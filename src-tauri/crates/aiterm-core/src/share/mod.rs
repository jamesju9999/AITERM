//! 遠端終端機共享：協定、短碼註冊表、TLS 身分與 SAS、mDNS 廣播。
//!
//! server 端（`server`）與觀看端（`viewer`）之後也會搬進來；把事件推播給 GUI
//! 的那一層留在 `app` crate 的 `share::viewer_manager`。

pub mod events;
pub mod mdns;
pub mod protocol;
pub mod registry;
pub mod tls;

/// rustls 0.23 要求行程層級的預設加密供應者。裝一次就好；重複呼叫會回
/// `Err`，直接忽略——那代表別人已經裝過了，不是錯誤。
pub fn ensure_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

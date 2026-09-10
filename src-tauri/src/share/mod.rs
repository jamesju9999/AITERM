//! GUI 專屬的共享接線。協定與 server 住在 `aiterm-core`。

pub mod tauri_events;
pub mod viewer_manager;

pub use aiterm_core::share::{
    ensure_crypto_provider, events, mdns, protocol, registry, server, tls, viewer,
    ShareServerState,
};

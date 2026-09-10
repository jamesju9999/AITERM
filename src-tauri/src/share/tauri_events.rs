//! `ShareEvents` 的 GUI 實作：把事件轉成 Tauri 事件送給前端。

use aiterm_core::share::events::ShareEvents;
use aiterm_core::share::protocol::PendingRequestEvent;
use tauri::{AppHandle, Emitter};

pub struct TauriShareEvents {
    pub app: AppHandle,
}

impl ShareEvents for TauriShareEvents {
    fn pending_request(&self, ev: &PendingRequestEvent) {
        let _ = self.app.emit("share://request-pending", ev.clone());
    }

    fn viewers_changed(&self) {
        let _ = self.app.emit("share://viewers-changed", ());
    }
}

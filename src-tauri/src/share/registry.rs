//! 共享狀態機：短碼、待審連線、觀看者名單、單一控制權。
//!
//! 刻意不碰網路、TLS 或 PTY——控制權的不變式（同時只有一人）值得被密集
//! 測試，起伺服器才能測會讓那些測試又慢又脆。

use std::collections::HashMap;

use parking_lot::Mutex;
use rand::Rng;
use uuid::Uuid;

/// 觀看者被授予的存取層級。由主控端在同意視窗上當場選擇，不走「先唯讀再
/// 請求控制」的兩段式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    ReadOnly,
    Control,
}

/// 一筆等待主控端裁決的連線請求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub request_id: String,
    pub tab_id: String,
    /// 請求方自報的名字。**未經驗證**，僅供主控端辨識用；真正的身分保證來自
    /// SAS 人工核對（見 `share::tls`）。
    pub display_name: String,
}

/// 一位已獲准的觀看者。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    pub viewer_id: String,
    pub display_name: String,
    pub mode: AccessMode,
    /// 產生這位觀看者的那筆 `request_id`。ws handler 手上只有 request_id
    /// （`approve` 的回傳值是給主控端 UI 的），靠這個欄位把兩者對起來。
    pub from_request: String,
}

struct SharedTab {
    code: String,
    viewers: Vec<Viewer>,
    /// 目前持有控制權的 `viewer_id`。一支麥克風：同時最多一人。
    controller: Option<String>,
}

#[derive(Default)]
pub struct ShareRegistry {
    /// tab_id → 該分頁的共享狀態。
    tabs: Mutex<HashMap<String, SharedTab>>,
    /// request_id → 待審請求。
    pending: Mutex<HashMap<String, PendingRequest>>,
}

impl ShareRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 開始分享一個分頁，回傳它的 6 位短碼。
    ///
    /// **冪等**：這個分頁若已在分享中，直接回傳既有短碼、什麼都不動。刻意
    /// 不產生新短碼——覆蓋既有 entry 會無聲斷線所有觀看者並釋放控制權，對
    /// 一個「雙擊分享鈕」就能觸發的操作來說，那是純粹的傷害。規格裡也沒有
    /// 「重新產生短碼」這個功能。
    pub fn start_share(&self, tab_id: String) -> String {
        let mut tabs = self.tabs.lock();
        if let Some(existing) = tabs.get(&tab_id) {
            return existing.code.clone();
        }
        let code = loop {
            let candidate = format!("{:06}", rand::rng().random_range(0..1_000_000u32));
            if !tabs.values().any(|t| t.code == candidate) {
                break candidate;
            }
        };
        tabs.insert(
            tab_id,
            SharedTab { code: code.clone(), viewers: Vec::new(), controller: None },
        );
        code
    }

    /// 停止分享：短碼作廢、觀看者全數移除、控制權釋放。
    pub fn stop_share(&self, tab_id: &str) {
        self.tabs.lock().remove(tab_id);
        self.pending.lock().retain(|_, p| p.tab_id != tab_id);
    }

    /// 是否還有任何分頁在分享中。共享 server 靠這個決定要不要關閉自己。
    pub fn any_active(&self) -> bool {
        !self.tabs.lock().is_empty()
    }

    /// 短碼對應到哪個分頁；短碼不存在或已作廢時回 `None`。
    pub fn tab_for_code(&self, code: &str) -> Option<String> {
        self.tabs
            .lock()
            .iter()
            .find(|(_, t)| t.code == code)
            .map(|(id, _)| id.clone())
    }

    /// 用短碼發起一筆待審請求。回傳 `request_id`，或在短碼無效時回 `None`。
    /// **這一步不會讓對方看到任何東西**——要等主控端 `approve`。
    pub fn request_join(&self, code: &str, display_name: String) -> Option<String> {
        let tab_id = self.tab_for_code(code)?;
        let request_id = Uuid::new_v4().to_string();
        self.pending.lock().insert(
            request_id.clone(),
            PendingRequest { request_id: request_id.clone(), tab_id, display_name },
        );
        Some(request_id)
    }

    /// 某分頁目前所有待審請求。
    pub fn pending(&self, tab_id: &str) -> Vec<PendingRequest> {
        self.pending.lock().values().filter(|p| p.tab_id == tab_id).cloned().collect()
    }

    /// 某分頁目前所有觀看者。
    pub fn viewers(&self, tab_id: &str) -> Vec<Viewer> {
        self.tabs.lock().get(tab_id).map(|t| t.viewers.clone()).unwrap_or_default()
    }

    /// 核准一筆待審請求，回傳新的 `viewer_id`。
    ///
    /// 以 `AccessMode::Control` 核准時，若控制權已被別人持有則**整筆核准失敗**
    /// 並回 `None`——不靜默降級成唯讀，那會讓主控端以為自己給出了控制權。
    ///
    /// 注意：若分享在裁決前就被停掉，這裡也會回 `None`，而該筆請求兩個 map
    /// 都不在。呼叫端若要區分「被拒絕」與「分享已停止」，必須自己先查
    /// `tab_for_code`——這個順序是 ws handler 依賴的，換別的呼叫端時不要
    /// 假設 `None` 一定代表拒絕。
    pub fn approve(&self, request_id: &str, mode: AccessMode) -> Option<String> {
        let request = self.pending.lock().remove(request_id)?;
        let mut tabs = self.tabs.lock();
        // `?` 在這裡的意思是「分享在裁決前就被停掉了」——見上方 doc comment
        // 對這條路徑的說明。
        let tab = tabs.get_mut(&request.tab_id)?;
        if mode == AccessMode::Control && tab.controller.is_some() {
            // 放回待審，讓主控端能改用唯讀重新裁決。
            drop(tabs);
            self.pending.lock().insert(request_id.to_string(), request);
            return None;
        }
        let viewer_id = Uuid::new_v4().to_string();
        tab.viewers.push(Viewer {
            viewer_id: viewer_id.clone(),
            display_name: request.display_name,
            mode,
            from_request: request_id.to_string(),
        });
        if mode == AccessMode::Control {
            tab.controller = Some(viewer_id.clone());
        }
        Some(viewer_id)
    }

    /// 找出某筆已核准請求所產生的觀看者 id。ws handler 在等待裁決的迴圈裡靠
    /// 這支判斷「我被核准了嗎」——請求離開待審名單後，查得到觀看者就是核准，
    /// 查不到就是拒絕。
    pub fn viewer_for_request(&self, tab_id: &str, request_id: &str) -> Option<String> {
        self.tabs
            .lock()
            .get(tab_id)?
            .viewers
            .iter()
            .find(|v| v.from_request == request_id)
            .map(|v| v.viewer_id.clone())
    }

    /// 拒絕一筆待審請求。
    pub fn deny(&self, request_id: &str) {
        self.pending.lock().remove(request_id);
    }

    /// 把控制權交給一位既有的唯讀觀看者。控制權已被持有時回 `false`——必須先
    /// `revoke_control`，避免「轉移」變成無聲地把前一個人踢下台。
    pub fn grant_control(&self, tab_id: &str, viewer_id: &str) -> bool {
        let mut tabs = self.tabs.lock();
        let Some(tab) = tabs.get_mut(tab_id) else { return false };
        if tab.controller.is_some() {
            return false;
        }
        let Some(v) = tab.viewers.iter_mut().find(|v| v.viewer_id == viewer_id) else {
            return false;
        };
        v.mode = AccessMode::Control;
        tab.controller = Some(viewer_id.to_string());
        true
    }

    /// 收回目前的控制權（若有）。持有者降為唯讀，仍留在觀看者名單裡。
    pub fn revoke_control(&self, tab_id: &str) {
        let mut tabs = self.tabs.lock();
        let Some(tab) = tabs.get_mut(tab_id) else { return };
        if let Some(current) = tab.controller.take() {
            if let Some(v) = tab.viewers.iter_mut().find(|v| v.viewer_id == current) {
                v.mode = AccessMode::ReadOnly;
            }
        }
    }

    /// 移除一位觀看者。若他正持有控制權，控制權一併釋放。
    pub fn remove_viewer(&self, tab_id: &str, viewer_id: &str) {
        let mut tabs = self.tabs.lock();
        let Some(tab) = tabs.get_mut(tab_id) else { return };
        tab.viewers.retain(|v| v.viewer_id != viewer_id);
        if tab.controller.as_deref() == Some(viewer_id) {
            tab.controller = None;
        }
    }

    /// 這位觀看者送來的按鍵可不可以寫進 PTY。
    ///
    /// 這是**伺服器端**的授權檢查，不是 UI 提示。唯讀端理應根本不送輸入，但
    /// 那是對方程式的自律，不能當成安全邊界。
    pub fn may_send_input(&self, tab_id: &str, viewer_id: &str) -> bool {
        self.tabs
            .lock()
            .get(tab_id)
            .and_then(|t| t.controller.clone())
            .as_deref()
            == Some(viewer_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_with_one_share() -> (ShareRegistry, String) {
        let reg = ShareRegistry::new();
        let code = reg.start_share("tab-1".to_string());
        (reg, code)
    }

    #[test]
    fn a_share_code_is_six_digits() {
        let (_reg, code) = registry_with_one_share();
        assert_eq!(code.len(), 6, "code should be 6 chars, got {code:?}");
        assert!(
            code.chars().all(|c| c.is_ascii_digit()),
            "code should be all digits, got {code:?}"
        );
    }

    #[test]
    fn two_shares_never_collide_on_a_code() {
        let reg = ShareRegistry::new();
        let a = reg.start_share("tab-1".to_string());
        let b = reg.start_share("tab-2".to_string());
        assert_ne!(a, b);
    }

    #[test]
    fn an_unknown_code_resolves_to_nothing() {
        let (reg, code) = registry_with_one_share();
        // 短碼是亂數，所以不能寫死一個「一定不存在」的值——從真正的短碼推
        // 一個保證不同的出來。
        let bogus: String = code
            .chars()
            .map(|c| if c == '0' { '1' } else { '0' })
            .collect();
        assert_ne!(bogus, code, "test bug: bogus code must differ from the real one");
        assert_eq!(reg.tab_for_code(&bogus), None);
    }

    #[test]
    fn stopping_a_share_invalidates_its_code() {
        let (reg, code) = registry_with_one_share();
        assert_eq!(reg.tab_for_code(&code), Some("tab-1".to_string()));
        reg.stop_share("tab-1");
        assert_eq!(reg.tab_for_code(&code), None);
    }

    #[test]
    fn a_pending_request_is_not_yet_a_viewer() {
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).expect("code is live");
        assert_eq!(reg.viewers("tab-1").len(), 0);
        assert_eq!(reg.pending("tab-1").len(), 1);
        assert_eq!(reg.pending("tab-1")[0].request_id, req);
    }

    #[test]
    fn joining_with_a_dead_code_is_refused() {
        let (reg, code) = registry_with_one_share();
        reg.stop_share("tab-1");
        assert!(reg.request_join(&code, "Alice".to_string()).is_none());
    }

    #[test]
    fn approving_read_only_adds_a_viewer_without_control() {
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).unwrap();
        let viewer = reg.approve(&req, AccessMode::ReadOnly).expect("approve");
        assert_eq!(reg.viewers("tab-1").len(), 1);
        assert_eq!(reg.pending("tab-1").len(), 0);
        assert!(!reg.may_send_input("tab-1", &viewer));
    }

    #[test]
    fn approving_with_control_lets_that_viewer_send_input() {
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).unwrap();
        let viewer = reg.approve(&req, AccessMode::Control).expect("approve");
        assert!(reg.may_send_input("tab-1", &viewer));
    }

    #[test]
    fn denying_a_request_leaves_no_viewer_and_no_pending() {
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).unwrap();
        reg.deny(&req);
        assert_eq!(reg.viewers("tab-1").len(), 0);
        assert_eq!(reg.pending("tab-1").len(), 0);
    }

    #[test]
    fn control_is_a_single_microphone() {
        let (reg, code) = registry_with_one_share();
        let r1 = reg.request_join(&code, "Alice".to_string()).unwrap();
        let alice = reg.approve(&r1, AccessMode::Control).unwrap();
        let r2 = reg.request_join(&code, "Bob".to_string()).unwrap();

        // Bob cannot be approved with control while Alice holds it.
        assert_eq!(reg.approve(&r2, AccessMode::Control), None);

        // Read-only is fine, and he still cannot type.
        let bob = reg.approve(&r2, AccessMode::ReadOnly).expect("read-only approve");
        assert!(reg.may_send_input("tab-1", &alice));
        assert!(!reg.may_send_input("tab-1", &bob));
    }

    #[test]
    fn control_must_be_revoked_before_it_can_be_granted_to_someone_else() {
        let (reg, code) = registry_with_one_share();
        let r1 = reg.request_join(&code, "Alice".to_string()).unwrap();
        let alice = reg.approve(&r1, AccessMode::Control).unwrap();
        let r2 = reg.request_join(&code, "Bob".to_string()).unwrap();
        let bob = reg.approve(&r2, AccessMode::ReadOnly).unwrap();

        assert!(!reg.grant_control("tab-1", &bob), "must refuse while Alice holds it");
        reg.revoke_control("tab-1");
        assert!(!reg.may_send_input("tab-1", &alice));
        assert!(reg.grant_control("tab-1", &bob));
        assert!(reg.may_send_input("tab-1", &bob));
    }

    #[test]
    fn removing_the_controller_releases_control() {
        let (reg, code) = registry_with_one_share();
        let r1 = reg.request_join(&code, "Alice".to_string()).unwrap();
        let alice = reg.approve(&r1, AccessMode::Control).unwrap();
        reg.remove_viewer("tab-1", &alice);
        assert_eq!(reg.viewers("tab-1").len(), 0);

        // Control is now free for the next viewer.
        let r2 = reg.request_join(&code, "Bob".to_string()).unwrap();
        let bob = reg.approve(&r2, AccessMode::Control).expect("control should be free");
        assert!(reg.may_send_input("tab-1", &bob));
    }

    #[test]
    fn stopping_a_share_drops_every_viewer() {
        let (reg, code) = registry_with_one_share();
        let r1 = reg.request_join(&code, "Alice".to_string()).unwrap();
        let alice = reg.approve(&r1, AccessMode::Control).unwrap();
        reg.stop_share("tab-1");
        assert_eq!(reg.viewers("tab-1").len(), 0);
        assert!(!reg.may_send_input("tab-1", &alice));
    }

    #[test]
    fn input_from_an_unknown_viewer_is_refused() {
        let (reg, _code) = registry_with_one_share();
        assert!(!reg.may_send_input("tab-1", "no-such-viewer"));
    }

    #[test]
    fn an_approved_request_can_be_mapped_back_to_its_viewer() {
        // ws handler 手上只有 request_id；approve 的回傳值是給主控端 UI 的。
        // 沒有這條對應，連線就不知道自己變成了哪一位觀看者。
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).unwrap();
        let viewer = reg.approve(&req, AccessMode::ReadOnly).unwrap();
        assert_eq!(reg.viewer_for_request("tab-1", &req), Some(viewer));
    }

    #[test]
    fn a_denied_request_maps_to_no_viewer() {
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).unwrap();
        reg.deny(&req);
        assert_eq!(reg.viewer_for_request("tab-1", &req), None);
    }

    #[test]
    fn a_share_is_live_only_while_it_has_a_code() {
        let reg = ShareRegistry::new();
        assert!(!reg.any_active());
        let _ = reg.start_share("tab-1".to_string());
        assert!(reg.any_active());
        reg.stop_share("tab-1");
        assert!(!reg.any_active());
    }

    #[test]
    fn stopping_a_share_clears_its_pending_requests() {
        let (reg, code) = registry_with_one_share();
        let _req = reg.request_join(&code, "Alice".to_string()).unwrap();
        assert_eq!(reg.pending("tab-1").len(), 1);
        reg.stop_share("tab-1");
        assert_eq!(reg.pending("tab-1").len(), 0);
    }

    #[test]
    fn a_pending_request_cannot_survive_into_a_later_share_of_the_same_tab() {
        // tab_id 是穩定的前端分頁 ID，所以「停止分享 → 重新分享」是正常操作，
        // 不是邊角案例。若 stop_share 沒清掉 pending，舊會話的待審請求會被
        // 核准進新會話，繞過「短碼失效即安全」這個假設。
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Eve".to_string()).unwrap();
        reg.stop_share("tab-1");
        let _new_code = reg.start_share("tab-1".to_string());

        assert_eq!(
            reg.pending("tab-1").len(),
            0,
            "a stopped share's pending request came back after re-sharing"
        );
        assert_eq!(
            reg.approve(&req, AccessMode::Control),
            None,
            "a stopped share's request was approved into the new share"
        );
    }

    #[test]
    fn sharing_an_already_shared_tab_keeps_the_same_code_and_its_viewers() {
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).unwrap();
        let alice = reg.approve(&req, AccessMode::Control).unwrap();

        let again = reg.start_share("tab-1".to_string());
        assert_eq!(again, code, "a second start_share minted a new code");
        assert_eq!(
            reg.viewers("tab-1").len(),
            1,
            "a second start_share evicted the existing viewers"
        );
        assert!(
            reg.may_send_input("tab-1", &alice),
            "a second start_share silently released control"
        );
    }
}

//! `commands/share.rs` 的核心不變式測試。
//!
//! 這裡不起 Tauri app——`#[tauri::command]` 的函式吃 `State<'_, T>`，測試裡
//! 拿不到。所以這些測試直接驗**被 command 呼叫的那層邏輯**（`decide`），
//! command 本身只是很薄的轉接。端到端由 `share_end_to_end.rs` 涵蓋。

use aiterm_lib::commands::share::{decide, Decision, DiscoverResult};
use aiterm_lib::share::mdns::DiscoverOutcome;
use aiterm_lib::share::registry::{AccessMode, ShareRegistry};

#[test]
fn approving_with_the_wrong_code_denies_the_request_outright() {
    // 攻擊者只有 1/10000 的一發機會。給重試等於送他一萬次——所以輸錯不是
    // 「再試一次」，是直接拒絕。
    let reg = ShareRegistry::new();
    let code = reg.start_share("tab-1".to_string());
    let req = reg
        .request_join(&code, "Alice".to_string(), "4917".to_string())
        .unwrap();

    let outcome = decide(&reg, &req, AccessMode::Control, "1234");

    assert!(matches!(outcome, Decision::CodeMismatch));
    assert_eq!(
        reg.pending("tab-1").len(),
        0,
        "a mismatched code must drop the request, not leave it retryable"
    );
    assert_eq!(reg.viewers("tab-1").len(), 0);
}

#[test]
fn approving_with_the_right_code_admits_the_viewer() {
    let reg = ShareRegistry::new();
    let code = reg.start_share("tab-1".to_string());
    let req = reg
        .request_join(&code, "Alice".to_string(), "4917".to_string())
        .unwrap();

    let outcome = decide(&reg, &req, AccessMode::Control, "4917");

    match outcome {
        Decision::Approved { viewer_id } => {
            assert!(reg.may_send_input("tab-1", &viewer_id));
        }
        other => panic!("expected Approved, got {other:?}"),
    }
    assert_eq!(reg.pending("tab-1").len(), 0);
}

#[test]
fn control_already_taken_is_reported_and_the_request_survives() {
    // 控制權被占用時整筆核准失敗，但請求要留著——主控端可以改用唯讀重新
    // 裁決。這是 registry 既有的語意，command 層不能把它吃掉。
    let reg = ShareRegistry::new();
    let code = reg.start_share("tab-1".to_string());
    let r1 = reg.request_join(&code, "Alice".to_string(), "1111".to_string()).unwrap();
    reg.approve(&r1, AccessMode::Control).unwrap();

    let r2 = reg.request_join(&code, "Bob".to_string(), "2222".to_string()).unwrap();
    let outcome = decide(&reg, &r2, AccessMode::Control, "2222");

    assert!(matches!(outcome, Decision::ControlTaken));
    assert_eq!(
        reg.pending("tab-1").len(),
        1,
        "the request must survive so the host can re-decide as read-only"
    );
}

#[test]
fn a_request_that_vanished_is_reported_not_panicked() {
    let reg = ShareRegistry::new();
    let code = reg.start_share("tab-1".to_string());
    let req = reg.request_join(&code, "Alice".to_string(), "4917".to_string()).unwrap();
    reg.deny(&req);

    let outcome = decide(&reg, &req, AccessMode::ReadOnly, "4917");
    assert!(matches!(outcome, Decision::RequestGone));
}

#[test]
fn discover_result_mirrors_the_outcome_kind() {
    assert!(matches!(
        DiscoverResult::from(DiscoverOutcome::Found { host: "1.2.3.4".into(), port: 9 }),
        DiscoverResult::Found { host, port } if host == "1.2.3.4" && port == 9
    ));
    assert!(matches!(DiscoverResult::from(DiscoverOutcome::NotFound), DiscoverResult::NotFound));
    assert!(matches!(DiscoverResult::from(DiscoverOutcome::Ambiguous), DiscoverResult::Ambiguous));
}

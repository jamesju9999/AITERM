//! 真的執行 `aiterm-intake-server` 二進位：init → project add → pair → run → Client 配對 → 撤銷。
use aiterm_intake_client::api::Api;
use aiterm_intake_client::pair::pair;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_aiterm-intake-server");

fn run(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(BIN).arg("--data-dir").arg(dir).args(args).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn code_from(output: &str) -> String {
    output.lines().find_map(|l| l.strip_prefix("邀請碼：")).expect("輸出裡要有邀請碼").trim().to_string()
}

#[test]
fn init_creates_everything_and_is_safe_to_repeat() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, out) = run(dir.path(), &["init", "--san", "10.0.0.5"]);
    assert!(ok, "{out}");
    for f in ["intake.db", "server.db", "secrets.key", "tls/cert.pem", "tls/key.pem"] {
        assert!(dir.path().join(f).exists(), "缺少 {f}");
    }
    let key = std::fs::read(dir.path().join("secrets.key")).unwrap();
    let cert = std::fs::read(dir.path().join("tls/cert.pem")).unwrap();
    let (ok, out2) = run(dir.path(), &["init"]);
    assert!(ok, "{out2}");
    assert_eq!(std::fs::read(dir.path().join("secrets.key")).unwrap(), key, "金鑰不可被覆蓋");
    assert_eq!(std::fs::read(dir.path().join("tls/cert.pem")).unwrap(), cert, "憑證不可被覆蓋");
    let fp = |s: &str| s.lines().find(|l| l.starts_with("憑證指紋")).map(str::to_string);
    assert_eq!(fp(&out), fp(&out2), "指紋穩定");
}

#[test]
fn pair_rejects_an_unknown_role_and_ai_refuses_an_unknown_provider() {
    let dir = tempfile::tempdir().unwrap();
    run(dir.path(), &["init"]);
    let (ok, out) = run(dir.path(), &["pair", "--name", "小明", "--role", "root"]);
    assert!(!ok && out.contains("member 或 admin"), "{out}");
    let child = Command::new(BIN).arg("--data-dir").arg(dir.path()).args(["ai", "--provider", "banana", "--model", "m"]).stdin(Stdio::null()).output().unwrap();
    assert!(!child.status.success());
}

#[tokio::test]
async fn a_real_server_process_pairs_a_client_and_revocation_takes_effect() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, out) = run(dir.path(), &["init"]);
    assert!(ok, "{out}");
    let (ok, out) = run(dir.path(), &["project", "add", "範例科技"]);
    assert!(ok, "{out}");

    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut server = Command::new(BIN)
        .arg("--data-dir")
        .arg(dir.path())
        .args(["run", "--bind", &format!("127.0.0.1:{port}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // 等它開始聽。
    let addr = format!("127.0.0.1:{port}");
    let mut up = false;
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(up, "server 沒有在時限內啟動");

    let (ok, out) = run(dir.path(), &["pair", "--name", "小明", "--role", "admin", "--address", &addr]);
    assert!(ok, "{out}");
    let paired = pair(&addr, &code_from(&out)).await.expect("用 CLI 產生的邀請碼配對");
    let api = Api::new(&addr, &paired.token, Some(paired.response.fingerprint.clone())).unwrap();
    let projects: Vec<serde_json::Value> = api.get("/v1/projects").await.unwrap();
    assert_eq!(projects[0]["name"], "範例科技", "CLI 建的專案，Client 看得到");

    let (ok, out) = run(dir.path(), &["members", "revoke", "小明"]);
    assert!(ok && out.contains("已撤銷 1"), "{out}");
    assert_eq!(api.request(reqwest::Method::GET, "/v1/me", None).await.unwrap_err(), aiterm_intake_client::ClientError::Revoked);

    let (_, status) = run(dir.path(), &["status"]);
    assert!(status.contains("成員：0 位有效") && status.contains("AI：尚未設定"), "{status}");
    let _ = server.kill();
    let _ = server.wait();
}

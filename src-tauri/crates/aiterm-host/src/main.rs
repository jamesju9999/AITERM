//! AITerm CLI Host：把這台機器的一個 shell 開放給桌面版 AITerm 連進來，
//! 讓觀看端用它自己的 AI 操作。
//!
//! 這個執行檔**不需要任何 AI 設定或 API key**——AI 跑在觀看端。

mod keyfile;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use portable_pty::PtySize;

use aiterm_core::pty::PtyManager;
use aiterm_core::share::events::SilentEvents;
use aiterm_core::share::registry::AccessMode;
use aiterm_core::share::server::HostAuth;
use aiterm_core::share::ShareServerState;

#[derive(Parser, Debug)]
#[command(
    name = "aiterm-host",
    version,
    about = "把一個 shell 開放給 AITerm 遠端連線與 AI 操作"
)]
struct Args {
    /// 監聽埠。刻意是固定的預設值而不是隨機——GUI 裡存的連線記著它。
    #[arg(long, default_value_t = 8022)]
    port: u16,

    /// 綁定位址。`127.0.0.1` 配 SSH tunnel 是最保守的用法。
    #[arg(long, default_value_t = Ipv4Addr::UNSPECIFIED)]
    bind: Ipv4Addr,

    /// 金鑰檔位置。預設 `<設定檔目錄>/aiterm-host/key`。
    #[arg(long)]
    key_file: Option<PathBuf>,

    /// 要跑的 shell。預設沿用 AITerm 既有的偵測。
    #[arg(long)]
    shell: Option<PathBuf>,

    /// shell 的起始工作目錄。
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// 連進來的人只能看，不能打字。
    #[arg(long)]
    read_only: bool,

    /// 開啟 mDNS 廣播。**預設關閉**，跟 GUI 相反：伺服器情境用不到自動發現，
    /// 而廣播等於在網路上宣告「這裡有一個 shell」。
    #[arg(long)]
    advertise: bool,

    /// 只印出連線資訊就退出，不開 shell。
    #[arg(long)]
    print_connection: bool,
}

impl Args {
    fn access_mode(&self) -> AccessMode {
        if self.read_only { AccessMode::ReadOnly } else { AccessMode::Control }
    }

    fn key_path(&self) -> Result<PathBuf> {
        match &self.key_file {
            Some(p) => Ok(p.clone()),
            None => keyfile::default_path(),
        }
    }
}

/// `--shell` 的實作：覆寫**這個行程自己**的 `SHELL`／`COMSPEC`。
///
/// **不能用 `ShellSpec.envs` 做這件事**：那些是給子行程的環境變數，而
/// `pty::shell::unix_default_shell()` 讀的是 `std::env::var("SHELL")`——
/// 呼叫端行程自己的環境，而且它在 `envs` 被套用**之前**就已經執行完了。
/// 用 envs 的話 `--shell` 會完全無效，且零錯誤訊息。
///
/// **必須在建立任何 PTY 之前呼叫。**
fn apply_shell_override(args: &Args) {
    let Some(p) = &args.shell else { return };
    let key = if cfg!(windows) { "COMSPEC" } else { "SHELL" };
    std::env::set_var(key, p);
}

/// 金鑰的來源。環境變數優先於檔案——容器情境常常只能給環境變數，而在那種
/// 情況下檔案往往是不存在或唯讀的。
fn resolve_key(args: &Args) -> Result<(Vec<u8>, String)> {
    if let Ok(hex) = std::env::var("AITERM_HOST_KEY") {
        let key = aiterm_core::share::tls::decode_hex(hex.trim())
            .context("AITERM_HOST_KEY 不是合法的 hex")?;
        if key.len() != aiterm_core::share::auth::KEY_LEN {
            anyhow::bail!(
                "AITERM_HOST_KEY 是 {} bytes，應該是 {}",
                key.len(),
                aiterm_core::share::auth::KEY_LEN
            );
        }
        if args.key_file.is_some() {
            eprintln!("警告：同時給了 --key-file 與 AITERM_HOST_KEY，採用環境變數。");
        }
        return Ok((key, "AITERM_HOST_KEY".to_string()));
    }
    let path = args.key_path()?;
    let (key, created) = keyfile::load_or_create(&path)?;
    if created {
        eprintln!("已產生新金鑰：{}", path.display());
    }
    Ok((key, path.display().to_string()))
}

fn print_connection(args: &Args, key_hex: &str, key_source: &str) {
    println!("AITerm CLI Host");
    println!("  位址：{}", args.bind);
    println!("  埠　：{}", args.port);
    println!("  金鑰：{key_hex}");
    println!("  來源：{key_source}");
    println!("  存取：{}", if args.read_only { "唯讀" } else { "可控制" });
    println!();
    println!("在 AITerm 的「連線到遠端終端機」裡填入上面的位址、埠與金鑰。");
    println!("跨網段時位址請填這台機器對觀看端可達的位址（Tailscale / VPN / SSH tunnel）。");
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();
    let (key, key_source) = resolve_key(&args)?;
    let key_hex = aiterm_core::share::tls::hex_of(&key);

    if args.print_connection {
        print_connection(&args, &key_hex, &key_source);
        return Ok(());
    }

    // 必須在建立任何 PTY 之前——`default_shell()` 讀的是這個行程的環境。
    apply_shell_override(&args);

    let pty = Arc::new(PtyManager::new());
    let session_id = "cli".to_string();
    let (cols, rows) = (120u16, 40u16);

    // 輸出丟掉：CLI host 自己不畫任何東西，觀看端是透過
    // `subscribe_with_history` 拿畫面的，那條路徑不經過這個 callback。
    pty.create_with_callback_and_id(
        PtySize { rows, cols, pixel_width: 0, pixel_height: 0 },
        session_id.clone(),
        args.cwd.clone(),
        Vec::new(),
        Vec::new(),
        |_chunk| {},
    )
    .context("開不出 shell")?;

    let server = ShareServerState::new();
    // registry 以短碼為索引，所以還是要走一次 start_share。它回傳的 6 位短碼
    // 在金鑰模式下不印、不用——身分完全由金鑰決定。
    let code = server.registry.start_share(session_id.clone());

    let auth = Arc::new(HostAuth {
        key: key.clone(),
        code: code.clone(),
        mode: args.access_mode(),
    });

    let port = server
        .start_if_needed_on_with_auth(
            pty.clone(),
            args.bind,
            args.port,
            Arc::new(SilentEvents),
            Some(auth),
        )
        .await
        .with_context(|| format!("綁不上 {}:{}", args.bind, args.port))?;

    if args.advertise {
        server.mdns_register(&session_id, &code);
    }

    print_connection(&args, &key_hex, &key_source);
    println!("監聽中：{}:{port}", args.bind);

    wait_for_shutdown(&pty, &session_id).await;
    Ok(())
}

/// 等到該收工為止：shell 自己結束，或收到終止訊號。
///
/// shell 結束的訊號取自 PTY 的 broadcast channel 被關閉——reader thread 在
/// PTY EOF 時結束、sender 被 drop，接收端就會拿到 `Closed`。這比輪詢輸出
/// 可靠：一個閒置的 shell 跟一個結束的 shell 一樣安靜。
async fn wait_for_shutdown(pty: &PtyManager, session_id: &str) {
    let mut rx = match pty.subscribe(session_id) {
        Some(rx) => rx,
        None => return,
    };

    let shell_ended = async {
        loop {
            match rx.recv().await {
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    tokio::select! {
        _ = shell_ended => {
            println!("shell 結束，收工。");
        }
        _ = terminate_signal() => {
            println!("收到終止訊號，關閉連線並收工。");
        }
    }

    // 收掉 PTY。觀看端會因為 `subscribe_with_history` 的 channel 關閉而收到
    // `SessionClosed`，這是既有行為，不需要另外送訊息。
    let _ = pty.close(session_id);
}

#[cfg(unix)]
async fn terminate_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("SIGINT handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

#[cfg(windows)]
async fn terminate_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn the_default_port_is_fixed_not_random() {
        // GUI 裡存的連線記著一個 port。隨機 port（GUI 主控端的作法）會讓那筆
        // 連線在每次重啟後失效。
        let args = Args::parse_from(["aiterm-host"]);
        assert_eq!(args.port, 8022);
    }

    #[test]
    fn mdns_is_off_by_default() {
        // 跟 GUI 相反，且是刻意的：伺服器情境用不到自動發現，而廣播等於在
        // 辦公室網路上宣告「這裡有一個 shell」。
        let args = Args::parse_from(["aiterm-host"]);
        assert!(!args.advertise);
    }

    #[test]
    fn control_is_the_default_access_level() {
        let args = Args::parse_from(["aiterm-host"]);
        assert!(!args.read_only);
        assert_eq!(args.access_mode(), AccessMode::Control);
    }

    #[test]
    fn read_only_flips_the_access_level() {
        let args = Args::parse_from(["aiterm-host", "--read-only"]);
        assert_eq!(args.access_mode(), AccessMode::ReadOnly);
    }

    #[test]
    fn the_default_bind_is_all_interfaces() {
        let args = Args::parse_from(["aiterm-host"]);
        assert_eq!(args.bind, std::net::Ipv4Addr::UNSPECIFIED);
    }

    #[test]
    fn bind_accepts_loopback() {
        let args = Args::parse_from(["aiterm-host", "--bind", "127.0.0.1"]);
        assert_eq!(args.bind, std::net::Ipv4Addr::LOCALHOST);
    }

    #[test]
    fn the_version_flag_reports_the_crate_version() {
        // 發版時 CI 會把 tag 的版本寫進這個 crate 的 Cargo.toml。使用者回報問題時
        // 第一件要問的就是「你跑的是哪一版」，所以這支一定要有，而且要跟 release
        // 的 tag 對得起來。
        let err = Args::try_parse_from(["aiterm-host", "--version"]).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains(env!("CARGO_PKG_VERSION")),
            "--version 要印出 crate 版本，got: {text}"
        );
    }
}

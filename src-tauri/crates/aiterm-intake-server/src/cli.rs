//! 命令列：`init`、`run`、`pair`、`members`、`project`、`ai`、`status`。
use crate::members::{self, Role};
use crate::secrets::FileSecrets;
use crate::state::AppState;
use aiterm_intake_core::host::Secrets;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "aiterm-intake-server", version, about = "AITerm 問題收集 Server：常駐收訊、共用佇列，供 AITerm Client 連線")]
pub struct Cli {
    /// 資料夾（資料庫、金鑰、憑證）。預設 `~/.aiterm-intake`。
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// 建立資料夾、金鑰、自簽憑證與資料庫（可重複執行，不會覆蓋既有的金鑰與憑證）。
    Init {
        /// 憑證涵蓋的主機名稱或 IP，可重複指定（內網模式 Client 靠指紋釘住，名稱不影響安全）。
        #[arg(long = "san")]
        sans: Vec<String>,
    },
    /// 啟動 server。
    Run {
        /// API 監聽位址。
        #[arg(long, default_value = "0.0.0.0:8443")]
        bind: SocketAddr,
        /// LINE webhook 綁定的位址；預設只收本機（前面接反向代理或 Tunnel）。
        #[arg(long, default_value = "127.0.0.1")]
        channel_bind: std::net::IpAddr,
        /// 公開網路用的正式憑證（PEM）；沒指定就用 `init` 產生的自簽憑證。
        #[arg(long, requires = "tls_key")]
        tls_cert: Option<PathBuf>,
        #[arg(long, requires = "tls_cert")]
        tls_key: Option<PathBuf>,
    },
    /// 產生一組一次性邀請碼（15 分鐘內有效、只能用一次）。
    Pair {
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "member")]
        role: String,
        /// 順便印出 Client 要輸入的位址。
        #[arg(long)]
        address: Option<String>,
    },
    /// 成員管理。
    Members {
        #[command(subcommand)]
        cmd: MembersCmd,
    },
    /// 專案管理。
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// 設定 AI 供應商（用於題目分類與群組「是不是新問題」）。金鑰從標準輸入讀取，不要放在參數裡。
    Ai {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
        #[arg(long, default_value = "")]
        base_url: String,
    },
    /// 顯示目前狀態。
    Status,
}

#[derive(Subcommand, Debug)]
pub enum MembersCmd {
    List,
    /// 以 id 或名稱撤銷；已連線的 Client 會在幾秒內被中斷。
    Revoke { id_or_name: String },
}

#[derive(Subcommand, Debug)]
pub enum ProjectCmd {
    Add {
        name: String,
        #[arg(long, default_value = "")]
        work_dir: String,
    },
    List,
}

pub fn default_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".aiterm-intake")
}

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let dir = cli.data_dir.unwrap_or_else(default_dir);
    match cli.cmd {
        Cmd::Init { sans } => init(&dir, sans).await,
        Cmd::Run { bind, channel_bind, tls_cert, tls_key } => serve(&dir, bind, channel_bind, tls_cert, tls_key).await,
        Cmd::Pair { name, role, address } => pair(&dir, &name, &role, address.as_deref()).await,
        Cmd::Members { cmd } => members_cmd(&dir, cmd).await,
        Cmd::Project { cmd } => project_cmd(&dir, cmd).await,
        Cmd::Ai { provider, model, base_url } => ai_set(&dir, provider, model, base_url).await,
        Cmd::Status => status(&dir).await,
    }
}

async fn init(dir: &std::path::Path, sans: Vec<String>) -> anyhow::Result<()> {
    let identity = crate::tls::load_or_generate(dir, sans)?;
    crate::db::open(dir).await?;
    FileSecrets::open(dir)?;
    println!("資料夾：{}", dir.display());
    println!("憑證指紋（SHA-256）：{}", identity.fingerprint());
    println!("下一步：aiterm-intake-server project add <專案名稱>，再用 pair 產生邀請碼，最後 run。");
    Ok(())
}

async fn serve(dir: &std::path::Path, bind: SocketAddr, channel_bind: std::net::IpAddr, cert: Option<PathBuf>, key: Option<PathBuf>) -> anyhow::Result<()> {
    let identity = match (cert, key) {
        (Some(c), Some(k)) => crate::tls::from_files(&c, &k)?,
        _ => crate::tls::load_or_generate(dir, vec![])?,
    };
    let pools = crate::db::open(dir).await?;
    let secrets = Arc::new(FileSecrets::open(dir)?);
    let state = AppState::with_channel_bind(pools, secrets, identity.fingerprint(), dir.to_path_buf(), channel_bind);
    let mut running = crate::server::start(bind, state, identity).await?;
    println!("aiterm-intake-server {} 已啟動：https://{}", env!("CARGO_PKG_VERSION"), running.addr);
    tokio::select! {
        _ = running.wait() => {}
        _ = shutdown_signal() => println!("收到結束訊號，正在關閉…"),
    }
    running.stop().await;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

async fn pair(dir: &std::path::Path, name: &str, role: &str, address: Option<&str>) -> anyhow::Result<()> {
    let role = Role::parse(role).ok_or_else(|| anyhow::anyhow!("角色必須是 member 或 admin"))?;
    let pools = crate::db::open(dir).await?;
    let inv = members::create_invite(&pools.server, name, role, crate::db::now()).await?;
    if let Some(a) = address {
        println!("位址：{a}");
    }
    println!("邀請碼：{}", inv.code);
    println!("有效 15 分鐘，只能使用一次。請在 AITerm 的問題收集設定 →「連線」輸入。");
    Ok(())
}

async fn members_cmd(dir: &std::path::Path, cmd: MembersCmd) -> anyhow::Result<()> {
    let pools = crate::db::open(dir).await?;
    match cmd {
        MembersCmd::List => {
            for m in members::list(&pools.server).await {
                println!("{}\t{}\t{}\t{}", m.id, m.name, m.role, if m.revoked_at.is_some() { "已撤銷" } else { "有效" });
            }
        }
        MembersCmd::Revoke { id_or_name } => {
            let n = members::revoke(&pools.server, &id_or_name, crate::db::now()).await?;
            crate::audit::record(&pools.server, "cli", "member.revoke", &id_or_name).await;
            println!("已撤銷 {n} 位成員");
        }
    }
    Ok(())
}

async fn project_cmd(dir: &std::path::Path, cmd: ProjectCmd) -> anyhow::Result<()> {
    let pools = crate::db::open(dir).await?;
    match cmd {
        ProjectCmd::Add { name, work_dir } => {
            let id = uuid::Uuid::new_v4().to_string();
            sqlx::query("INSERT INTO projects(id, name, work_dir, created_at) VALUES (?,?,?,?)").bind(&id).bind(name.trim()).bind(work_dir.trim()).bind(crate::db::now()).execute(&pools.server).await?;
            println!("{id}\t{name}");
        }
        ProjectCmd::List => {
            let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT id, name, work_dir FROM projects ORDER BY name").fetch_all(&pools.server).await?;
            for (id, name, dir) in rows {
                println!("{id}\t{name}\t{dir}");
            }
        }
    }
    Ok(())
}

async fn ai_set(dir: &std::path::Path, provider: String, model: String, base_url: String) -> anyhow::Result<()> {
    use std::io::Read;
    let pools = crate::db::open(dir).await?;
    crate::ai::save(&pools.intake, &crate::ai::AiSettings { provider: provider.clone(), model, base_url }).await.map_err(|e| anyhow::anyhow!(e))?;
    let mut key = String::new();
    std::io::stdin().read_to_string(&mut key)?;
    let key = key.trim();
    if !key.is_empty() {
        FileSecrets::open(dir)?.set(crate::ai::API_KEY_SECRET, key).map_err(|e| anyhow::anyhow!(e))?;
        println!("已設定 {provider} 與 API 金鑰");
    } else if provider == "ollama" {
        println!("已設定 {provider}（不需要金鑰）");
    } else {
        println!("已設定 {provider}，但標準輸入沒有金鑰；請用 `echo 金鑰 | aiterm-intake-server ai …` 補上");
    }
    Ok(())
}

async fn status(dir: &std::path::Path) -> anyhow::Result<()> {
    let pools = crate::db::open(dir).await?;
    let secrets = FileSecrets::open(dir)?;
    let ai = crate::ai::load(&pools.intake).await;
    println!("資料夾：{}", dir.display());
    println!("成員：{} 位有效", members::list(&pools.server).await.iter().filter(|m| m.revoked_at.is_none()).count());
    println!("LINE：Channel secret {}、access token {}", yes(secrets.get(aiterm_intake_core::line_server::LINE_SECRET_ID).is_some()), yes(secrets.get(aiterm_intake_core::line_server::LINE_TOKEN_ID).is_some()));
    println!("AI：{}", if crate::ai::configured(&ai, &secrets) { format!("{}（{}）", ai.provider, ai.model) } else { "尚未設定".into() });
    Ok(())
}

fn yes(b: bool) -> &'static str {
    if b {
        "已設定"
    } else {
        "未設定"
    }
}

use clap::Parser;

#[tokio::main]
async fn main() {
    env_logger::init();
    if let Err(e) = aiterm_intake_server::cli::run(aiterm_intake_server::cli::Cli::parse()).await {
        eprintln!("錯誤：{e}");
        std::process::exit(1);
    }
}

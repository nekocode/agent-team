use anyhow::Result;

fn main() -> Result<()> {
    // tracing 仅当 RUST_LOG 显式设置时启用（调试用）
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_env("RUST_LOG"))
            .init();
    }

    let cli = agent_team::cli::parse();
    agent_team::cli::run(cli)
}

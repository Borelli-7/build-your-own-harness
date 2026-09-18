mod app;
mod cli;
mod events;
mod llm;
mod state;
mod tools;
mod ui;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = cli::parse();
    app::run(cli).await
}

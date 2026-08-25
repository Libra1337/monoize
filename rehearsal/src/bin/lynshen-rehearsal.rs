#[tokio::main]
async fn main() -> anyhow::Result<()> {
    monoize_lynshen_rehearsal::cli::run(std::env::args_os()).await
}

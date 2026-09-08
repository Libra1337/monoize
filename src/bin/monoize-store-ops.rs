use monoize::store_billing::quota_gate_cli::{QuotaGateCliError, execute_from};

#[tokio::main]
async fn main() {
    let database_dsn = std::env::var("MONOIZE_DATABASE_DSN").ok();
    match execute_from(std::env::args_os(), database_dsn.as_deref()).await {
        Ok(output) => match serde_json::to_string(&output) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("error[output_serialize_failed]: {error}");
                std::process::exit(1);
            }
        },
        Err(QuotaGateCliError::Arguments(error)) => error.exit(),
        Err(error) => {
            eprintln!("error[{}]: {error}", error.code());
            std::process::exit(1);
        }
    }
}

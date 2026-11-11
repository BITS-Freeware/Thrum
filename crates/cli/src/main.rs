use clap::Parser;

/// Thrum command line client.
#[derive(Parser)]
#[command(name = "thrum", version, about)]
struct Cli {
    /// Base URL of the Thrum server.
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    server: String,
}

fn main() {
    let cli = Cli::parse();
    let base = match thrum_cli::normalize_base_url(&cli.server) {
        Ok(base) => base,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };
    println!("thrum client (server: {base})");
}

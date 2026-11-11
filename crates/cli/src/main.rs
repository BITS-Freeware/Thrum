use clap::Parser;

/// Geist3 command line client.
#[derive(Parser)]
#[command(name = "geist3", version, about)]
struct Cli {
    /// Base URL of the Geist3 server.
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    server: String,
}

fn main() {
    let cli = Cli::parse();
    let base = match geist3_cli::normalize_base_url(&cli.server) {
        Ok(base) => base,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };
    println!("geist3 client (server: {base})");
}

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use thrum_cli::api::Client;
use thrum_cli::session::{self, Session};
use thrum_cli::ws::{RecvOptions, recv};

/// Thrum command line client.
#[derive(Parser)]
#[command(name = "thrum", version, about)]
struct Cli {
    /// Base URL of the Thrum server.
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    server: String,
    /// Where to keep the logged-in session between commands.
    #[arg(long, default_value = "thrum-state.json")]
    state_file: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create an account.
    Register {
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: String,
    },
    /// Log in on this machine; each login is a new device.
    Login {
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: String,
        /// A name for this device, shown in the device list.
        #[arg(long, default_value = "cli")]
        label: String,
    },
    /// Create a conversation with the given members.
    Create {
        /// Comma-separated member usernames, e.g. alice,bob.
        #[arg(long = "members")]
        members: String,
    },
    /// Send a text message to a conversation.
    Send {
        #[arg(long)]
        conversation: String,
        #[arg(long)]
        text: String,
    },
    /// Receive messages live until quiet or one is delivered.
    Recv {
        /// Stop after the first message is delivered.
        #[arg(long)]
        once: bool,
        /// Give up after this many seconds without a message.
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// Show the stored session.
    Whoami,
}

fn load_session(state_file: &std::path::Path, base: &str) -> Result<Session, String> {
    session::load_for_server(state_file, base)
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let base = match thrum_cli::normalize_base_url(&cli.server) {
        Ok(base) => base,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = run(&cli, &base).await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run(cli: &Cli, base: &str) -> Result<(), String> {
    let client = Client::new(base.to_string());
    match &cli.command {
        Command::Register { username, password } => {
            client.register(username, password).await?;
            println!("registered {username}");
        }
        Command::Login {
            username,
            password,
            label,
        } => {
            let session = client.login(username, password, label).await?;
            session::save(&cli.state_file, &session)?;
            println!(
                "logged in {} (device {})",
                session.username, session.device_id
            );
        }
        Command::Create { members } => {
            let session = load_session(&cli.state_file, base)?;
            let list: Vec<String> = members
                .split(',')
                .map(str::trim)
                .filter(|member| !member.is_empty())
                .map(str::to_string)
                .collect();
            if list.is_empty() {
                return Err("no members given".to_string());
            }
            let id = client.create_conversation(&session.token, &list).await?;
            println!("conversation {id}");
        }
        Command::Send { conversation, text } => {
            let session = load_session(&cli.state_file, base)?;
            let id = client
                .send_message(&session.token, conversation, text.as_bytes())
                .await?;
            println!("sent {id}");
        }
        Command::Recv { once, timeout } => {
            let session = load_session(&cli.state_file, base)?;
            recv(
                &session,
                &RecvOptions {
                    once: *once,
                    timeout_secs: *timeout,
                },
                |line| println!("{line}"),
            )
            .await?;
        }
        Command::Whoami => {
            let session = load_session(&cli.state_file, base)?;
            println!(
                "{} on {} (device {})",
                session.username, session.server, session.device_id
            );
        }
    }
    Ok(())
}

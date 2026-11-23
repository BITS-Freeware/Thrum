use std::path::PathBuf;

use clap::{Parser, Subcommand};
use thrum_cli::api::Client;
use thrum_cli::chat::Chat;
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
    /// Create a conversation with the given members; you become its admin.
    Create {
        /// Comma-separated member usernames, e.g. alice,bob.
        #[arg(long = "members")]
        members: String,
        /// Optional group name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Rename a conversation; admin only.
    Rename {
        #[arg(long)]
        conversation: String,
        #[arg(long)]
        name: String,
    },
    /// Send a text message to a conversation.
    Send {
        #[arg(long)]
        conversation: String,
        #[arg(long)]
        text: String,
        /// Quote an earlier message by its id.
        #[arg(long = "reply-to")]
        reply_to: Option<String>,
    },
    /// Edit a message you sent.
    Edit {
        #[arg(long)]
        message: String,
        #[arg(long)]
        text: String,
    },
    /// Delete a message you sent.
    Delete {
        #[arg(long)]
        message: String,
    },
    /// Send a file as an encrypted attachment to a conversation.
    Attach {
        #[arg(long)]
        conversation: String,
        /// Path of the file to send.
        #[arg(long)]
        file: PathBuf,
    },
    /// Save the attachment of a received message to a file.
    Fetch {
        #[arg(long)]
        message: String,
        /// Where to save it; defaults to the attachment's name in this
        /// device's attachments directory.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Tell the group you have read everything so far.
    Read {
        #[arg(long)]
        conversation: String,
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
    /// Add members to a conversation; admin only.
    Add {
        #[arg(long)]
        conversation: String,
        /// Comma-separated member usernames to add.
        #[arg(long = "members")]
        members: String,
    },
    /// Remove a member from a conversation; admin only.
    Remove {
        #[arg(long)]
        conversation: String,
        #[arg(long)]
        member: String,
    },
    /// Mint an invite link for a conversation; admin only.
    Invite {
        #[arg(long)]
        conversation: String,
    },
    /// Revoke an invite link; admin only.
    Revoke {
        #[arg(long)]
        conversation: String,
        #[arg(long)]
        token: String,
    },
    /// Join a conversation with an invite link.
    Join {
        #[arg(long)]
        token: String,
    },
    /// Block a user: no new shared conversations with them.
    Block {
        #[arg(long)]
        user: String,
    },
    /// Unblock a user.
    Unblock {
        #[arg(long)]
        user: String,
    },
    /// Join trusted devices to all groups, drop revoked ones, and top up key packages.
    Sync,
    /// Allow a device identity into this account's conversations.
    ///
    /// The identity is `username#device-id`, as shown by a pending line from
    /// `sync`. The device is added on the sync that follows.
    Trust {
        #[arg(long)]
        device: String,
    },
    /// Revoke one of your devices. It can no longer sign in. Other members
    /// drop it from conversations the next time they sync.
    RevokeDevice {
        #[arg(long)]
        device: String,
    },
    /// Show the stored session.
    Whoami,
    /// Download a JSON export of everything the server knows about you.
    Export {
        /// Where to write the JSON document; defaults to `thrum-export.json`.
        #[arg(long, default_value = "thrum-export.json")]
        out: PathBuf,
    },
    /// Permanently delete the account on this server; requires --yes.
    DeleteAccount {
        #[arg(long)]
        password: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
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
            thrum_cli::errln!("error: {error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = run(&cli, &base).await {
        thrum_cli::errln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run(cli: &Cli, base: &str) -> Result<(), String> {
    let client = Client::new(base.to_string());
    match &cli.command {
        Command::Register { username, password } => {
            client.register(username, password).await?;
            thrum_cli::outln!("registered {username}");
        }
        Command::Login {
            username,
            password,
            label,
        } => {
            let session = client.login(username, password, label).await?;
            session::save(&cli.state_file, &session)?;
            let chat = Chat::open(client.clone(), session.clone(), &cli.state_file)?;
            chat.top_up_key_packages().await?;
            thrum_cli::outln!(
                "logged in {} (device {})",
                session.username,
                session.device_id
            );
        }
        Command::Create { members, name } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            let list = parse_member_list(members)?;
            let id = chat
                .create_conversation(&list, name.as_deref().unwrap_or(""))
                .await?;
            thrum_cli::outln!("conversation {id}");
        }
        Command::Rename { conversation, name } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            chat.rename(conversation, name).await?;
            thrum_cli::outln!("renamed {conversation}");
        }
        Command::Send {
            conversation,
            text,
            reply_to,
        } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            let id = match reply_to {
                Some(original) => chat.send_reply(conversation, original, text).await?,
                None => chat.send_text(conversation, text).await?,
            };
            thrum_cli::outln!("sent {id}");
        }
        Command::Edit { message, text } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            let (conversation, entry) = chat
                .find_message(message)
                .ok_or_else(|| format!("message {message} is not in this device's store"))?;
            if entry.sender != chat.session.username {
                return Err(format!("message {message} is not yours to edit"));
            }
            chat.send_edit(&conversation, message, text).await?;
            thrum_cli::outln!("edited {message}");
        }
        Command::Delete { message } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            let (conversation, entry) = chat
                .find_message(message)
                .ok_or_else(|| format!("message {message} is not in this device's store"))?;
            if entry.sender != chat.session.username {
                return Err(format!("message {message} is not yours to delete"));
            }
            chat.send_delete(&conversation, message).await?;
            thrum_cli::outln!("deleted {message}");
        }
        Command::Attach { conversation, file } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            let id = chat.send_attachment(conversation, file).await?;
            thrum_cli::outln!("sent {id}");
        }
        Command::Fetch { message, out } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            let path = chat.fetch_attachment(message, out.as_deref()).await?;
            thrum_cli::outln!("saved {}", path.display());
        }
        Command::Read { conversation } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            let count = chat.send_read(conversation).await?;
            thrum_cli::outln!("sent read receipts for {count} message(s)");
        }
        Command::Recv { once, timeout } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            // Show what is already queued, then pull in new devices and
            // redeemed invitations before listening, so fresh devices get
            // their welcomes promptly. The queue goes first: planning roster
            // changes before applying a commit already waiting would split
            // the group.
            let mut shown = 0usize;
            thrum_cli::ws::drain(
                &chat.session,
                async |conversation: &str, message_id: &str, sender: &str, payload: &[u8]| {
                    chat.interpret(conversation, message_id, sender, payload)
                        .await
                },
                |line| {
                    shown += 1;
                    thrum_cli::outln!("{line}");
                },
            )
            .await?;
            if let Err(error) = chat.sync().await {
                thrum_cli::errln!("note: sync failed: {error}");
            }
            if *once && shown > 0 {
                return Ok(());
            }
            recv(
                &chat.session,
                &RecvOptions {
                    once: *once,
                    timeout_secs: *timeout,
                },
                async |conversation: &str, message_id: &str, sender: &str, payload: &[u8]| {
                    chat.interpret(conversation, message_id, sender, payload)
                        .await
                },
                |line| thrum_cli::outln!("{line}"),
            )
            .await?;
        }
        Command::Add {
            conversation,
            members,
        } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            let list = parse_member_list(members)?;
            chat.add_members(conversation, &list).await?;
            thrum_cli::outln!("added {}", list.join(", "));
        }
        Command::Remove {
            conversation,
            member,
        } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            chat.remove_member(conversation, member).await?;
            thrum_cli::outln!("removed {member}");
        }
        Command::Invite { conversation } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            let token = chat.create_invite(conversation).await?;
            thrum_cli::outln!("invite {token}");
        }
        Command::Revoke {
            conversation,
            token,
        } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            chat.revoke_invite(conversation, token).await?;
            thrum_cli::outln!("revoked");
        }
        Command::Join { token } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            let conversation = chat.join(token).await?;
            thrum_cli::outln!(
                "joined conversation {conversation} on the server; run `recv` to catch up"
            );
        }
        Command::Block { user } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            chat.block(user).await?;
            thrum_cli::outln!("blocked {user}");
        }
        Command::Unblock { user } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            chat.unblock(user).await?;
            thrum_cli::outln!("unblocked {user}");
        }
        Command::Sync => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            let report = chat.sync().await?;
            report_lines(&report);
            thrum_cli::outln!("synced");
        }
        Command::Trust { device } => {
            let session = load_session(&cli.state_file, base)?;
            let chat = Chat::open(client, session, &cli.state_file)?;
            chat.trust_device(device)?;
            let report = chat.sync().await?;
            report_lines(&report);
            thrum_cli::outln!("trusted {device}");
        }
        Command::RevokeDevice { device } => {
            let session = load_session(&cli.state_file, base)?;
            client.revoke_device(&session.token, device).await?;
            thrum_cli::outln!("revoked {device}");
        }
        Command::Whoami => {
            let session = load_session(&cli.state_file, base)?;
            thrum_cli::outln!(
                "{} on {} (device {})",
                session.username,
                session.server,
                session.device_id
            );
        }
        Command::Export { out } => {
            let session = load_session(&cli.state_file, base)?;
            let document = client.export_account(&session.token).await?;
            let text = serde_json::to_string_pretty(&document).unwrap();
            thrum_core::mls::write_private_file(out, text.as_bytes())
                .map_err(|error| format!("write failed: {error}"))?;
            let msg_count = document["messages"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);
            thrum_cli::outln!(
                "exported {} ({} message(s)) → {}",
                document["account"]["username"],
                msg_count,
                out.display()
            );
        }
        Command::DeleteAccount { password, yes } => {
            if !yes {
                return Err("refusing to delete account; use --yes to confirm".to_string());
            }
            let session = load_session(&cli.state_file, base)?;
            client.delete_account(&session.token, password).await?;
            // The token is dead; the keys, history and attachments go too.
            let removed = thrum_cli::chat::forget_device(&cli.state_file)?;
            thrum_cli::outln!("account {} deleted", session.username);
            for path in removed {
                thrum_cli::outln!("removed {}", path.display());
            }
        }
    }
    Ok(())
}

fn report_lines(report: &thrum_cli::chat::SyncReport) {
    for line in report
        .removed
        .iter()
        .chain(&report.added)
        .chain(&report.pending)
    {
        thrum_cli::errln!("{line}");
    }
}

/// Parse the comma-separated member list.
fn parse_member_list(members: &str) -> Result<Vec<String>, String> {
    let list: Vec<String> = members
        .split(',')
        .map(str::trim)
        .filter(|member| !member.is_empty())
        .map(str::to_string)
        .collect();
    if list.is_empty() {
        return Err("no members given".to_string());
    }
    Ok(list)
}

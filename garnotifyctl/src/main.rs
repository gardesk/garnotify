//! garnotifyctl - Control utility for garnotify daemon

use anyhow::Result;
use clap::{Parser, Subcommand};

mod ipc;

/// garnotifyctl - Control utility for garnotify
#[derive(Parser)]
#[command(name = "garnotifyctl")]
#[command(about = "Control utility for garnotify daemon", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Close a notification by ID
    Close {
        /// Notification ID to close (omit to close most recent)
        id: Option<u32>,
    },
    /// Close all notifications
    CloseAll,
    /// Pop and redisplay the most recent notification from history
    HistoryPop,
    /// Clear notification history
    HistoryClear,
    /// Set Do Not Disturb mode
    SetPaused {
        /// Enable or disable DND ("true" or "false")
        #[arg(value_parser = ["true", "false"])]
        paused: String,
        /// Pause level: 0=show all, 1=critical only, 2=show none
        #[arg(short, long, default_value = "2")]
        level: u8,
    },
    /// Check if Do Not Disturb is enabled
    IsPaused,
    /// Get count of active notifications
    Count,
    /// List active notifications
    List {
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },
    /// Enable a rule by name
    RuleEnable {
        /// Rule name to enable
        name: String,
    },
    /// Disable a rule by name
    RuleDisable {
        /// Rule name to disable
        name: String,
    },
    /// Reload configuration
    Reload,
    /// Get daemon status
    Status,
    /// Stop the daemon
    Quit,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let response = match cli.command {
        Commands::Close { id } => ipc::send_command(&ipc::Command::Close { id })?,
        Commands::CloseAll => ipc::send_command(&ipc::Command::CloseAll)?,
        Commands::HistoryPop => ipc::send_command(&ipc::Command::HistoryPop)?,
        Commands::HistoryClear => ipc::send_command(&ipc::Command::HistoryClear)?,
        Commands::SetPaused { paused, level } => {
            let paused = paused == "true";
            ipc::send_command(&ipc::Command::SetPaused { paused, level })?
        }
        Commands::IsPaused => ipc::send_command(&ipc::Command::IsPaused)?,
        Commands::Count => ipc::send_command(&ipc::Command::Count)?,
        Commands::List { json: _ } => ipc::send_command(&ipc::Command::List)?,
        Commands::RuleEnable { name } => ipc::send_command(&ipc::Command::RuleEnable { name })?,
        Commands::RuleDisable { name } => ipc::send_command(&ipc::Command::RuleDisable { name })?,
        Commands::Reload => ipc::send_command(&ipc::Command::Reload)?,
        Commands::Status => ipc::send_command(&ipc::Command::Status)?,
        Commands::Quit => ipc::send_command(&ipc::Command::Quit)?,
    };

    if response.success {
        if let Some(ref msg) = response.message {
            println!("{}", msg);
        }
        if let Some(ref data) = response.data {
            println!("{}", serde_json::to_string_pretty(data)?);
        }
        if response.message.is_none() && response.data.is_none() {
            println!("OK");
        }
    } else {
        eprintln!("Error: {}", response.message.as_deref().unwrap_or("Unknown error"));
        std::process::exit(1);
    }

    Ok(())
}

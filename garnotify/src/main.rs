//! garnotify - Notification daemon for gar desktop
//!
//! A FreeDesktop-compliant notification daemon that implements
//! the org.freedesktop.Notifications D-Bus interface.

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

mod config;
mod daemon;
mod dbus;
mod ipc;
mod notification;
mod ui;

/// garnotify - Notification daemon for gar desktop
#[derive(Parser)]
#[command(name = "garnotify")]
#[command(about = "Notification daemon for gar desktop", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run in foreground (don't daemonize)
    #[arg(short, long)]
    foreground: bool,

    /// Configuration file path
    #[arg(short, long)]
    config: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the notification daemon
    Daemon {
        /// Run in foreground
        #[arg(short, long)]
        foreground: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Daemon { foreground }) => {
            info!("Starting garnotify daemon");
            daemon::run(cli.config, foreground || cli.foreground).await
        }
        None => {
            // Default: run daemon
            info!("Starting garnotify daemon");
            daemon::run(cli.config, cli.foreground).await
        }
    }
}

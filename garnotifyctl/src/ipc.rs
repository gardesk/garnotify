//! IPC client for garnotifyctl

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// Get the path to the IPC socket
fn socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("garnotify.sock")
}

/// IPC commands (must match daemon's Command enum)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    Close {
        #[serde(default)]
        id: Option<u32>,
    },
    CloseAll,
    HistoryPop,
    HistoryClear,
    SetPaused {
        paused: bool,
        #[serde(default)]
        level: u8,
    },
    IsPaused,
    Count,
    List,
    RuleEnable {
        name: String,
    },
    RuleDisable {
        name: String,
    },
    Reload,
    Status,
    Quit,
}

/// IPC response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub success: bool,
    pub message: Option<String>,
    pub data: Option<serde_json::Value>,
}

/// Send a command to the running daemon
pub fn send_command(cmd: &Command) -> Result<Response> {
    let path = socket_path();

    if !path.exists() {
        anyhow::bail!("garnotify daemon not running (socket not found at {})", path.display());
    }

    let mut stream =
        UnixStream::connect(&path).with_context(|| "Failed to connect to garnotify daemon")?;

    let cmd_json = serde_json::to_string(cmd)?;
    writeln!(stream, "{}", cmd_json)?;
    stream.flush()?;

    // Read response
    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    reader.read_line(&mut response_line)?;

    let response: Response = serde_json::from_str(&response_line)
        .with_context(|| "Failed to parse response from daemon")?;

    Ok(response)
}

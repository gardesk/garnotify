//! IPC for garnotify daemon <-> garnotifyctl communication
//!
//! Uses Unix domain sockets with JSON protocol and oneshot channels
//! for proper request-response handling.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info};

/// Get the path to the IPC socket
pub fn socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("garnotify.sock")
}

/// IPC commands
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Close notification by ID (or all if None)
    Close {
        #[serde(default)]
        id: Option<u32>,
    },
    /// Close all notifications
    CloseAll,
    /// Pop notification from history
    HistoryPop,
    /// Clear notification history
    HistoryClear,
    /// Set paused (DND) state
    SetPaused {
        paused: bool,
        #[serde(default)]
        level: u8,
    },
    /// Query paused state
    IsPaused,
    /// Get notification count
    Count,
    /// List active notifications
    List,
    /// Enable a rule by name
    RuleEnable { name: String },
    /// Disable a rule by name
    RuleDisable { name: String },
    /// Reload configuration
    Reload,
    /// Get daemon status
    Status,
    /// Quit the daemon
    Quit,
}

/// IPC response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            success: true,
            message: None,
            data: None,
        }
    }

    pub fn ok_with_message(msg: impl Into<String>) -> Self {
        Self {
            success: true,
            message: Some(msg.into()),
            data: None,
        }
    }

    pub fn ok_with_data(data: serde_json::Value) -> Self {
        Self {
            success: true,
            message: None,
            data: Some(data),
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            message: Some(msg.into()),
            data: None,
        }
    }
}

/// An IPC request wrapping a command with a response channel
pub struct IpcRequest {
    pub command: Command,
    pub response_tx: std::sync::mpsc::Sender<Response>,
}

/// IPC server for the daemon
pub struct IpcServer {
    socket_path: PathBuf,
    listener: Option<UnixListener>,
    tx: Sender<IpcRequest>,
}

impl IpcServer {
    /// Create a new IPC server
    pub fn new() -> (Self, Receiver<IpcRequest>) {
        let (tx, rx) = mpsc::channel();
        let server = Self {
            socket_path: socket_path(),
            listener: None,
            tx,
        };
        (server, rx)
    }

    /// Start listening for connections
    pub fn start(&mut self) -> Result<()> {
        // Remove stale socket
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        let listener = UnixListener::bind(&self.socket_path)
            .with_context(|| format!("Failed to bind socket: {}", self.socket_path.display()))?;

        info!("IPC server listening on {}", self.socket_path.display());

        let tx = self.tx.clone();
        self.listener = Some(listener.try_clone()?);

        // Spawn listener thread
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let tx = tx.clone();
                        thread::spawn(move || {
                            if let Err(e) = handle_client(stream, tx) {
                                error!("Client error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Accept error: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    /// Stop the IPC server
    pub fn stop(&mut self) {
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
        debug!("IPC server stopped");
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Handle a client connection
fn handle_client(mut stream: UnixStream, tx: Sender<IpcRequest>) -> Result<()> {
    let reader = BufReader::new(stream.try_clone()?);

    for line in reader.lines() {
        let line = line?;
        debug!("Received: {}", line);

        let response = match serde_json::from_str::<Command>(&line) {
            Ok(cmd) => {
                // Create response channel
                let (response_tx, response_rx) = mpsc::channel();

                // Forward command with response channel to daemon
                let request = IpcRequest {
                    command: cmd,
                    response_tx,
                };

                if tx.send(request).is_err() {
                    Response::error("Daemon not responding")
                } else {
                    // Wait for response from daemon (with timeout)
                    match response_rx.recv_timeout(Duration::from_secs(5)) {
                        Ok(response) => response,
                        Err(_) => Response::error("Timeout waiting for daemon response"),
                    }
                }
            }
            Err(e) => Response::error(format!("Invalid command: {}", e)),
        };

        let response_json = serde_json::to_string(&response)?;
        writeln!(stream, "{}", response_json)?;
        stream.flush()?;
    }

    Ok(())
}

/// Send a command to the running daemon (client side)
pub fn send_command(cmd: &Command) -> Result<Response> {
    let path = socket_path();

    if !path.exists() {
        anyhow::bail!("garnotify daemon not running (socket not found)");
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

    let response: Response = serde_json::from_str(&response_line)?;
    Ok(response)
}

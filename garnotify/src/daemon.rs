//! Daemon state machine and main event loop for garnotify

use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::config::{self, Config};
use crate::dbus::NotificationsService;
use crate::ipc::{Command, IpcRequest, IpcServer, Response};
use crate::notification::{
    new_shared_store, CloseReason, History, Notification, NotificationEvent,
    SharedNotificationStore, UrgencyTimeouts,
};
use crate::rules::RuleEngine;
use crate::ui::{PopupCommand, PopupEvent, PopupManager};

/// Get the path to the PID file
fn pid_file_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("garnotify.pid")
}

/// Check if an existing daemon is running
fn check_existing_daemon() -> Result<()> {
    let pid_path = pid_file_path();
    let socket_path = crate::ipc::socket_path();

    if pid_path.exists() {
        let pid_str = fs::read_to_string(&pid_path)?;
        let pid: i32 = pid_str.trim().parse()?;

        let proc_path = format!("/proc/{}", pid);
        if std::path::Path::new(&proc_path).exists() {
            anyhow::bail!(
                "garnotify daemon already running (PID {}). Remove {} if incorrect.",
                pid,
                pid_path.display()
            );
        } else {
            warn!("Removing stale PID file for PID {}", pid);
            fs::remove_file(&pid_path)?;
            // Also clean up stale socket
            if socket_path.exists() {
                warn!("Removing stale socket file");
                let _ = fs::remove_file(&socket_path);
            }
        }
    } else if socket_path.exists() {
        // Socket exists but no PID file - orphaned socket
        warn!("Removing orphaned socket file (no PID file)");
        let _ = fs::remove_file(&socket_path);
    }

    Ok(())
}

/// Write the current process PID to the PID file
fn write_pid_file() -> Result<()> {
    let pid_path = pid_file_path();
    let pid = std::process::id();

    let mut file = fs::File::create(&pid_path)?;
    writeln!(file, "{}", pid)?;

    debug!("Wrote PID {} to {}", pid, pid_path.display());
    Ok(())
}

/// Remove the PID file
fn remove_pid_file() {
    let pid_path = pid_file_path();
    if let Err(e) = fs::remove_file(&pid_path) {
        warn!("Failed to remove PID file: {}", e);
    } else {
        debug!("Removed PID file");
    }
}

/// PID file guard - removes on drop
struct PidGuard;

impl Drop for PidGuard {
    fn drop(&mut self) {
        remove_pid_file();
    }
}

/// DND pause level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseLevel {
    /// Show all notifications (not paused)
    ShowAll = 0,
    /// Only show critical notifications
    CriticalOnly = 1,
    /// Show no notifications
    ShowNone = 2,
}

impl From<u8> for PauseLevel {
    fn from(level: u8) -> Self {
        match level {
            0 => PauseLevel::ShowAll,
            1 => PauseLevel::CriticalOnly,
            _ => PauseLevel::ShowNone,
        }
    }
}

/// Daemon state
pub struct Daemon {
    config: Arc<Config>,
    ipc_server: IpcServer,
    ipc_rx: Receiver<IpcRequest>,
    dbus_service: Option<NotificationsService>,
    notification_store: SharedNotificationStore,
    notification_event_rx: tokio::sync::mpsc::Receiver<NotificationEvent>,
    history: Arc<Mutex<History>>,
    running: bool,
    /// Do Not Disturb mode
    paused: bool,
    /// Pause level (0=show all, 1=critical only, 2=show none)
    pause_level: PauseLevel,
    /// Rule engine for filtering/modifying notifications
    rule_engine: RuleEngine,
    /// Channel to send commands to the UI thread
    ui_cmd_tx: Option<std::sync::mpsc::Sender<PopupCommand>>,
    /// Channel to receive events from the UI thread
    ui_event_rx: Option<tokio::sync::mpsc::Receiver<PopupEvent>>,
    /// Handle to the UI thread
    ui_thread: Option<std::thread::JoinHandle<()>>,
}

impl Daemon {
    /// Create a new daemon
    pub fn new(config: Config) -> Result<Self> {
        let (ipc_server, ipc_rx) = IpcServer::new();

        // Create notification event channel
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(100);

        // Create urgency timeouts from config
        let urgency_timeouts = UrgencyTimeouts {
            low: config.timeouts.low,
            normal: config.timeouts.normal,
            critical: config.timeouts.critical,
        };

        // Create notification store with tokio handle for spawning timeout tasks
        let tokio_handle = tokio::runtime::Handle::current();
        let notification_store = new_shared_store(event_tx, urgency_timeouts, tokio_handle);

        // Create history
        let history = Arc::new(Mutex::new(History::new(config.history.max_length)));

        // Create rule engine from config
        let rule_engine = RuleEngine::with_rules(config.rules.clone());

        let config = Arc::new(config);

        Ok(Self {
            config,
            ipc_server,
            ipc_rx,
            dbus_service: None,
            notification_store,
            notification_event_rx: event_rx,
            history,
            running: true,
            paused: false,
            pause_level: PauseLevel::ShowAll,
            rule_engine,
            ui_cmd_tx: None,
            ui_event_rx: None,
            ui_thread: None,
        })
    }

    /// Initialize IPC server
    pub fn init_ipc(&mut self) -> Result<()> {
        self.ipc_server
            .start()
            .context("Failed to start IPC server")?;
        info!("IPC server started");
        Ok(())
    }

    /// Initialize history (load from file if persistence is enabled)
    pub async fn init_history(&mut self) -> Result<()> {
        if self.config.history.persist {
            let mut history = self.history.lock().await;
            match history.load_from_file() {
                Ok(count) => {
                    if count > 0 {
                        info!("Loaded {} notifications from history", count);
                    }
                }
                Err(e) => {
                    warn!("Failed to load history: {}", e);
                }
            }
        }
        Ok(())
    }

    /// Save history to file (if persistence is enabled)
    async fn save_history(&self) {
        if self.config.history.persist {
            let history = self.history.lock().await;
            if let Err(e) = history.save_to_file() {
                error!("Failed to save history: {}", e);
            }
        }
    }

    /// Initialize D-Bus service
    pub async fn init_dbus(&mut self) -> Result<()> {
        let service = NotificationsService::new(
            self.config.clone(),
            self.notification_store.clone(),
        )
        .await?;
        self.dbus_service = Some(service);
        info!("D-Bus service registered");
        Ok(())
    }

    /// Initialize UI (popup manager) in a separate thread
    pub fn init_ui(&mut self) -> Result<()> {
        let config = self.config.clone();

        // Create channels for communication
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<PopupCommand>();
        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<PopupEvent>(100);

        // Spawn UI thread
        let ui_thread = std::thread::Builder::new()
            .name("garnotify-ui".into())
            .spawn(move || {
                if let Err(e) = run_ui_thread(config, cmd_rx, event_tx) {
                    error!("UI thread error: {}", e);
                }
            })
            .context("Failed to spawn UI thread")?;

        self.ui_cmd_tx = Some(cmd_tx);
        self.ui_event_rx = Some(event_rx);
        self.ui_thread = Some(ui_thread);

        info!("UI thread started");
        Ok(())
    }

    /// Send a command to the UI thread
    fn send_ui_command(&self, cmd: PopupCommand) {
        if let Some(ref tx) = self.ui_cmd_tx {
            if let Err(e) = tx.send(cmd) {
                warn!("Failed to send UI command: {}", e);
            }
        }
    }

    /// Show a notification popup
    fn show_notification_popup(&self, notification: Notification) {
        self.send_ui_command(PopupCommand::Show(notification));
    }

    /// Close a notification popup
    fn close_notification_popup(&self, id: u32, reason: CloseReason) {
        self.send_ui_command(PopupCommand::Close { id, reason });
    }

    /// Run the main event loop
    pub async fn run(&mut self) -> Result<()> {
        info!("Entering main event loop");

        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sighup = signal(SignalKind::hangup())?;

        while self.running {
            // Check for IPC commands (non-blocking)
            self.poll_ipc_commands().await;

            // Check for UI events (non-blocking)
            self.poll_ui_events().await;

            tokio::select! {
                _ = sigterm.recv() => {
                    info!("Received SIGTERM, shutting down");
                    self.running = false;
                }
                _ = sighup.recv() => {
                    info!("Received SIGHUP, reloading config");
                    self.handle_reload()?;
                }
                _ = tokio::signal::ctrl_c() => {
                    info!("Received Ctrl+C, shutting down");
                    self.running = false;
                }
                Some(event) = self.notification_event_rx.recv() => {
                    self.handle_notification_event(event).await;
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(50)) => {
                    // Poll interval for IPC and UI events
                }
            }
        }

        // Save history before shutdown
        self.save_history().await;

        // Clean up UI thread
        if let Some(tx) = self.ui_cmd_tx.take() {
            drop(tx); // Close the channel to signal UI thread to exit
        }
        if let Some(handle) = self.ui_thread.take() {
            let _ = handle.join();
        }

        info!("Daemon shutdown complete");
        Ok(())
    }

    /// Poll for UI events
    async fn poll_ui_events(&mut self) {
        // Collect events first to avoid borrow issues
        let events: Vec<PopupEvent> = if let Some(ref mut rx) = self.ui_event_rx {
            let mut events = Vec::new();
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
            events
        } else {
            Vec::new()
        };

        // Handle collected events
        for event in events {
            self.handle_ui_event(event).await;
        }
    }

    /// Handle a UI event
    async fn handle_ui_event(&mut self, event: PopupEvent) {
        match event {
            PopupEvent::Dismissed(id) => {
                info!("Notification {} dismissed by user", id);
                // Remove from store and emit D-Bus signal
                let mut store = self.notification_store.lock().await;
                if let Some(notification) = store.remove(id) {
                    let mut history = self.history.lock().await;
                    history.push(notification);
                }
                if let Some(ref dbus) = self.dbus_service {
                    if let Err(e) = dbus.emit_closed(id, CloseReason::Dismissed).await {
                        error!("Failed to emit NotificationClosed signal: {}", e);
                    }
                }
            }
            PopupEvent::ActionInvoked { id, action_key } => {
                info!("Action '{}' invoked on notification {}", action_key, id);
                if let Some(ref dbus) = self.dbus_service {
                    if let Err(e) = dbus.emit_action_invoked(id, &action_key).await {
                        error!("Failed to emit ActionInvoked signal: {}", e);
                    }
                }
            }
            PopupEvent::Closed { id, reason } => {
                debug!("Popup closed for notification {}: {:?}", id, reason);
                // Already handled by the notification event
            }
        }
    }

    /// Poll for IPC commands
    async fn poll_ipc_commands(&mut self) {
        while let Ok(request) = self.ipc_rx.try_recv() {
            let response = self.handle_ipc_command(request.command).await;
            // Send response back (ignore error if receiver dropped)
            let _ = request.response_tx.send(response);
        }
    }

    /// Handle a notification event (created, updated, closed)
    async fn handle_notification_event(&mut self, event: NotificationEvent) {
        match event {
            NotificationEvent::Created(mut notification) => {
                info!(
                    "Notification created: id={} summary=\"{}\"",
                    notification.id, notification.summary
                );

                // Process through rule engine (may modify or suppress)
                if self.rule_engine.process(&mut notification).is_none() {
                    debug!(
                        "Notification {} suppressed by rules",
                        notification.id
                    );
                    return;
                }

                // Check DND mode before showing popup
                let should_show = if !self.paused {
                    true
                } else {
                    match self.pause_level {
                        PauseLevel::ShowAll => true,
                        PauseLevel::CriticalOnly => {
                            notification.hints.urgency == crate::notification::Urgency::Critical
                        }
                        PauseLevel::ShowNone => false,
                    }
                };

                if should_show {
                    self.show_notification_popup(notification);
                } else {
                    debug!(
                        "Notification {} suppressed by DND (level={:?})",
                        notification.id, self.pause_level
                    );
                }
            }
            NotificationEvent::Updated(notification) => {
                info!(
                    "Notification updated: id={} summary=\"{}\"",
                    notification.id, notification.summary
                );
                // Update popup
                self.send_ui_command(PopupCommand::Update {
                    id: notification.id,
                    notification,
                });
            }
            NotificationEvent::Closed { id, reason } => {
                info!("Notification {} closed: reason={:?}", id, reason);

                // Close popup
                self.close_notification_popup(id, reason.clone());

                // Remove from store and add to history
                let mut store = self.notification_store.lock().await;
                if let Some(notification) = store.remove(id) {
                    // Add to history (unless transient)
                    let mut history = self.history.lock().await;
                    history.push(notification);
                }

                // Emit D-Bus signal
                if let Some(ref dbus) = self.dbus_service {
                    if let Err(e) = dbus.emit_closed(id, reason).await {
                        error!("Failed to emit NotificationClosed signal: {}", e);
                    }
                }
            }
        }
    }

    /// Handle an IPC command and return a response
    async fn handle_ipc_command(&mut self, cmd: Command) -> Response {
        debug!("Handling IPC command: {:?}", cmd);
        match cmd {
            Command::Status => {
                let store = self.notification_store.lock().await;
                let history = self.history.lock().await;
                let status = serde_json::json!({
                    "running": true,
                    "active_count": store.count(),
                    "history_count": history.len(),
                    "paused": self.paused,
                    "pause_level": self.pause_level as u8,
                });
                Response::ok_with_data(status)
            }
            Command::Reload => {
                info!("Reloading config via IPC");
                match self.handle_reload() {
                    Ok(_) => Response::ok_with_message("Configuration reloaded"),
                    Err(e) => Response::error(format!("Failed to reload: {}", e)),
                }
            }
            Command::Quit => {
                info!("Quit requested via IPC");
                self.running = false;
                Response::ok_with_message("Shutting down")
            }
            Command::Close { id } => {
                let target_id = if let Some(id) = id {
                    Some(id)
                } else {
                    // Close most recent
                    let store = self.notification_store.lock().await;
                    store.list().last().map(|n| n.id)
                };

                if let Some(id) = target_id {
                    info!("Close notification: {}", id);
                    // Send close command to UI
                    self.close_notification_popup(id, CloseReason::Closed);

                    let mut store = self.notification_store.lock().await;
                    if let Some(notification) = store.remove(id) {
                        let mut history = self.history.lock().await;
                        history.push(notification);

                        // Emit D-Bus signal
                        if let Some(ref dbus) = self.dbus_service {
                            if let Err(e) = dbus.emit_closed(id, CloseReason::Closed).await {
                                error!("Failed to emit NotificationClosed signal: {}", e);
                            }
                        }
                    }
                    Response::ok_with_message(format!("Closed notification {}", id))
                } else {
                    Response::ok_with_message("No notifications to close")
                }
            }
            Command::CloseAll => {
                info!("Close all notifications");
                // Send close-all to UI
                self.send_ui_command(PopupCommand::CloseAll);

                let mut store = self.notification_store.lock().await;
                let notifications = store.clear();
                let count = notifications.len();
                let mut history = self.history.lock().await;

                for notification in notifications {
                    let id = notification.id;
                    history.push(notification);

                    // Emit D-Bus signal for each
                    if let Some(ref dbus) = self.dbus_service {
                        if let Err(e) = dbus.emit_closed(id, CloseReason::Closed).await {
                            error!("Failed to emit NotificationClosed signal: {}", e);
                        }
                    }
                }
                Response::ok_with_message(format!("Closed {} notifications", count))
            }
            Command::HistoryPop => {
                let mut history = self.history.lock().await;
                if let Some(notification) = history.pop() {
                    info!(
                        "Popped from history: id={} summary=\"{}\"",
                        notification.id, notification.summary
                    );
                    // Re-display the notification
                    drop(history); // Release lock before showing popup
                    self.show_notification_popup(notification);
                    Response::ok_with_message("Restored notification from history")
                } else {
                    Response::ok_with_message("History is empty")
                }
            }
            Command::HistoryClear => {
                let mut history = self.history.lock().await;
                let count = history.len();
                history.clear();
                Response::ok_with_message(format!("Cleared {} items from history", count))
            }
            Command::SetPaused { paused, level } => {
                self.paused = paused;
                self.pause_level = PauseLevel::from(level);
                info!(
                    "DND mode: {} (level {:?})",
                    if paused { "enabled" } else { "disabled" },
                    self.pause_level
                );
                Response::ok_with_message(format!(
                    "DND {}",
                    if paused { "enabled" } else { "disabled" }
                ))
            }
            Command::IsPaused => {
                let data = serde_json::json!({
                    "paused": self.paused,
                    "level": self.pause_level as u8,
                });
                Response::ok_with_data(data)
            }
            Command::Count => {
                let store = self.notification_store.lock().await;
                let data = serde_json::json!({
                    "count": store.count(),
                });
                Response::ok_with_data(data)
            }
            Command::List => {
                let store = self.notification_store.lock().await;
                let list: Vec<_> = store
                    .list()
                    .iter()
                    .map(|n| {
                        serde_json::json!({
                            "id": n.id,
                            "app_name": n.app_name,
                            "summary": n.summary,
                            "body": n.body,
                            "urgency": format!("{:?}", n.hints.urgency),
                        })
                    })
                    .collect();
                Response::ok_with_data(serde_json::json!(list))
            }
            Command::RuleEnable { name } => {
                if self.rule_engine.enable_rule(&name) {
                    Response::ok_with_message(format!("Enabled rule '{}'", name))
                } else {
                    Response::error(format!("Rule '{}' not found", name))
                }
            }
            Command::RuleDisable { name } => {
                if self.rule_engine.disable_rule(&name) {
                    Response::ok_with_message(format!("Disabled rule '{}'", name))
                } else {
                    Response::error(format!("Rule '{}' not found", name))
                }
            }
        }
    }

    /// Handle config reload
    fn handle_reload(&mut self) -> Result<()> {
        match config::load(None) {
            Ok(new_config) => {
                info!("Reloaded configuration");
                // Reload rules
                self.rule_engine = RuleEngine::with_rules(new_config.rules.clone());
                self.config = Arc::new(new_config);
            }
            Err(e) => {
                warn!("Failed to reload config: {}", e);
            }
        }
        Ok(())
    }
}

/// Run the notification daemon
pub async fn run(config_path: Option<String>, _foreground: bool) -> Result<()> {
    check_existing_daemon()?;
    write_pid_file()?;
    let _pid_guard = PidGuard;

    let config = config::load(config_path.as_deref())?;
    info!("Loaded configuration");
    info!("Position: {}", config.geometry.position);
    info!("Width: {}px", config.geometry.width);
    info!(
        "Timeouts: low={}ms, normal={}ms, critical={}ms",
        config.timeouts.low, config.timeouts.normal, config.timeouts.critical
    );

    let mut daemon = Daemon::new(config)?;
    daemon.init_ipc().context("Failed to initialize IPC")?;
    daemon
        .init_dbus()
        .await
        .context("Failed to initialize D-Bus")?;
    daemon.init_ui().context("Failed to initialize UI")?;
    daemon
        .init_history()
        .await
        .context("Failed to initialize history")?;
    daemon.run().await
}

/// Run the UI thread - handles X11 popup windows
fn run_ui_thread(
    config: Arc<Config>,
    cmd_rx: std::sync::mpsc::Receiver<PopupCommand>,
    event_tx: tokio::sync::mpsc::Sender<PopupEvent>,
) -> Result<()> {
    let mut manager = PopupManager::new(config, event_tx)?;

    loop {
        // Check for commands (with timeout to allow X11 event polling)
        match cmd_rx.recv_timeout(std::time::Duration::from_millis(16)) {
            Ok(cmd) => {
                if let Err(e) = manager.handle_command(cmd) {
                    error!("Failed to handle UI command: {}", e);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Normal timeout, continue to poll X11 events
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // Channel closed, daemon shutting down
                info!("UI thread: command channel closed, exiting");
                break;
            }
        }

        // Poll X11 events
        if let Err(e) = manager.poll_events() {
            error!("Failed to poll X11 events: {}", e);
        }

        // Update animations
        if let Err(e) = manager.update_animations() {
            error!("Failed to update animations: {}", e);
        }
    }

    Ok(())
}

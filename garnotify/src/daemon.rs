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
use crate::ipc::{Command, IpcServer};
use crate::notification::{
    new_shared_store, CloseReason, History, Notification, NotificationEvent,
    SharedNotificationStore, UrgencyTimeouts,
};
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

/// Daemon state
pub struct Daemon {
    config: Arc<Config>,
    ipc_server: IpcServer,
    ipc_rx: Receiver<Command>,
    dbus_service: Option<NotificationsService>,
    notification_store: SharedNotificationStore,
    notification_event_rx: tokio::sync::mpsc::Receiver<NotificationEvent>,
    history: Arc<Mutex<History>>,
    running: bool,
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
        while let Ok(cmd) = self.ipc_rx.try_recv() {
            self.handle_ipc_command(cmd).await;
        }
    }

    /// Handle a notification event (created, updated, closed)
    async fn handle_notification_event(&mut self, event: NotificationEvent) {
        match event {
            NotificationEvent::Created(notification) => {
                info!(
                    "Notification created: id={} summary=\"{}\"",
                    notification.id, notification.summary
                );
                // Show popup
                self.show_notification_popup(notification);
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

    /// Handle an IPC command
    async fn handle_ipc_command(&mut self, cmd: Command) {
        debug!("Handling IPC command: {:?}", cmd);
        match cmd {
            Command::Status => {
                let store = self.notification_store.lock().await;
                let history = self.history.lock().await;
                info!(
                    "Status: running, {} active notifications, {} in history",
                    store.count(),
                    history.len()
                );
            }
            Command::Reload => {
                info!("Reloading config via IPC");
                if let Err(e) = self.handle_reload() {
                    error!("Failed to reload config: {}", e);
                }
            }
            Command::Quit => {
                info!("Quit requested via IPC");
                self.running = false;
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
                }
            }
            Command::CloseAll => {
                info!("Close all notifications");
                let mut store = self.notification_store.lock().await;
                let notifications = store.clear();
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
            }
            Command::HistoryPop => {
                info!("History pop requested");
                let mut history = self.history.lock().await;
                if let Some(notification) = history.pop() {
                    info!(
                        "Popped from history: id={} summary=\"{}\"",
                        notification.id, notification.summary
                    );
                    // TODO: In sprint 3, re-display the notification
                } else {
                    info!("History is empty");
                }
            }
            Command::HistoryClear => {
                info!("History clear requested");
                let mut history = self.history.lock().await;
                history.clear();
            }
            Command::SetPaused { paused, level } => {
                info!("Set paused: {} (level {})", paused, level);
                // TODO: Implement in sprint 4
            }
            Command::IsPaused => {
                info!("Is paused query");
                // TODO: Implement in sprint 4
            }
            Command::Count => {
                let store = self.notification_store.lock().await;
                info!("Notification count: {}", store.count());
            }
            Command::List => {
                let store = self.notification_store.lock().await;
                info!("Active notifications:");
                for notification in store.list() {
                    info!(
                        "  id={} app=\"{}\" summary=\"{}\"",
                        notification.id, notification.app_name, notification.summary
                    );
                }
            }
            Command::RuleEnable { name } => {
                info!("Rule enable: {}", name);
                // TODO: Implement in sprint 4
            }
            Command::RuleDisable { name } => {
                info!("Rule disable: {}", name);
                // TODO: Implement in sprint 4
            }
        }
    }

    /// Handle config reload
    fn handle_reload(&mut self) -> Result<()> {
        match config::load(None) {
            Ok(new_config) => {
                info!("Reloaded configuration");
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
    }

    Ok(())
}

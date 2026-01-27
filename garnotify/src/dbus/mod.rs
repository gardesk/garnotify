//! D-Bus interface implementation for org.freedesktop.Notifications
//!
//! Implements the FreeDesktop Desktop Notifications Specification 1.2
//! https://specifications.freedesktop.org/notification/latest-single/

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};
use zbus::{interface, Connection, SignalContext};

use crate::config::Config;
use crate::notification::{
    Action, CloseReason, Hints, Notification, SharedNotificationStore,
};
use crate::notification::store::next_notification_id;

/// The notifications D-Bus service
pub struct NotificationsService {
    connection: Connection,
}

impl NotificationsService {
    /// Create and register the notifications service
    pub async fn new(config: Arc<Config>, store: SharedNotificationStore) -> Result<Self> {
        let connection = Connection::session()
            .await
            .context("Failed to connect to session bus")?;

        let service = NotificationsInterface {
            config,
            store,
        };

        connection
            .object_server()
            .at("/org/freedesktop/Notifications", service)
            .await
            .context("Failed to register D-Bus object")?;

        connection
            .request_name("org.freedesktop.Notifications")
            .await
            .context("Failed to request D-Bus name (is another notification daemon running?)")?;

        info!("Registered org.freedesktop.Notifications on session bus");

        Ok(Self { connection })
    }

    /// Get the D-Bus connection
    #[allow(dead_code)]
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Emit NotificationClosed signal
    pub async fn emit_closed(&self, id: u32, reason: CloseReason) -> Result<()> {
        let iface_ref = self
            .connection
            .object_server()
            .interface::<_, NotificationsInterface>("/org/freedesktop/Notifications")
            .await?;

        NotificationsInterface::notification_closed(
            iface_ref.signal_context(),
            id,
            reason.as_u32(),
        )
        .await?;

        debug!("Emitted NotificationClosed signal: id={} reason={:?}", id, reason);
        Ok(())
    }

    /// Emit ActionInvoked signal
    #[allow(dead_code)]
    pub async fn emit_action_invoked(&self, id: u32, action_key: &str) -> Result<()> {
        let iface_ref = self
            .connection
            .object_server()
            .interface::<_, NotificationsInterface>("/org/freedesktop/Notifications")
            .await?;

        NotificationsInterface::action_invoked(
            iface_ref.signal_context(),
            id,
            action_key,
        )
        .await?;

        debug!("Emitted ActionInvoked signal: id={} action={}", id, action_key);
        Ok(())
    }
}

/// The actual D-Bus interface implementation
pub struct NotificationsInterface {
    config: Arc<Config>,
    store: SharedNotificationStore,
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationsInterface {
    /// Returns the capabilities supported by this notification server.
    async fn get_capabilities(&self) -> Vec<String> {
        debug!("GetCapabilities called");
        vec![
            "actions".into(),
            "body".into(),
            "body-markup".into(),
            "body-hyperlinks".into(),
            "icon-static".into(),
            "persistence".into(),
        ]
    }

    /// Sends a notification to the notification server.
    ///
    /// Returns the notification ID.
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, zbus::zvariant::Value<'_>>,
        expire_timeout: i32,
    ) -> u32 {
        // Generate or use replacement ID
        let id = if replaces_id > 0 {
            replaces_id
        } else {
            next_notification_id()
        };

        // Parse hints
        let parsed_hints = Hints::parse_from_dbus(&hints);

        // Parse actions
        let parsed_actions = Action::parse_from_dbus(&actions);

        info!(
            "Notify: id={} app=\"{}\" summary=\"{}\" urgency={} actions={}",
            id,
            app_name,
            summary,
            parsed_hints.urgency,
            parsed_actions.len()
        );

        if !body.is_empty() {
            debug!("  body=\"{}\"", body);
        }
        if !app_icon.is_empty() {
            debug!("  icon=\"{}\"", app_icon);
        }
        debug!("  timeout={}ms", expire_timeout);

        // Create notification
        let notification = Notification::new(
            id,
            app_name,
            replaces_id,
            app_icon,
            summary,
            body,
            parsed_actions,
            parsed_hints,
            expire_timeout,
        );

        // Store notification
        let mut store = self.store.lock().await;
        let stored_id = store.add(notification);

        stored_id
    }

    /// Closes the notification with the given ID.
    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_context)] ctx: SignalContext<'_>,
    ) -> zbus::fdo::Result<()> {
        info!("CloseNotification: id={}", id);

        // Remove from store
        let mut store = self.store.lock().await;
        if store.remove(id).is_some() {
            // Emit the NotificationClosed signal with reason 3 (closed by CloseNotification)
            Self::notification_closed(&ctx, id, CloseReason::Closed.as_u32()).await?;
        } else {
            warn!("CloseNotification: notification {} not found", id);
        }

        Ok(())
    }

    /// Returns information about the notification server.
    async fn get_server_information(&self) -> (String, String, String, String) {
        debug!("GetServerInformation called");
        (
            "garnotify".into(),
            "gardesk".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.2".into(), // Spec version we implement
        )
    }

    /// Signal emitted when a notification is closed.
    ///
    /// Reason codes:
    /// 1 - The notification expired
    /// 2 - The notification was dismissed by the user
    /// 3 - The notification was closed by CloseNotification
    /// 4 - Reserved
    #[zbus(signal)]
    async fn notification_closed(
        ctx: &SignalContext<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    /// Signal emitted when a notification action is invoked.
    #[zbus(signal)]
    async fn action_invoked(
        ctx: &SignalContext<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

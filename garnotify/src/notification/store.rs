//! Notification storage and timeout management

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use super::types::{CloseReason, Notification, UrgencyTimeouts};

/// Counter for notification IDs
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// Generate the next notification ID
pub fn next_notification_id() -> u32 {
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

/// Event sent for notification lifecycle changes
#[derive(Debug, Clone)]
pub enum NotificationEvent {
    /// A new notification was created
    Created(Notification),
    /// A notification was updated (replaced)
    Updated(Notification),
    /// A notification should be closed
    Closed { id: u32, reason: CloseReason },
}

/// Notification storage with timeout management
pub struct NotificationStore {
    /// Active notifications by ID
    notifications: HashMap<u32, Notification>,
    /// Timeout handles by notification ID
    timeouts: HashMap<u32, JoinHandle<()>>,
    /// Channel to send expiration events
    event_tx: tokio::sync::mpsc::Sender<NotificationEvent>,
    /// Default timeout configuration
    urgency_timeouts: UrgencyTimeouts,
    /// Tokio runtime handle for spawning tasks
    tokio_handle: Handle,
}

impl NotificationStore {
    /// Create a new notification store
    pub fn new(
        event_tx: tokio::sync::mpsc::Sender<NotificationEvent>,
        urgency_timeouts: UrgencyTimeouts,
        tokio_handle: Handle,
    ) -> Self {
        Self {
            notifications: HashMap::new(),
            timeouts: HashMap::new(),
            event_tx,
            urgency_timeouts,
            tokio_handle,
        }
    }

    /// Add a notification to the store
    ///
    /// If replaces_id > 0 and exists, replaces that notification.
    /// Returns the notification ID.
    pub fn add(&mut self, mut notification: Notification) -> u32 {
        // Handle replacement
        let is_replacement = notification.replaces_id > 0
            && self.notifications.contains_key(&notification.replaces_id);

        let id = if is_replacement {
            // Cancel existing timeout
            if let Some(handle) = self.timeouts.remove(&notification.replaces_id) {
                handle.abort();
            }
            notification.replaces_id
        } else {
            notification.id
        };

        // Update ID if we're replacing
        notification.id = id;

        info!(
            "Storing notification: id={} app=\"{}\" summary=\"{}\" urgency={}",
            id, notification.app_name, notification.summary, notification.hints.urgency
        );

        // Schedule timeout if needed
        self.schedule_timeout(&notification);

        // Send event for UI to show/update popup
        let event = if is_replacement {
            NotificationEvent::Updated(notification.clone())
        } else {
            NotificationEvent::Created(notification.clone())
        };
        let tx = self.event_tx.clone();
        let _ = self.tokio_handle.spawn(async move {
            let _ = tx.send(event).await;
        });

        // Store notification
        self.notifications.insert(id, notification);

        id
    }

    /// Schedule a timeout for a notification
    fn schedule_timeout(&mut self, notification: &Notification) {
        let timeout_ms = notification.effective_timeout(
            self.urgency_timeouts.normal,
            &self.urgency_timeouts,
        );

        if let Some(ms) = timeout_ms {
            let id = notification.id;
            let tx = self.event_tx.clone();

            debug!("Scheduling timeout for notification {}: {}ms", id, ms);

            // Use the stored tokio handle to spawn the timeout task
            // This allows spawning from non-tokio contexts (like zbus's executor)
            let handle = self.tokio_handle.spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(ms)).await;
                let _ = tx
                    .send(NotificationEvent::Closed {
                        id,
                        reason: CloseReason::Expired,
                    })
                    .await;
            });

            self.timeouts.insert(id, handle);
        } else {
            debug!(
                "Notification {} has no timeout (will persist)",
                notification.id
            );
        }
    }

    /// Get a notification by ID
    pub fn get(&self, id: u32) -> Option<&Notification> {
        self.notifications.get(&id)
    }

    /// Remove a notification from the store
    ///
    /// Returns the removed notification if it existed.
    pub fn remove(&mut self, id: u32) -> Option<Notification> {
        // Cancel timeout
        if let Some(handle) = self.timeouts.remove(&id) {
            handle.abort();
        }

        self.notifications.remove(&id)
    }

    /// Get all active notifications
    pub fn list(&self) -> Vec<&Notification> {
        self.notifications.values().collect()
    }

    /// Get count of active notifications
    pub fn count(&self) -> usize {
        self.notifications.len()
    }

    /// Check if a notification exists
    pub fn contains(&self, id: u32) -> bool {
        self.notifications.contains_key(&id)
    }

    /// Clear all notifications
    pub fn clear(&mut self) -> Vec<Notification> {
        // Cancel all timeouts
        for (_, handle) in self.timeouts.drain() {
            handle.abort();
        }

        self.notifications.drain().map(|(_, n)| n).collect()
    }
}

/// Thread-safe wrapper around NotificationStore
pub type SharedNotificationStore = Arc<Mutex<NotificationStore>>;

/// Create a new shared notification store
pub fn new_shared_store(
    event_tx: tokio::sync::mpsc::Sender<NotificationEvent>,
    urgency_timeouts: UrgencyTimeouts,
    tokio_handle: Handle,
) -> SharedNotificationStore {
    Arc::new(Mutex::new(NotificationStore::new(event_tx, urgency_timeouts, tokio_handle)))
}

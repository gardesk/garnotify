//! Popup manager for coordinating notification windows
//!
//! Manages the lifecycle of notification popups, X11 event handling,
//! and coordinates with the layout manager for positioning.

use anyhow::{Context, Result};
use gartk_x11::Connection;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use x11rb::protocol::Event as X11Event;

use crate::config::Config;
use crate::notification::{CloseReason, Notification};
use crate::ui::animation::{AnimationConfig, AnimationState};
use crate::ui::layout::{LayoutManager, NotificationPosition};
use crate::ui::popup::{calculate_notification_height, NotificationPopup};
use crate::ui::{PopupCommand, PopupEvent};

/// Manager for notification popup windows
pub struct PopupManager {
    /// X11 connection
    conn: Connection,
    /// Configuration
    config: Arc<Config>,
    /// Layout manager for positioning
    layout: LayoutManager,
    /// Active popup windows by notification ID
    popups: HashMap<u32, NotificationPopup>,
    /// Channel to send events back to daemon
    event_tx: mpsc::Sender<PopupEvent>,
    /// Window ID to notification ID mapping
    window_to_notification: HashMap<u32, u32>,
    /// Pending close reasons for popups with disappear animations
    pending_close: HashMap<u32, CloseReason>,
}

impl PopupManager {
    /// Create a new popup manager
    pub fn new(config: Arc<Config>, event_tx: mpsc::Sender<PopupEvent>) -> Result<Self> {
        let conn = Connection::connect(None).context("Failed to connect to X11")?;

        let position = NotificationPosition::from_str(&config.geometry.position);
        let layout = LayoutManager::new(
            &conn,
            config.geometry.width,
            position,
            config.geometry.max_visible,
            &config.general.monitor,
        );

        info!(
            "PopupManager initialized: position={}, width={}, max_visible={}",
            position, config.geometry.width, config.geometry.max_visible
        );

        Ok(Self {
            conn,
            config,
            layout,
            popups: HashMap::new(),
            event_tx,
            window_to_notification: HashMap::new(),
            pending_close: HashMap::new(),
        })
    }

    /// Handle a popup command
    pub fn handle_command(&mut self, cmd: PopupCommand) -> Result<()> {
        match cmd {
            PopupCommand::Show(notification) => {
                self.show_notification(notification)?;
            }
            PopupCommand::Update { id, notification } => {
                self.update_notification(id, notification)?;
            }
            PopupCommand::Close { id, reason } => {
                self.close_notification(id, reason)?;
            }
            PopupCommand::CloseAll => {
                self.close_all_notifications()?;
            }
        }
        Ok(())
    }

    /// Show a new notification popup
    fn show_notification(&mut self, notification: Notification) -> Result<()> {
        let id = notification.id;

        // Check if we have an available slot
        if !self.layout.has_available_slot() {
            warn!(
                "No available slots for notification {}, max_visible={}",
                id,
                self.config.geometry.max_visible
            );
            // Could queue it or close oldest, for now just warn
            return Ok(());
        }

        // Allocate a slot
        let slot = self.layout.allocate_slot(id).ok_or_else(|| {
            anyhow::anyhow!("Failed to allocate slot for notification {}", id)
        })?;

        // Calculate height based on content
        let height = calculate_notification_height(
            &notification,
            self.config.geometry.width,
            &self.config.appearance,
        );

        // Get geometry from layout
        let rect = self.layout.calculate_geometry(slot, height);

        // Create animation config from app config
        let animation_config = AnimationConfig::from_config(&self.config.animation);

        // Create popup window
        let mut popup = NotificationPopup::new(
            self.conn.clone(),
            notification,
            rect,
            &self.config.appearance,
            animation_config,
        )?;

        // Track window ID mapping
        let window_id = popup.window_id();
        self.window_to_notification.insert(window_id, id);

        // Show the popup
        popup.show()?;

        info!(
            "Showed notification {} in slot {} at ({}, {})",
            id, slot, rect.x, rect.y
        );

        // Store popup
        self.popups.insert(id, popup);

        Ok(())
    }

    /// Update an existing notification
    fn update_notification(&mut self, id: u32, notification: Notification) -> Result<()> {
        if let Some(popup) = self.popups.get_mut(&id) {
            popup.update_notification(notification)?;
            info!("Updated notification {}", id);
        } else {
            // Notification doesn't exist, show it as new
            self.show_notification(notification)?;
        }
        Ok(())
    }

    /// Close a notification popup (starts disappear animation)
    fn close_notification(&mut self, id: u32, reason: CloseReason) -> Result<()> {
        if let Some(popup) = self.popups.get_mut(&id) {
            // Check if already closing
            if self.pending_close.contains_key(&id) {
                return Ok(());
            }

            // Release the slot immediately so other notifications can use it
            self.layout.release_slot(id);

            // Start disappear animation
            popup.start_disappear();

            // Track the close reason
            self.pending_close.insert(id, reason);

            info!("Closing notification {}: reason={:?}", id, reason);
        }
        Ok(())
    }

    /// Immediately remove a popup (after animation completes)
    fn finish_close(&mut self, id: u32) {
        if let Some(popup) = self.popups.remove(&id) {
            // Remove window mapping
            self.window_to_notification.remove(&popup.window_id());

            // Get the close reason
            let reason = self.pending_close.remove(&id).unwrap_or(CloseReason::Closed);

            // Hide the popup
            let _ = popup.hide();

            info!("Finished closing notification {}: reason={:?}", id, reason);

            // Send event back to daemon
            let _ = self.event_tx.try_send(PopupEvent::Closed { id, reason });

            // Compact slots and trigger reflow animations for moved notifications
            self.reflow_stack();
        }
    }

    /// Compact the notification stack and animate notifications to new positions
    fn reflow_stack(&mut self) {
        let moves = self.layout.compact_slots();

        for (notification_id, _old_slot, new_slot) in moves {
            if let Some(popup) = self.popups.get_mut(&notification_id) {
                // Calculate new geometry for the new slot
                let height = popup.height();
                let new_rect = self.layout.calculate_geometry(new_slot, height);

                // Start reflow animation
                popup.start_reflow(new_rect.x, new_rect.y);

                debug!(
                    "Reflowing notification {} to slot {} at ({}, {})",
                    notification_id, new_slot, new_rect.x, new_rect.y
                );
            }
        }
    }

    /// Close all notification popups
    fn close_all_notifications(&mut self) -> Result<()> {
        let ids: Vec<u32> = self.popups.keys().copied().collect();
        for id in ids {
            self.close_notification(id, CloseReason::Closed)?;
        }
        Ok(())
    }

    /// Poll and handle X11 events (non-blocking)
    pub fn poll_events(&mut self) -> Result<()> {
        while let Ok(Some(event)) = self.conn.poll_event() {
            self.handle_x11_event(event)?;
        }
        Ok(())
    }

    /// Handle an X11 event
    fn handle_x11_event(&mut self, event: X11Event) -> Result<()> {
        match event {
            X11Event::Expose(e) => {
                // Re-render the exposed window
                if let Some(&id) = self.window_to_notification.get(&e.window) {
                    if let Some(popup) = self.popups.get_mut(&id) {
                        popup.render()?;
                        debug!("Re-rendered notification {} on expose", id);
                    }
                }
            }
            X11Event::ButtonPress(e) => {
                if let Some(&id) = self.window_to_notification.get(&e.event) {
                    // Check if click hit an action button
                    if let Some(popup) = self.popups.get(&id) {
                        // Convert root coordinates to window coordinates for action click check
                        let action_key = popup.check_action_click(e.event_x as i32, e.event_y as i32);

                        if let Some(key) = action_key {
                            info!("Action '{}' invoked on notification {}", key, id);
                            // Notify daemon of action
                            let _ = self.event_tx.try_send(PopupEvent::ActionInvoked { id, action_key: key });

                            // Close notification unless it's resident
                            if !popup.notification().hints.resident {
                                self.close_notification(id, CloseReason::Dismissed)?;
                            }
                        } else {
                            // Click outside action buttons - dismiss
                            info!("Click on notification {} - dismissing", id);
                            self.close_notification(id, CloseReason::Dismissed)?;
                            let _ = self.event_tx.try_send(PopupEvent::Dismissed(id));
                        }
                    }
                }
            }
            X11Event::EnterNotify(e) => {
                if let Some(&id) = self.window_to_notification.get(&e.event) {
                    if let Some(popup) = self.popups.get_mut(&id) {
                        popup.on_enter();
                    }
                }
            }
            X11Event::LeaveNotify(e) => {
                if let Some(&id) = self.window_to_notification.get(&e.event) {
                    if let Some(popup) = self.popups.get_mut(&id) {
                        popup.on_leave();
                        // Re-render and present to clear hover effects
                        if let Err(e) = popup.render_and_present() {
                            warn!("Failed to re-render on leave: {}", e);
                        }
                    }
                }
            }
            X11Event::MotionNotify(e) => {
                if let Some(&id) = self.window_to_notification.get(&e.event) {
                    if let Some(popup) = self.popups.get_mut(&id) {
                        // Update hover state, re-render and present if changed
                        if popup.on_motion(e.event_x as i32, e.event_y as i32) {
                            if let Err(err) = popup.render_and_present() {
                                warn!("Failed to re-render on motion: {}", err);
                            }
                        }
                    }
                }
            }
            _ => {
                // Ignore other events
            }
        }
        Ok(())
    }

    /// Get the X11 connection file descriptor for polling
    pub fn x11_fd(&self) -> std::os::unix::io::RawFd {
        use std::os::unix::io::AsRawFd;
        self.conn.inner().stream().as_raw_fd()
    }

    /// Check if any popups are currently shown
    pub fn has_popups(&self) -> bool {
        !self.popups.is_empty()
    }

    /// Get the number of active popups
    pub fn popup_count(&self) -> usize {
        self.popups.len()
    }

    /// Update all animations, returns true if any are still animating
    pub fn update_animations(&mut self) -> Result<bool> {
        use crate::ui::animation::AnimationState;

        let mut any_animating = false;
        let mut to_finish = Vec::new();

        for (&id, popup) in &mut self.popups {
            if popup.update_animation()? {
                any_animating = true;
            }

            // Check if disappear animation finished
            if popup.animation_state() == AnimationState::Hidden {
                to_finish.push(id);
            }
        }

        // Clean up popups that finished disappearing
        for id in to_finish {
            self.finish_close(id);
        }

        Ok(any_animating)
    }
}

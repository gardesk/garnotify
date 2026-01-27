//! Layout management for notification positioning and stacking
//!
//! Handles positioning notifications on screen and managing the stack
//! when multiple notifications are visible.

use gartk_core::Rect;
use gartk_x11::{Connection, Monitor, detect_monitors, primary_monitor, monitor_at_pointer};
use tracing::{debug, info, warn};

/// Screen position for notifications
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotificationPosition {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
    TopCenter,
    BottomCenter,
}

impl NotificationPosition {
    /// Parse from string (for config)
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "top-left" | "topleft" => Self::TopLeft,
            "top-right" | "topright" => Self::TopRight,
            "bottom-left" | "bottomleft" => Self::BottomLeft,
            "bottom-right" | "bottomright" => Self::BottomRight,
            "top-center" | "topcenter" => Self::TopCenter,
            "bottom-center" | "bottomcenter" => Self::BottomCenter,
            _ => Self::TopRight,
        }
    }
}

impl std::fmt::Display for NotificationPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomRight => "bottom-right",
            Self::TopCenter => "top-center",
            Self::BottomCenter => "bottom-center",
        };
        write!(f, "{}", s)
    }
}

/// Direction notifications stack
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackDirection {
    /// Stack downward (for top positions)
    Down,
    /// Stack upward (for bottom positions)
    Up,
}

/// Monitor selection mode
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorSelection {
    /// Use primary monitor
    Primary,
    /// Follow mouse pointer
    Mouse,
    /// Use specific monitor by name
    Named(String),
}

impl MonitorSelection {
    /// Parse from config string
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "primary" => Self::Primary,
            "mouse" | "pointer" => Self::Mouse,
            name => Self::Named(name.to_string()),
        }
    }
}

/// Layout manager for notification popups
pub struct LayoutManager {
    /// Current monitor bounds
    monitor: Monitor,
    /// Monitor selection mode
    monitor_selection: MonitorSelection,
    /// X11 connection for monitor updates
    conn: Connection,
    /// Notification width
    notification_width: u32,
    /// Margin from screen edges
    margin: u32,
    /// Gap between stacked notifications
    gap: u32,
    /// Position anchor
    position: NotificationPosition,
    /// Maximum visible notifications
    max_visible: u32,
    /// Currently occupied slots (notification IDs)
    slots: Vec<Option<u32>>,
}

impl LayoutManager {
    /// Create a new layout manager
    pub fn new(
        conn: &Connection,
        width: u32,
        position: NotificationPosition,
        max_visible: u32,
        monitor_config: &str,
    ) -> Self {
        let monitor_selection = MonitorSelection::from_str(monitor_config);
        let monitor = Self::get_target_monitor_static(conn, &monitor_selection);

        debug!(
            "LayoutManager: monitor='{}' ({}x{} at {},{}) position={}, width={}, max={}",
            monitor.name,
            monitor.rect.width,
            monitor.rect.height,
            monitor.rect.x,
            monitor.rect.y,
            position,
            width,
            max_visible
        );

        Self {
            monitor,
            monitor_selection,
            conn: conn.clone(),
            notification_width: width,
            margin: 16,
            gap: 8,
            position,
            max_visible,
            slots: vec![None; max_visible as usize],
        }
    }

    /// Get target monitor based on selection mode (static version for construction)
    fn get_target_monitor_static(conn: &Connection, selection: &MonitorSelection) -> Monitor {
        match selection {
            MonitorSelection::Primary => {
                primary_monitor(conn).unwrap_or_else(|e| {
                    warn!("Failed to get primary monitor: {}, using fallback", e);
                    Self::fallback_monitor(conn)
                })
            }
            MonitorSelection::Mouse => {
                monitor_at_pointer(conn).unwrap_or_else(|e| {
                    warn!("Failed to get monitor at pointer: {}, using primary", e);
                    primary_monitor(conn).unwrap_or_else(|_| Self::fallback_monitor(conn))
                })
            }
            MonitorSelection::Named(name) => {
                detect_monitors(conn)
                    .ok()
                    .and_then(|monitors| {
                        monitors.into_iter().find(|m| m.name == *name)
                    })
                    .unwrap_or_else(|| {
                        warn!("Monitor '{}' not found, using primary", name);
                        primary_monitor(conn).unwrap_or_else(|_| Self::fallback_monitor(conn))
                    })
            }
        }
    }

    /// Create a fallback monitor from screen dimensions
    fn fallback_monitor(conn: &Connection) -> Monitor {
        Monitor {
            name: "default".to_string(),
            rect: Rect::new(0, 0, conn.screen_width() as u32, conn.screen_height() as u32),
            primary: true,
            width_mm: 0,
            height_mm: 0,
        }
    }

    /// Update monitor if using mouse-follow mode
    pub fn update_monitor_if_needed(&mut self) {
        if self.monitor_selection == MonitorSelection::Mouse {
            let new_monitor = Self::get_target_monitor_static(&self.conn, &self.monitor_selection);
            if new_monitor.name != self.monitor.name {
                info!("Monitor changed: {} -> {}", self.monitor.name, new_monitor.name);
                self.monitor = new_monitor;
            }
        }
    }

    /// Get current monitor info
    pub fn monitor(&self) -> &Monitor {
        &self.monitor
    }

    /// Get the stack direction based on position
    pub fn stack_direction(&self) -> StackDirection {
        match self.position {
            NotificationPosition::TopLeft
            | NotificationPosition::TopRight
            | NotificationPosition::TopCenter => StackDirection::Down,
            NotificationPosition::BottomLeft
            | NotificationPosition::BottomRight
            | NotificationPosition::BottomCenter => StackDirection::Up,
        }
    }

    /// Allocate a slot for a notification, returns the slot index
    pub fn allocate_slot(&mut self, notification_id: u32) -> Option<usize> {
        // Find first empty slot
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(notification_id);
                debug!("Allocated slot {} for notification {}", i, notification_id);
                return Some(i);
            }
        }
        debug!(
            "No slots available for notification {} (max={})",
            notification_id, self.max_visible
        );
        None
    }

    /// Release a slot
    pub fn release_slot(&mut self, notification_id: u32) {
        for slot in &mut self.slots {
            if *slot == Some(notification_id) {
                *slot = None;
                debug!("Released slot for notification {}", notification_id);
                return;
            }
        }
    }

    /// Compact slots after a notification is removed.
    /// Returns a list of (notification_id, old_slot, new_slot) for notifications that moved.
    pub fn compact_slots(&mut self) -> Vec<(u32, usize, usize)> {
        let mut moves = Vec::new();
        let mut write_idx = 0;

        for read_idx in 0..self.slots.len() {
            if let Some(notification_id) = self.slots[read_idx] {
                if write_idx != read_idx {
                    // This notification needs to move
                    moves.push((notification_id, read_idx, write_idx));
                    self.slots[write_idx] = Some(notification_id);
                    self.slots[read_idx] = None;
                }
                write_idx += 1;
            }
        }

        if !moves.is_empty() {
            debug!("Compacted slots: {:?}", moves);
        }

        moves
    }

    /// Get the slot index for a notification
    pub fn get_slot(&self, notification_id: u32) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| *s == Some(notification_id))
    }

    /// Calculate notification geometry for a given slot and height
    pub fn calculate_geometry(&self, slot: usize, height: u32) -> Rect {
        let x = self.calculate_x();
        let y = self.calculate_y(slot, height);

        Rect::new(x, y, self.notification_width, height)
    }

    /// Calculate X position based on monitor position
    fn calculate_x(&self) -> i32 {
        let mon = &self.monitor.rect;
        match self.position {
            NotificationPosition::TopLeft | NotificationPosition::BottomLeft => {
                mon.x + self.margin as i32
            }
            NotificationPosition::TopRight | NotificationPosition::BottomRight => {
                mon.x + mon.width as i32 - self.notification_width as i32 - self.margin as i32
            }
            NotificationPosition::TopCenter | NotificationPosition::BottomCenter => {
                mon.x + (mon.width as i32 - self.notification_width as i32) / 2
            }
        }
    }

    /// Calculate Y position based on slot and stack direction
    fn calculate_y(&self, slot: usize, height: u32) -> i32 {
        let mon = &self.monitor.rect;
        // Calculate offset from edge based on slot position
        let slot_offset = self.calculate_slot_offset(slot, height);

        match self.stack_direction() {
            StackDirection::Down => mon.y + self.margin as i32 + slot_offset,
            StackDirection::Up => {
                mon.y + mon.height as i32 - height as i32 - self.margin as i32 - slot_offset
            }
        }
    }

    /// Calculate cumulative offset for a slot (accounts for varying heights)
    fn calculate_slot_offset(&self, slot: usize, height: u32) -> i32 {
        // For simplicity, assume uniform height for now
        // In a full implementation, we'd track actual heights of each slot
        (slot as i32) * (height as i32 + self.gap as i32)
    }

    /// Get the notification width
    pub fn notification_width(&self) -> u32 {
        self.notification_width
    }

    /// Check if there are available slots
    pub fn has_available_slot(&self) -> bool {
        self.slots.iter().any(|s| s.is_none())
    }

    /// Get number of active notifications
    pub fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}

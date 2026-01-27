//! Layout management for notification positioning and stacking
//!
//! Handles positioning notifications on screen and managing the stack
//! when multiple notifications are visible.

use gartk_core::Rect;
use gartk_x11::Connection;
use tracing::debug;

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

/// Layout manager for notification popups
pub struct LayoutManager {
    /// Screen width
    screen_width: u32,
    /// Screen height
    screen_height: u32,
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
    pub fn new(conn: &Connection, width: u32, position: NotificationPosition, max_visible: u32) -> Self {
        let screen_width = conn.screen_width() as u32;
        let screen_height = conn.screen_height() as u32;

        debug!(
            "LayoutManager: screen={}x{}, position={}, width={}, max={}",
            screen_width, screen_height, position, width, max_visible
        );

        Self {
            screen_width,
            screen_height,
            notification_width: width,
            margin: 16,
            gap: 8,
            position,
            max_visible,
            slots: vec![None; max_visible as usize],
        }
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

    /// Calculate X position based on screen position
    fn calculate_x(&self) -> i32 {
        match self.position {
            NotificationPosition::TopLeft | NotificationPosition::BottomLeft => self.margin as i32,
            NotificationPosition::TopRight | NotificationPosition::BottomRight => {
                self.screen_width as i32 - self.notification_width as i32 - self.margin as i32
            }
            NotificationPosition::TopCenter | NotificationPosition::BottomCenter => {
                (self.screen_width as i32 - self.notification_width as i32) / 2
            }
        }
    }

    /// Calculate Y position based on slot and stack direction
    fn calculate_y(&self, slot: usize, height: u32) -> i32 {
        // Calculate offset from edge based on slot position
        let slot_offset = self.calculate_slot_offset(slot, height);

        match self.stack_direction() {
            StackDirection::Down => self.margin as i32 + slot_offset,
            StackDirection::Up => {
                self.screen_height as i32 - height as i32 - self.margin as i32 - slot_offset
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

//! Core notification types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

/// Notification urgency level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    Low = 0,
    #[default]
    Normal = 1,
    Critical = 2,
}

impl Urgency {
    /// Convert from D-Bus byte value
    pub fn from_byte(value: u8) -> Self {
        match value {
            0 => Urgency::Low,
            2 => Urgency::Critical,
            _ => Urgency::Normal,
        }
    }

    /// Get string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            Urgency::Low => "low",
            Urgency::Normal => "normal",
            Urgency::Critical => "critical",
        }
    }
}

impl std::fmt::Display for Urgency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Raw image data from D-Bus hints
#[derive(Debug, Clone)]
pub struct ImageData {
    pub width: i32,
    pub height: i32,
    pub rowstride: i32,
    pub has_alpha: bool,
    pub bits_per_sample: i32,
    pub channels: i32,
    pub data: Vec<u8>,
}

/// Notification action button
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    /// Action identifier (sent back in ActionInvoked signal)
    pub key: String,
    /// Human-readable label
    pub label: String,
}

impl Action {
    /// Parse actions from D-Bus array (alternating key, label pairs)
    pub fn parse_from_dbus(actions: &[String]) -> Vec<Self> {
        actions
            .chunks(2)
            .filter_map(|chunk| {
                if chunk.len() == 2 {
                    Some(Action {
                        key: chunk[0].clone(),
                        label: chunk[1].clone(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Parsed notification hints
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Hints {
    /// Urgency level (0=low, 1=normal, 2=critical)
    pub urgency: Urgency,
    /// Notification category (e.g., "email.arrived", "im.received")
    pub category: Option<String>,
    /// Desktop entry name of the calling application
    pub desktop_entry: Option<String>,
    /// Path to image file
    pub image_path: Option<String>,
    /// Raw image data (not persisted - too large)
    #[serde(skip)]
    pub image_data: Option<ImageData>,
    /// Path to sound file to play
    pub sound_file: Option<String>,
    /// Named sound from sound theme
    pub sound_name: Option<String>,
    /// Suppress notification sound
    pub suppress_sound: bool,
    /// Notification is transient (skip persistence)
    pub transient: bool,
    /// Keep notification after action invoked
    pub resident: bool,
    /// X coordinate for positioning
    pub x: Option<i32>,
    /// Y coordinate for positioning
    pub y: Option<i32>,
    /// Interpret action identifiers as icon names
    pub action_icons: bool,
}

impl Hints {
    /// Parse hints from D-Bus dictionary
    pub fn parse_from_dbus(hints: &HashMap<String, zbus::zvariant::Value<'_>>) -> Self {
        let mut result = Hints::default();

        // Urgency
        if let Some(v) = hints.get("urgency") {
            if let Ok(u) = v.downcast_ref::<u8>() {
                result.urgency = Urgency::from_byte(u);
            }
        }

        // Category
        if let Some(v) = hints.get("category") {
            if let Ok(s) = v.downcast_ref::<&str>() {
                result.category = Some(s.to_string());
            }
        }

        // Desktop entry
        if let Some(v) = hints.get("desktop-entry") {
            if let Ok(s) = v.downcast_ref::<&str>() {
                result.desktop_entry = Some(s.to_string());
            }
        }

        // Image path (try both hyphen and underscore versions)
        for key in &["image-path", "image_path"] {
            if let Some(v) = hints.get(*key) {
                if let Ok(s) = v.downcast_ref::<&str>() {
                    result.image_path = Some(s.to_string());
                    break;
                }
            }
        }

        // Sound file
        if let Some(v) = hints.get("sound-file") {
            if let Ok(s) = v.downcast_ref::<&str>() {
                result.sound_file = Some(s.to_string());
            }
        }

        // Sound name
        if let Some(v) = hints.get("sound-name") {
            if let Ok(s) = v.downcast_ref::<&str>() {
                result.sound_name = Some(s.to_string());
            }
        }

        // Suppress sound
        if let Some(v) = hints.get("suppress-sound") {
            if let Ok(b) = v.downcast_ref::<bool>() {
                result.suppress_sound = b;
            }
        }

        // Transient
        if let Some(v) = hints.get("transient") {
            if let Ok(b) = v.downcast_ref::<bool>() {
                result.transient = b;
            }
        }

        // Resident
        if let Some(v) = hints.get("resident") {
            if let Ok(b) = v.downcast_ref::<bool>() {
                result.resident = b;
            }
        }

        // X position
        if let Some(v) = hints.get("x") {
            if let Ok(x) = v.downcast_ref::<i32>() {
                result.x = Some(x);
            }
        }

        // Y position
        if let Some(v) = hints.get("y") {
            if let Ok(y) = v.downcast_ref::<i32>() {
                result.y = Some(y);
            }
        }

        // Action icons
        if let Some(v) = hints.get("action-icons") {
            if let Ok(b) = v.downcast_ref::<bool>() {
                result.action_icons = b;
            }
        }

        result
    }
}

/// A desktop notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// Unique notification ID
    pub id: u32,
    /// Application name
    pub app_name: String,
    /// ID of notification this replaces (0 if none)
    pub replaces_id: u32,
    /// Icon name or path
    pub app_icon: String,
    /// Notification title/summary
    pub summary: String,
    /// Notification body text
    pub body: String,
    /// Action buttons
    pub actions: Vec<Action>,
    /// Parsed hints
    pub hints: Hints,
    /// Expiration timeout in milliseconds (-1 = default, 0 = never)
    pub expire_timeout: i32,
    /// When the notification was created (not persisted)
    #[serde(skip, default = "Instant::now")]
    pub created_at: Instant,
}

impl Notification {
    /// Create a new notification
    pub fn new(
        id: u32,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<Action>,
        hints: Hints,
        expire_timeout: i32,
    ) -> Self {
        Self {
            id,
            app_name,
            replaces_id,
            app_icon,
            summary,
            body,
            actions,
            hints,
            expire_timeout,
            created_at: Instant::now(),
        }
    }

    /// Get the effective timeout in milliseconds
    ///
    /// Returns None if the notification should never expire
    pub fn effective_timeout(&self, _default_timeout: i32, urgency_timeouts: &UrgencyTimeouts) -> Option<u64> {
        let timeout = if self.expire_timeout == -1 {
            // Use server default based on urgency
            match self.hints.urgency {
                Urgency::Low => urgency_timeouts.low,
                Urgency::Normal => urgency_timeouts.normal,
                Urgency::Critical => urgency_timeouts.critical,
            }
        } else {
            self.expire_timeout
        };

        // 0 means never expire
        if timeout <= 0 {
            None
        } else {
            Some(timeout as u64)
        }
    }
}

impl Default for Notification {
    fn default() -> Self {
        Self {
            id: 0,
            app_name: String::new(),
            replaces_id: 0,
            app_icon: String::new(),
            summary: String::new(),
            body: String::new(),
            actions: Vec::new(),
            hints: Hints::default(),
            expire_timeout: -1,
            created_at: Instant::now(),
        }
    }
}

/// Timeout configuration per urgency level
#[derive(Debug, Clone)]
pub struct UrgencyTimeouts {
    pub low: i32,
    pub normal: i32,
    pub critical: i32,
}

impl Default for UrgencyTimeouts {
    fn default() -> Self {
        Self {
            low: 10000,
            normal: 5000,
            critical: 0, // Never expire by default
        }
    }
}

/// Reason a notification was closed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CloseReason {
    /// The notification expired
    Expired = 1,
    /// The notification was dismissed by the user
    Dismissed = 2,
    /// The notification was closed by CloseNotification call
    Closed = 3,
    /// Undefined/reserved reason
    Undefined = 4,
}

impl CloseReason {
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

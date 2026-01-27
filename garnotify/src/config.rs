//! Configuration loading and management for garnotify

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::rules::Rule;

/// Get the default configuration file path
fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("garnotify")
        .join("config.toml")
}

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub geometry: GeometryConfig,
    pub timeouts: TimeoutConfig,
    pub appearance: AppearanceConfig,
    pub animation: AnimationConfig,
    pub history: HistoryConfig,
    /// Notification rules (use [[rules]] in TOML)
    #[serde(rename = "rules", default)]
    pub rules: Vec<Rule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            geometry: GeometryConfig::default(),
            timeouts: TimeoutConfig::default(),
            appearance: AppearanceConfig::default(),
            animation: AnimationConfig::default(),
            history: HistoryConfig::default(),
            rules: Vec::new(),
        }
    }
}

/// General daemon settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Monitor to show notifications on: "primary", "mouse", or monitor name
    pub monitor: String,
    /// Follow mouse to determine monitor
    pub follow_mouse: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            monitor: "primary".into(),
            follow_mouse: false,
        }
    }
}

/// Geometry and positioning settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeometryConfig {
    /// Position anchor: top-right, top-left, top-center, bottom-right, bottom-left, bottom-center
    pub position: String,
    /// Notification width in pixels
    pub width: u32,
    /// Maximum height per notification (0 = unlimited)
    pub max_height: u32,
    /// Horizontal offset from screen edge
    pub offset_x: i32,
    /// Vertical offset from screen edge
    pub offset_y: i32,
    /// Gap between stacked notifications
    pub gap: i32,
    /// Maximum number of visible notifications (0 = unlimited)
    pub max_visible: u32,
}

impl Default for GeometryConfig {
    fn default() -> Self {
        Self {
            position: "top-right".into(),
            width: 350,
            max_height: 150,
            offset_x: 20,
            offset_y: 40,
            gap: 10,
            max_visible: 5,
        }
    }
}

/// Timeout settings per urgency level
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimeoutConfig {
    /// Default timeout in milliseconds (-1 = server default 5000ms)
    pub default: i32,
    /// Timeout for low urgency notifications
    pub low: i32,
    /// Timeout for normal urgency notifications
    pub normal: i32,
    /// Timeout for critical urgency notifications (0 = never expire)
    pub critical: i32,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            default: 5000,
            low: 10000,
            normal: 5000,
            critical: 0,
        }
    }
}

/// Appearance settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    /// Font specification (Pango format)
    pub font: String,
    /// Title/summary font
    pub title_font: String,
    /// Icon size in pixels
    pub icon_size: u32,
    /// Padding inside notification
    pub padding: u32,
    /// Corner radius
    pub corner_radius: u32,
    /// Border width
    pub border_width: u32,
    /// Enable Pango markup in body
    pub markup_enabled: bool,
    /// Color scheme
    pub colors: ColorConfig,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            font: "Sans 11".into(),
            title_font: "Sans Bold 12".into(),
            icon_size: 48,
            padding: 12,
            corner_radius: 8,
            border_width: 2,
            markup_enabled: true,
            colors: ColorConfig::default(),
        }
    }
}

/// Color configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorConfig {
    /// Default background color
    pub background: String,
    /// Default foreground (text) color
    pub foreground: String,
    /// Default border color
    pub border: String,

    /// Low urgency background
    pub low_background: String,
    /// Low urgency foreground
    pub low_foreground: String,
    /// Low urgency border
    pub low_border: String,

    /// Critical urgency background
    pub critical_background: String,
    /// Critical urgency foreground
    pub critical_foreground: String,
    /// Critical urgency border
    pub critical_border: String,
}

impl Default for ColorConfig {
    fn default() -> Self {
        // Catppuccin-inspired colors
        Self {
            background: "#1e1e2e".into(),
            foreground: "#cdd6f4".into(),
            border: "#45475a".into(),

            low_background: "#1e1e2e".into(),
            low_foreground: "#6c7086".into(),
            low_border: "#45475a".into(),

            critical_background: "#f38ba8".into(),
            critical_foreground: "#1e1e2e".into(),
            critical_border: "#f38ba8".into(),
        }
    }
}

/// Animation settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimationConfig {
    /// Enable animations
    pub enabled: bool,
    /// Fade-in duration in milliseconds
    pub fade_in: u32,
    /// Fade-out duration in milliseconds
    pub fade_out: u32,
    /// Slide direction: up, down, left, right, none
    pub slide: String,
    /// Slide distance in pixels
    pub slide_distance: u32,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fade_in: 150,
            fade_out: 150,
            slide: "down".into(),
            slide_distance: 20,
        }
    }
}

/// History settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// Maximum notifications to keep in history
    pub max_length: usize,
    /// Persist history to disk
    pub persist: bool,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_length: 100,
            persist: false,
        }
    }
}

/// Load configuration from file
pub fn load(path: Option<&str>) -> Result<Config> {
    let config_path = path.map(PathBuf::from).unwrap_or_else(config_path);

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", config_path.display()))?;

        Ok(config)
    } else {
        // Config file doesn't exist, use defaults
        Ok(Config::default())
    }
}

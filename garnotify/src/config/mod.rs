//! Configuration loading and management for garnotify

pub mod lua;

use anyhow::{Context, Result};
use gartk_core::Color;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::path::PathBuf;
use tracing::{debug, info, warn};

use crate::rules::Rule;

/// Serialize Color as hex string
fn serialize_color<S: Serializer>(color: &Color, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&color.to_hex())
}

/// Deserialize Color from any color string (hex, rgb, rgba, named)
fn deserialize_color<'de, D: Deserializer<'de>>(d: D) -> Result<Color, D::Error> {
    let s: String = Deserialize::deserialize(d)?;
    Color::parse(&s).map_err(de::Error::custom)
}

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

/// Color configuration using gartk Color type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorConfig {
    /// Default background color
    #[serde(serialize_with = "serialize_color", deserialize_with = "deserialize_color")]
    pub background: Color,
    /// Default foreground (text) color
    #[serde(serialize_with = "serialize_color", deserialize_with = "deserialize_color")]
    pub foreground: Color,
    /// Default border color
    #[serde(serialize_with = "serialize_color", deserialize_with = "deserialize_color")]
    pub border: Color,

    /// Low urgency background
    #[serde(serialize_with = "serialize_color", deserialize_with = "deserialize_color")]
    pub low_background: Color,
    /// Low urgency foreground
    #[serde(serialize_with = "serialize_color", deserialize_with = "deserialize_color")]
    pub low_foreground: Color,
    /// Low urgency border
    #[serde(serialize_with = "serialize_color", deserialize_with = "deserialize_color")]
    pub low_border: Color,

    /// Critical urgency background
    #[serde(serialize_with = "serialize_color", deserialize_with = "deserialize_color")]
    pub critical_background: Color,
    /// Critical urgency foreground
    #[serde(serialize_with = "serialize_color", deserialize_with = "deserialize_color")]
    pub critical_foreground: Color,
    /// Critical urgency border
    #[serde(serialize_with = "serialize_color", deserialize_with = "deserialize_color")]
    pub critical_border: Color,
}

impl Default for ColorConfig {
    fn default() -> Self {
        // Catppuccin-inspired colors (matching gartk Theme::dark())
        Self {
            background: Color::from_hex("#1e1e2e").unwrap(),
            foreground: Color::from_hex("#cdd6f4").unwrap(),
            border: Color::from_hex("#45475a").unwrap(),

            low_background: Color::from_hex("#1e1e2e").unwrap(),
            low_foreground: Color::from_hex("#6c7086").unwrap(),
            low_border: Color::from_hex("#45475a").unwrap(),

            critical_background: Color::from_hex("#f38ba8").unwrap(),
            critical_foreground: Color::from_hex("#1e1e2e").unwrap(),
            critical_border: Color::from_hex("#f38ba8").unwrap(),
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
            persist: true,
        }
    }
}

/// Load configuration from file
///
/// Priority:
/// 1. Explicit path (if provided)
/// 2. gar's Lua config: ~/.config/gar/init.lua (gar.notification table)
/// 3. TOML config: ~/.config/garnotify/config.toml
/// 4. Defaults
pub fn load(path: Option<&str>) -> Result<Config> {
    // If explicit path provided, use it
    if let Some(p) = path {
        let config_path = PathBuf::from(p);
        if config_path.exists() {
            return load_toml(&config_path);
        }
    }

    // Try gar's Lua config first (gar ecosystem integration)
    let lua_path = lua::gar_config_path();
    if lua_path.exists() {
        match lua::load_from_lua(&lua_path) {
            Ok(Some(config)) => {
                info!("Loaded config from gar's init.lua");
                return Ok(config);
            }
            Ok(None) => {
                debug!("No gar.notification table in init.lua");
            }
            Err(e) => {
                warn!("Failed to load Lua config: {}", e);
            }
        }
    }

    // Try TOML config
    let toml_path = config_path();
    if toml_path.exists() {
        return load_toml(&toml_path);
    }

    // Use defaults
    debug!("Using default configuration");
    Ok(Config::default())
}

/// Load config from TOML file
fn load_toml(path: &PathBuf) -> Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: Config = toml::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Loaded config from {}", path.display());
    Ok(config)
}

//! Lua configuration loader for gar ecosystem integration
//!
//! Reads `gar.notification` table from `~/.config/gar/init.lua`

use anyhow::{anyhow, Result};
use gartk_core::Color;
use mlua::{Lua, Table, Value};
use std::path::Path;
use tracing::{debug, info, warn};

use super::{ColorConfig, Config};

/// Parse a color string from Lua, returning default on failure
fn parse_color(s: &str, default: Color) -> Color {
    Color::parse(s).unwrap_or(default)
}

/// Convert mlua Error to anyhow Error
fn lua_err(e: mlua::Error) -> anyhow::Error {
    anyhow!("Lua error: {}", e)
}

/// Setup stub functions for gar's Lua API
/// These no-ops allow garnotify to execute gar's init.lua without errors
fn setup_gar_stubs(lua: &Lua, gar: &Table) -> Result<()> {
    // gar.set(key, value) - configuration setter
    let set_fn = lua
        .create_function(|_, (_key, _value): (String, Value)| Ok(()))
        .map_err(lua_err)?;
    gar.set("set", set_fn).map_err(lua_err)?;

    // gar.bind(keyspec, callback) - keybinding
    let bind_fn = lua
        .create_function(|_, (_keyspec, _callback): (String, Value)| Ok(()))
        .map_err(lua_err)?;
    gar.set("bind", bind_fn).map_err(lua_err)?;

    // gar.exec(cmd) - execute command
    let exec_fn = lua
        .create_function(|_, _cmd: String| Ok(()))
        .map_err(lua_err)?;
    gar.set("exec", exec_fn).map_err(lua_err)?;

    // gar.exec_once(cmd) - execute command once at startup
    let exec_once_fn = lua
        .create_function(|_, _cmd: String| Ok(()))
        .map_err(lua_err)?;
    gar.set("exec_once", exec_once_fn).map_err(lua_err)?;

    // gar.rule(match, actions) - window rules
    let rule_fn = lua
        .create_function(|_, (_match_table, _actions_table): (Value, Value)| Ok(()))
        .map_err(lua_err)?;
    gar.set("rule", rule_fn).map_err(lua_err)?;

    // gar.picom_rule(config) - picom window rules
    let picom_rule_fn = lua
        .create_function(|_, _config: Value| Ok(()))
        .map_err(lua_err)?;
    gar.set("picom_rule", picom_rule_fn).map_err(lua_err)?;

    // Window management stubs
    for name in &[
        "focus",
        "swap",
        "close",
        "toggle_floating",
        "equalize",
        "reload",
        "exit",
        "focus_monitor",
        "move_to_monitor",
    ] {
        let stub = lua.create_function(|_, _: Value| Ok(())).map_err(lua_err)?;
        gar.set(*name, stub).map_err(lua_err)?;
    }

    // Workspace stubs
    let workspace_fn = lua
        .create_function(|_, _n: Value| Ok(()))
        .map_err(lua_err)?;
    gar.set("workspace", workspace_fn).map_err(lua_err)?;

    let workspace_next_fn = lua.create_function(|_, ()| Ok(())).map_err(lua_err)?;
    gar.set("workspace_next", workspace_next_fn)
        .map_err(lua_err)?;

    let workspace_prev_fn = lua.create_function(|_, ()| Ok(())).map_err(lua_err)?;
    gar.set("workspace_prev", workspace_prev_fn)
        .map_err(lua_err)?;

    let move_fn = lua
        .create_function(|_, _n: Value| Ok(()))
        .map_err(lua_err)?;
    gar.set("move", move_fn).map_err(lua_err)?;

    let move_to_workspace_fn = lua
        .create_function(|_, _n: Value| Ok(()))
        .map_err(lua_err)?;
    gar.set("move_to_workspace", move_to_workspace_fn)
        .map_err(lua_err)?;

    let resize_fn = lua
        .create_function(|_, (_direction, _amount): (Value, Value)| Ok(()))
        .map_err(lua_err)?;
    gar.set("resize", resize_fn).map_err(lua_err)?;

    Ok(())
}

/// Load configuration from gar's init.lua file
pub fn load_from_lua<P: AsRef<Path>>(path: P) -> Result<Option<Config>> {
    let path = path.as_ref();

    if !path.exists() {
        debug!("Lua config file not found: {}", path.display());
        return Ok(None);
    }

    info!("Loading config from {}", path.display());

    let lua = Lua::new();
    let content = std::fs::read_to_string(path)?;

    // Create the gar global table with stub functions
    let gar = lua.create_table().map_err(lua_err)?;
    setup_gar_stubs(&lua, &gar)?;
    lua.globals().set("gar", gar).map_err(lua_err)?;

    // Execute the config file
    lua.load(&content)
        .set_name(path.to_string_lossy())
        .exec()
        .map_err(|e| anyhow!("Failed to execute {}: {}", path.display(), e))?;

    // Try to get gar.notification
    let gar: Table = lua.globals().get("gar").map_err(lua_err)?;
    let notification_value: Value = gar.get("notification").map_err(lua_err)?;

    match notification_value {
        Value::Table(table) => {
            let config = parse_notification_config(&table)?;
            Ok(Some(config))
        }
        Value::Nil => {
            debug!("No gar.notification table found in config");
            Ok(None)
        }
        _ => {
            warn!("gar.notification is not a table");
            Ok(None)
        }
    }
}

/// Parse the gar.notification table into Config
fn parse_notification_config(table: &Table) -> Result<Config> {
    let mut config = Config::default();

    // General settings
    if let Ok(monitor) = table.get::<String>("monitor") {
        config.general.monitor = monitor;
    }
    if let Ok(follow_mouse) = table.get::<bool>("follow_mouse") {
        config.general.follow_mouse = follow_mouse;
    }

    // Geometry settings
    if let Ok(position) = table.get::<String>("position") {
        config.geometry.position = position;
    }
    if let Ok(width) = table.get::<u32>("width") {
        config.geometry.width = width;
    }
    if let Ok(max_height) = table.get::<u32>("max_height") {
        config.geometry.max_height = max_height;
    }
    if let Ok(offset_x) = table.get::<i32>("offset_x") {
        config.geometry.offset_x = offset_x;
    }
    if let Ok(offset_y) = table.get::<i32>("offset_y") {
        config.geometry.offset_y = offset_y;
    }
    if let Ok(gap) = table.get::<i32>("gap") {
        config.geometry.gap = gap;
    }
    if let Ok(max_visible) = table.get::<u32>("max_visible") {
        config.geometry.max_visible = max_visible;
    }

    // Timeout settings
    if let Ok(timeout) = table.get::<i32>("timeout") {
        // Single timeout applies to normal urgency
        config.timeouts.normal = timeout;
    }
    if let Ok(timeouts) = table.get::<Table>("timeouts") {
        if let Ok(low) = timeouts.get::<i32>("low") {
            config.timeouts.low = low;
        }
        if let Ok(normal) = timeouts.get::<i32>("normal") {
            config.timeouts.normal = normal;
        }
        if let Ok(critical) = timeouts.get::<i32>("critical") {
            config.timeouts.critical = critical;
        }
    }

    // Appearance settings
    if let Ok(font) = table.get::<String>("font") {
        config.appearance.font = font;
    }
    if let Ok(title_font) = table.get::<String>("title_font") {
        config.appearance.title_font = title_font;
    }
    if let Ok(icon_size) = table.get::<u32>("icon_size") {
        config.appearance.icon_size = icon_size;
    }
    if let Ok(padding) = table.get::<u32>("padding") {
        config.appearance.padding = padding;
    }
    if let Ok(corner_radius) = table.get::<u32>("corner_radius") {
        config.appearance.corner_radius = corner_radius;
    }
    if let Ok(border_width) = table.get::<u32>("border_width") {
        config.appearance.border_width = border_width;
    }

    // Colors - can be flat or nested
    if let Ok(background) = table.get::<String>("background") {
        let color = parse_color(&background, config.appearance.colors.background);
        config.appearance.colors.background = color;
        config.appearance.colors.low_background = color;
    }
    if let Ok(foreground) = table.get::<String>("foreground") {
        let color = parse_color(&foreground, config.appearance.colors.foreground);
        config.appearance.colors.foreground = color;
        config.appearance.colors.low_foreground = color;
    }
    if let Ok(border) = table.get::<String>("border") {
        let color = parse_color(&border, config.appearance.colors.border);
        config.appearance.colors.border = color;
        config.appearance.colors.low_border = color;
    }

    // Nested colors table
    if let Ok(colors) = table.get::<Table>("colors") {
        parse_colors(&colors, &mut config.appearance.colors);
    }

    // Animation settings
    if let Ok(animations) = table.get::<Table>("animation") {
        if let Ok(enabled) = animations.get::<bool>("enabled") {
            config.animation.enabled = enabled;
        }
        if let Ok(fade_in) = animations.get::<u32>("fade_in") {
            config.animation.fade_in = fade_in;
        }
        if let Ok(fade_out) = animations.get::<u32>("fade_out") {
            config.animation.fade_out = fade_out;
        }
        if let Ok(slide) = animations.get::<String>("slide") {
            config.animation.slide = slide;
        }
        if let Ok(slide_distance) = animations.get::<u32>("slide_distance") {
            config.animation.slide_distance = slide_distance;
        }
    }

    // History settings
    if let Ok(history) = table.get::<Table>("history") {
        if let Ok(max_length) = history.get::<usize>("max_length") {
            config.history.max_length = max_length;
        }
        if let Ok(persist) = history.get::<bool>("persist") {
            config.history.persist = persist;
        }
    }

    debug!(
        "Parsed notification config: position={}, width={}",
        config.geometry.position, config.geometry.width
    );
    Ok(config)
}

/// Parse colors table
fn parse_colors(table: &Table, colors: &mut ColorConfig) {
    if let Ok(bg) = table.get::<String>("background") {
        colors.background = parse_color(&bg, colors.background);
    }
    if let Ok(fg) = table.get::<String>("foreground") {
        colors.foreground = parse_color(&fg, colors.foreground);
    }
    if let Ok(border) = table.get::<String>("border") {
        colors.border = parse_color(&border, colors.border);
    }

    // Low urgency
    if let Ok(low_bg) = table.get::<String>("low_background") {
        colors.low_background = parse_color(&low_bg, colors.low_background);
    }
    if let Ok(low_fg) = table.get::<String>("low_foreground") {
        colors.low_foreground = parse_color(&low_fg, colors.low_foreground);
    }
    if let Ok(low_border) = table.get::<String>("low_border") {
        colors.low_border = parse_color(&low_border, colors.low_border);
    }

    // Critical urgency
    if let Ok(crit_bg) = table.get::<String>("critical_background") {
        colors.critical_background = parse_color(&crit_bg, colors.critical_background);
    }
    if let Ok(crit_fg) = table.get::<String>("critical_foreground") {
        colors.critical_foreground = parse_color(&crit_fg, colors.critical_foreground);
    }
    if let Ok(crit_border) = table.get::<String>("critical_border") {
        colors.critical_border = parse_color(&crit_border, colors.critical_border);
    }
}

/// Get the path to gar's init.lua
pub fn gar_config_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
        .join("gar")
        .join("init.lua")
}

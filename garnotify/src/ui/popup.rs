//! Notification popup window
//!
//! Handles X11 window creation, Cairo rendering, and user interaction
//! for individual notification popups.

use anyhow::{Context, Result};
use cairo::{Context as CairoContext, Format, ImageSurface};
use gartk_core::{Color, Rect};
use gartk_x11::{Connection, Window, WindowConfig};
use tracing::{debug, info, warn};
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

use crate::config::AppearanceConfig;
use crate::notification::{Notification, Urgency};
use super::icons::{load_notification_icon, LoadedIcon};

/// Default notification height (will be calculated based on content)
const DEFAULT_HEIGHT: u32 = 80;
/// Padding inside the notification
const PADDING: u32 = 12;
/// Border radius for rounded corners
const BORDER_RADIUS: f64 = 8.0;
/// Icon size
const ICON_SIZE: u32 = 48;
/// Gap between icon and text
const ICON_TEXT_GAP: u32 = 12;
/// Action button height
const ACTION_BUTTON_HEIGHT: u32 = 28;
/// Action button padding
const ACTION_BUTTON_PADDING: u32 = 8;
/// Gap between action buttons
const ACTION_BUTTON_GAP: u32 = 6;
/// Gap between content and actions
const CONTENT_ACTION_GAP: u32 = 8;

/// A notification popup window
pub struct NotificationPopup {
    /// X11 connection
    conn: Connection,
    /// The X11 window
    window: Window,
    /// Graphics context for drawing
    gc: u32,
    /// Cairo surface for rendering
    surface: ImageSurface,
    /// The notification being displayed
    notification: Notification,
    /// Window geometry
    rect: Rect,
    /// Appearance configuration
    appearance: AppearanceConfig,
    /// Whether the mouse is hovering over the popup
    hovered: bool,
    /// Loaded icon (if any)
    icon: Option<LoadedIcon>,
    /// Bounds of action buttons (in window-local coordinates)
    action_bounds: Vec<Rect>,
    /// Index of currently hovered action button (None if no button hovered)
    hovered_action: Option<usize>,
}

impl NotificationPopup {
    /// Create a new notification popup
    pub fn new(
        conn: Connection,
        notification: Notification,
        rect: Rect,
        appearance: &AppearanceConfig,
    ) -> Result<Self> {
        // Create ARGB window for transparency
        let window = Window::create(
            conn.clone(),
            WindowConfig::default()
                .title(&format!("garnotify-{}", notification.id))
                .class("garnotify")
                .size(rect.width, rect.height)
                .position(rect.x, rect.y)
                .override_redirect(true)
                .transparent(true)
                .map_on_create(false),
        )
        .context("Failed to create notification window")?;

        // Create graphics context
        let gc = conn.generate_id()?;
        conn.inner()
            .create_gc(gc, window.id(), &Default::default())?;

        // Create Cairo surface
        let surface = ImageSurface::create(Format::ARgb32, rect.width as i32, rect.height as i32)
            .context("Failed to create Cairo surface")?;

        // Load icon
        let icon = load_notification_icon(&notification, appearance.icon_size);

        info!(
            "Created popup window {} for notification {} at ({}, {}) with icon: {}",
            window.id(),
            notification.id,
            rect.x,
            rect.y,
            icon.is_some()
        );

        Ok(Self {
            conn,
            window,
            gc,
            surface,
            notification,
            rect,
            appearance: appearance.clone(),
            hovered: false,
            icon,
            action_bounds: Vec::new(),
            hovered_action: None,
        })
    }

    /// Get the notification ID
    pub fn id(&self) -> u32 {
        self.notification.id
    }

    /// Get the window ID
    pub fn window_id(&self) -> u32 {
        self.window.id()
    }

    /// Get the notification
    pub fn notification(&self) -> &Notification {
        &self.notification
    }

    /// Update the notification content (for replacements)
    pub fn update_notification(&mut self, notification: Notification) -> Result<()> {
        // Reload icon if notification changed
        self.icon = load_notification_icon(&notification, self.appearance.icon_size);
        self.notification = notification;
        self.render()?;
        self.present()?;
        Ok(())
    }

    /// Show the popup window
    pub fn show(&mut self) -> Result<()> {
        self.render()?;
        self.window.map()?;
        self.present()?;
        self.conn.flush()?;
        debug!("Showed popup {} for notification {}", self.window.id(), self.notification.id);
        Ok(())
    }

    /// Hide the popup window
    pub fn hide(&self) -> Result<()> {
        self.window.unmap()?;
        self.conn.flush()?;
        debug!("Hid popup {} for notification {}", self.window.id(), self.notification.id);
        Ok(())
    }

    /// Move the popup to a new position
    pub fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        self.rect.x = x;
        self.rect.y = y;
        self.conn.inner().configure_window(
            self.window.id(),
            &x11rb::protocol::xproto::ConfigureWindowAux::new().x(x).y(y),
        )?;
        self.conn.flush()?;
        Ok(())
    }

    /// Render the notification content
    pub fn render(&mut self) -> Result<()> {
        let ctx = CairoContext::new(&self.surface).context("Failed to create Cairo context")?;

        // Clear with transparent background
        ctx.set_operator(cairo::Operator::Clear);
        ctx.paint()?;
        ctx.set_operator(cairo::Operator::Over);

        // Get colors based on urgency
        let (bg_color, fg_color, border_color) = self.get_urgency_colors();

        // Draw rounded rectangle background
        self.draw_rounded_rect(&ctx, bg_color, border_color)?;

        // Draw content (icon, summary, body)
        self.draw_content(&ctx, fg_color)?;

        // Draw action buttons if present
        if !self.notification.actions.is_empty() {
            self.draw_actions(&ctx, fg_color, border_color)?;
        }

        self.surface.flush();
        Ok(())
    }

    /// Render and present to window (for updates like hover effects)
    pub fn render_and_present(&mut self) -> Result<()> {
        self.render()?;
        self.present()?;
        Ok(())
    }

    /// Get colors based on notification urgency
    fn get_urgency_colors(&self) -> (Color, Color, Color) {
        let urgency = &self.notification.hints.urgency;
        let colors = &self.appearance.colors;

        let bg = match urgency {
            Urgency::Low => Color::from_hex(&colors.low_background)
                .unwrap_or(Color::new(0.1, 0.1, 0.1, 0.9)),
            Urgency::Normal => Color::from_hex(&colors.background)
                .unwrap_or(Color::new(0.12, 0.12, 0.14, 0.95)),
            Urgency::Critical => Color::from_hex(&colors.critical_background)
                .unwrap_or(Color::new(0.3, 0.1, 0.1, 0.95)),
        };

        let fg = match urgency {
            Urgency::Low => Color::from_hex(&colors.low_foreground)
                .unwrap_or(Color::new(0.7, 0.7, 0.7, 1.0)),
            Urgency::Normal => Color::from_hex(&colors.foreground)
                .unwrap_or(Color::new(0.9, 0.9, 0.9, 1.0)),
            Urgency::Critical => Color::from_hex(&colors.critical_foreground)
                .unwrap_or(Color::new(1.0, 0.9, 0.9, 1.0)),
        };

        let border = match urgency {
            Urgency::Low => Color::from_hex(&colors.low_border)
                .unwrap_or(Color::new(0.3, 0.3, 0.3, 0.5)),
            Urgency::Normal => Color::from_hex(&colors.border)
                .unwrap_or(Color::new(0.4, 0.4, 0.4, 0.5)),
            Urgency::Critical => Color::from_hex(&colors.critical_border)
                .unwrap_or(Color::new(0.8, 0.2, 0.2, 0.8)),
        };

        (bg, fg, border)
    }

    /// Draw rounded rectangle background with border
    fn draw_rounded_rect(&self, ctx: &CairoContext, bg: Color, border: Color) -> Result<()> {
        let w = self.rect.width as f64;
        let h = self.rect.height as f64;
        let r = BORDER_RADIUS;

        // Create rounded rectangle path
        ctx.new_path();
        ctx.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        ctx.arc(w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        ctx.arc(r, h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        ctx.arc(r, r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
        ctx.close_path();

        // Fill background
        ctx.set_source_rgba(bg.r, bg.g, bg.b, bg.a);
        ctx.fill_preserve()?;

        // Draw border
        ctx.set_source_rgba(border.r, border.g, border.b, border.a);
        ctx.set_line_width(1.0);
        ctx.stroke()?;

        Ok(())
    }

    /// Draw notification content (icon, summary, body)
    fn draw_content(&self, ctx: &CairoContext, fg: Color) -> Result<()> {
        let padding = PADDING as f64;
        let mut x = padding;
        let y = padding;

        // Draw icon if loaded
        if let Some(ref icon) = self.icon {
            let icon_y = (self.rect.height as f64 - icon.height as f64) / 2.0;
            match icon.to_cairo_surface() {
                Ok(icon_surface) => {
                    ctx.set_source_surface(&icon_surface, padding, icon_y)?;
                    ctx.paint()?;
                    debug!("Drew icon {}x{} at ({}, {})", icon.width, icon.height, padding, icon_y);
                }
                Err(e) => {
                    warn!("Failed to create icon surface: {}", e);
                }
            }
            x += icon.width as f64 + ICON_TEXT_GAP as f64;
        } else if !self.notification.app_icon.is_empty() || self.notification.hints.image_data.is_some() || self.notification.hints.image_path.is_some() {
            // Reserve space for icon even if loading failed (keeps layout consistent)
            x += ICON_SIZE as f64 + ICON_TEXT_GAP as f64;
        }

        // Create Pango layout for text
        let layout = pangocairo::functions::create_layout(ctx);

        // Set font (config has Pango-style font strings like "Sans 11")
        let font_desc = pango::FontDescription::from_string(&self.appearance.font);
        let title_font_desc = pango::FontDescription::from_string(&self.appearance.title_font);
        layout.set_font_description(Some(&font_desc));

        // Set width for wrapping
        let text_width = self.rect.width as f64 - x - padding;
        layout.set_width((text_width * pango::SCALE as f64) as i32);
        layout.set_ellipsize(pango::EllipsizeMode::End);

        // Draw summary (title font)
        layout.set_font_description(Some(&title_font_desc));
        layout.set_text(&self.notification.summary);

        ctx.set_source_rgba(fg.r, fg.g, fg.b, fg.a);
        ctx.move_to(x, y);
        pangocairo::functions::show_layout(ctx, &layout);

        // Get summary height for body positioning
        let (_, summary_height) = layout.pixel_size();

        // Draw body if present
        if !self.notification.body.is_empty() {
            layout.set_font_description(Some(&font_desc));
            layout.set_height(((self.rect.height as f64 - y - summary_height as f64 - padding - 4.0) * pango::SCALE as f64) as i32);

            // Try to parse as Pango markup (FreeDesktop spec allows HTML subset)
            // Sanitize the markup to only allow safe tags
            let body = sanitize_markup(&self.notification.body);

            // Pango's set_markup doesn't return a result, so we try it and
            // check if the layout has content. If markup parsing fails internally,
            // the layout might be empty or show an error.
            layout.set_markup(&body);

            // If the markup produced no content (possible parse error), fall back to plain text
            let (_, text_height) = layout.pixel_size();
            if text_height == 0 && !self.notification.body.is_empty() {
                let plain_body = strip_html_tags(&self.notification.body);
                layout.set_text(&plain_body);
                debug!("Body rendered as plain text (markup may have failed)");
            }

            // Slightly dimmer for body text
            ctx.set_source_rgba(fg.r * 0.8, fg.g * 0.8, fg.b * 0.8, fg.a);
            ctx.move_to(x, y + summary_height as f64 + 4.0);
            pangocairo::functions::show_layout(ctx, &layout);
        }

        Ok(())
    }

    /// Draw action buttons at the bottom of the notification
    fn draw_actions(&mut self, ctx: &CairoContext, fg: Color, border: Color) -> Result<()> {
        let actions = &self.notification.actions;
        if actions.is_empty() {
            return Ok(());
        }

        // Clear previous action bounds
        self.action_bounds.clear();

        let padding = PADDING as f64;
        let btn_height = ACTION_BUTTON_HEIGHT as f64;
        let btn_padding = ACTION_BUTTON_PADDING as f64;
        let btn_gap = ACTION_BUTTON_GAP as f64;

        // Position buttons at the bottom
        let btn_y = self.rect.height as f64 - padding - btn_height;

        // Calculate total width needed for all buttons
        let layout = pangocairo::functions::create_layout(ctx);
        let font_desc = pango::FontDescription::from_string(&self.appearance.font);
        layout.set_font_description(Some(&font_desc));

        // Measure button widths
        let mut button_widths: Vec<f64> = Vec::new();
        for action in actions {
            layout.set_text(&action.label);
            let (w, _) = layout.pixel_size();
            button_widths.push(w as f64 + btn_padding * 2.0);
        }

        let total_width: f64 = button_widths.iter().sum::<f64>() + btn_gap * (actions.len() as f64 - 1.0);
        let available_width = self.rect.width as f64 - padding * 2.0;

        // Scale buttons if they don't fit
        let scale = if total_width > available_width {
            available_width / total_width
        } else {
            1.0
        };

        // Draw buttons centered or left-aligned
        let mut btn_x = padding;
        if total_width < available_width {
            btn_x = padding + (available_width - total_width) / 2.0; // Center
        }

        let hovered_idx = self.hovered_action;

        for (i, action) in actions.iter().enumerate() {
            let btn_w = button_widths[i] * scale;
            let is_hovered = hovered_idx == Some(i);

            // Store button bounds for click detection
            self.action_bounds.push(Rect {
                x: btn_x as i32,
                y: btn_y as i32,
                width: btn_w as u32,
                height: btn_height as u32,
            });

            // Draw button background - brighter when hovered
            let btn_bg = if is_hovered {
                Color::new(border.r, border.g, border.b, border.a * 0.6)
            } else {
                Color::new(border.r, border.g, border.b, border.a * 0.3)
            };
            self.draw_button_rect(ctx, btn_x, btn_y, btn_w, btn_height, btn_bg)?;

            // Draw button border - more visible when hovered
            let border_alpha = if is_hovered { border.a * 0.9 } else { border.a * 0.5 };
            ctx.set_source_rgba(border.r, border.g, border.b, border_alpha);
            ctx.set_line_width(if is_hovered { 1.5 } else { 1.0 });
            self.stroke_button_rect(ctx, btn_x, btn_y, btn_w, btn_height)?;

            // Draw button label centered
            layout.set_text(&action.label);
            let (label_w, label_h) = layout.pixel_size();

            let label_x = btn_x + (btn_w - label_w as f64) / 2.0;
            let label_y = btn_y + (btn_height - label_h as f64) / 2.0;

            ctx.set_source_rgba(fg.r, fg.g, fg.b, fg.a);
            ctx.move_to(label_x, label_y);
            pangocairo::functions::show_layout(ctx, &layout);

            btn_x += btn_w + btn_gap;
        }

        debug!("Drew {} action buttons", actions.len());
        Ok(())
    }

    /// Draw a rounded button rectangle (filled)
    fn draw_button_rect(&self, ctx: &CairoContext, x: f64, y: f64, w: f64, h: f64, color: Color) -> Result<()> {
        let r = 4.0; // Small radius for buttons

        ctx.new_path();
        ctx.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        ctx.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        ctx.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        ctx.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
        ctx.close_path();

        ctx.set_source_rgba(color.r, color.g, color.b, color.a);
        ctx.fill()?;
        Ok(())
    }

    /// Stroke a rounded button rectangle (border only)
    fn stroke_button_rect(&self, ctx: &CairoContext, x: f64, y: f64, w: f64, h: f64) -> Result<()> {
        let r = 4.0;

        ctx.new_path();
        ctx.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        ctx.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        ctx.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        ctx.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
        ctx.close_path();

        ctx.stroke()?;
        Ok(())
    }

    /// Check if a click hit an action button, returns the action key if so
    /// x, y are window-local coordinates (from X11 ButtonPress event_x, event_y)
    pub fn check_action_click(&self, x: i32, y: i32) -> Option<String> {
        for (i, bounds) in self.action_bounds.iter().enumerate() {
            if x >= bounds.x
                && x < bounds.x + bounds.width as i32
                && y >= bounds.y
                && y < bounds.y + bounds.height as i32
            {
                if let Some(action) = self.notification.actions.get(i) {
                    debug!("Action button clicked: {} ({}) at ({}, {})", action.label, action.key, x, y);
                    return Some(action.key.clone());
                }
            }
        }
        debug!("Click at ({}, {}) didn't hit any action button", x, y);
        None
    }

    /// Present the rendered surface to the window
    fn present(&mut self) -> Result<()> {
        self.surface.flush();

        // Get surface data
        let data = self
            .surface
            .data()
            .map_err(|_| anyhow::anyhow!("Failed to get surface data"))?;

        // Put image to window
        self.conn
            .inner()
            .put_image(
                ImageFormat::Z_PIXMAP,
                self.window.id(),
                self.gc,
                self.rect.width as u16,
                self.rect.height as u16,
                0,
                0,
                0,
                self.window.depth(),
                &data,
            )
            .context("Failed to put image to window")?;

        self.conn.flush()?;
        Ok(())
    }

    /// Handle mouse enter event
    pub fn on_enter(&mut self) {
        self.hovered = true;
        debug!("Mouse entered notification {}", self.notification.id);
    }

    /// Handle mouse leave event
    pub fn on_leave(&mut self) {
        self.hovered = false;
        self.hovered_action = None;
        debug!("Mouse left notification {}", self.notification.id);
    }

    /// Handle mouse motion event, returns true if hover state changed (needs re-render)
    pub fn on_motion(&mut self, x: i32, y: i32) -> bool {
        let new_hovered = self.find_hovered_action(x, y);
        if new_hovered != self.hovered_action {
            self.hovered_action = new_hovered;
            true // Need to re-render
        } else {
            false
        }
    }

    /// Find which action button (if any) is at the given position
    fn find_hovered_action(&self, x: i32, y: i32) -> Option<usize> {
        for (i, bounds) in self.action_bounds.iter().enumerate() {
            if x >= bounds.x
                && x < bounds.x + bounds.width as i32
                && y >= bounds.y
                && y < bounds.y + bounds.height as i32
            {
                return Some(i);
            }
        }
        None
    }

    /// Check if point is inside the popup
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.rect.x
            && x < self.rect.x + self.rect.width as i32
            && y >= self.rect.y
            && y < self.rect.y + self.rect.height as i32
    }

    /// Check if mouse is hovering
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }
}

impl Drop for NotificationPopup {
    fn drop(&mut self) {
        // Free graphics context
        let _ = self.conn.inner().free_gc(self.gc);
        debug!("Dropped popup for notification {}", self.notification.id);
    }
}

/// Sanitize HTML markup for Pango
///
/// FreeDesktop spec allows: b, i, u, a (href), img (src, alt)
/// Pango supports: b, big, i, s, sub, sup, small, tt, u, span
/// We convert/pass through safe tags and strip/escape unsafe content
fn sanitize_markup(html: &str) -> String {
    // Tags that Pango supports and are safe to pass through
    let safe_tags = ["b", "i", "u", "s", "tt", "big", "small", "sub", "sup", "span"];

    let mut result = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '<' {
            // Start of a tag
            let mut tag = String::new();
            let mut in_tag = true;

            while let Some(&tc) = chars.peek() {
                if tc == '>' {
                    chars.next(); // consume '>'
                    in_tag = false;
                    break;
                }
                tag.push(chars.next().unwrap());
            }

            if in_tag {
                // Unclosed tag, escape the '<'
                result.push_str("&lt;");
                result.push_str(&tag);
                continue;
            }

            // Check if it's a safe tag
            let is_closing = tag.starts_with('/');
            let tag_name = if is_closing {
                &tag[1..]
            } else {
                tag.split_whitespace().next().unwrap_or(&tag)
            };
            let tag_name_lower = tag_name.to_lowercase();

            // Check if it's in our safe list
            let is_safe = safe_tags.iter().any(|&t| t == tag_name_lower);

            // Also allow closing tags for safe tags
            if is_safe {
                result.push('<');
                result.push_str(&tag);
                result.push('>');
            } else if tag_name_lower == "a" {
                // Convert <a href="..."> to underlined text (Pango doesn't support links)
                if is_closing {
                    result.push_str("</u>");
                } else {
                    result.push_str("<u>");
                }
            } else if tag_name_lower == "br" || tag_name_lower == "br/" {
                // Convert <br> to newline
                result.push('\n');
            } else if tag_name_lower == "img" {
                // Skip images (could show alt text, but let's keep it simple)
            } else {
                // Strip unknown tags (don't include them)
            }
        } else if c == '&' {
            // Check if it's a valid entity
            let mut entity = String::new();
            entity.push(c);
            let mut found_semicolon = false;

            while let Some(&ec) = chars.peek() {
                if ec == ';' {
                    entity.push(chars.next().unwrap());
                    found_semicolon = true;
                    break;
                } else if ec.is_alphanumeric() || ec == '#' {
                    entity.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            if found_semicolon {
                // Pass through valid entities
                result.push_str(&entity);
            } else {
                // Not a valid entity, escape the ampersand
                result.push_str("&amp;");
                result.push_str(&entity[1..]); // Skip the '&' we already escaped
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Strip basic HTML tags from body text (simplified implementation)
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }

    // Decode basic HTML entities
    result
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
}

/// Calculate the height needed for a notification based on its content
pub fn calculate_notification_height(
    notification: &Notification,
    width: u32,
    appearance: &AppearanceConfig,
) -> u32 {
    // Create a temporary surface just for measurement
    let surface = match ImageSurface::create(Format::ARgb32, 1, 1) {
        Ok(s) => s,
        Err(_) => return DEFAULT_HEIGHT,
    };

    let ctx = match CairoContext::new(&surface) {
        Ok(c) => c,
        Err(_) => return DEFAULT_HEIGHT,
    };

    let layout = pangocairo::functions::create_layout(&ctx);

    let font_desc = pango::FontDescription::from_string(&appearance.font);
    let title_font_desc = pango::FontDescription::from_string(&appearance.title_font);
    layout.set_font_description(Some(&font_desc));

    // Calculate text area width
    let text_x = if notification.app_icon.is_empty() {
        appearance.padding
    } else {
        appearance.padding + appearance.icon_size + ICON_TEXT_GAP
    };
    let text_width = width - text_x - appearance.padding;
    layout.set_width((text_width as i32) * pango::SCALE);

    // Measure summary (using title font)
    layout.set_font_description(Some(&title_font_desc));
    layout.set_text(&notification.summary);
    let (_, summary_height) = layout.pixel_size();

    // Measure body
    let body_height = if !notification.body.is_empty() {
        layout.set_font_description(Some(&font_desc));
        let body = strip_html_tags(&notification.body);
        layout.set_text(&body);
        let (_, h) = layout.pixel_size();
        h + 4 // gap between summary and body
    } else {
        0
    };

    // Account for action buttons if present
    let actions_height = if !notification.actions.is_empty() {
        CONTENT_ACTION_GAP + ACTION_BUTTON_HEIGHT
    } else {
        0
    };

    // Total height: padding + summary + body + actions + padding
    let height = (appearance.padding * 2) as i32 + summary_height + body_height + actions_height as i32;

    // Ensure minimum height and cap at reasonable maximum
    (height as u32).max(DEFAULT_HEIGHT).min(250)
}

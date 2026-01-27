//! Notification popup window
//!
//! Handles X11 window creation, Cairo rendering, and user interaction
//! for individual notification popups.

use anyhow::{Context, Result};
use cairo::{Context as CairoContext, Format, ImageSurface};
use gartk_core::{Color, Rect};
use gartk_x11::{Connection, Window, WindowConfig};
use tracing::{debug, info};
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

use crate::config::AppearanceConfig;
use crate::notification::{Notification, Urgency};

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

        info!(
            "Created popup window {} for notification {} at ({}, {})",
            window.id(),
            notification.id,
            rect.x,
            rect.y
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

        self.surface.flush();
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

        // TODO: Draw icon if present
        // For now, skip icon and just draw text
        if !self.notification.app_icon.is_empty() {
            // Reserve space for icon
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

            // Strip basic HTML tags (simplified)
            let body = strip_html_tags(&self.notification.body);
            layout.set_text(&body);
            layout.set_height(((self.rect.height as f64 - y - summary_height as f64 - padding - 4.0) * pango::SCALE as f64) as i32);

            // Slightly dimmer for body text
            ctx.set_source_rgba(fg.r * 0.8, fg.g * 0.8, fg.b * 0.8, fg.a);
            ctx.move_to(x, y + summary_height as f64 + 4.0);
            pangocairo::functions::show_layout(ctx, &layout);
        }

        Ok(())
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
        debug!("Mouse left notification {}", self.notification.id);
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

    // Total height: padding + summary + body + padding
    let height = (appearance.padding * 2) as i32 + summary_height + body_height;

    // Ensure minimum height and cap at reasonable maximum
    (height as u32).max(DEFAULT_HEIGHT).min(200)
}

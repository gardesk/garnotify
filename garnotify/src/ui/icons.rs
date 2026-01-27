//! Icon loading utilities for notification popups
//!
//! Handles loading icons from:
//! - Freedesktop icon themes (by name)
//! - File paths (PNG, JPEG, SVG)
//! - Raw image data from D-Bus hints
//! - Application icons via desktop entry

use anyhow::{Context, Result};
use image::GenericImageView;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

use crate::notification::{ImageData, Notification};

/// Loaded icon ready for Cairo rendering
#[derive(Debug, Clone)]
pub struct LoadedIcon {
    /// Width in pixels
    pub width: i32,
    /// Height in pixels
    pub height: i32,
    /// BGRA pixel data (premultiplied alpha) for Cairo
    pub data: Vec<u8>,
}

impl LoadedIcon {
    /// Create a Cairo ImageSurface from this icon
    pub fn to_cairo_surface(&self) -> Result<cairo::ImageSurface> {
        cairo::ImageSurface::create_for_data(
            self.data.clone(),
            cairo::Format::ARgb32,
            self.width,
            self.height,
            self.width * 4, // stride
        )
        .map_err(|e| anyhow::anyhow!("Failed to create Cairo surface: {}", e))
    }
}

/// Load the best available icon for a notification
///
/// Priority order:
/// 1. `image-data` hint (raw pixels from D-Bus)
/// 2. `image-path` hint (file path)
/// 3. `app_icon` parameter (theme name or file path)
/// 4. Desktop entry icon lookup
/// 5. None (no icon available)
pub fn load_notification_icon(notif: &Notification, target_size: u32) -> Option<LoadedIcon> {
    // 1. Try image-data hint (raw pixels)
    if let Some(ref img_data) = notif.hints.image_data {
        match load_from_image_data(img_data, target_size) {
            Ok(icon) => {
                debug!("Loaded icon from image-data hint for notification {}", notif.id);
                return Some(icon);
            }
            Err(e) => {
                warn!("Failed to load image-data: {}", e);
            }
        }
    }

    // 2. Try image-path hint
    if let Some(ref path) = notif.hints.image_path {
        let path = Path::new(path);
        if path.exists() {
            match load_from_file(path, target_size) {
                Ok(icon) => {
                    debug!("Loaded icon from image-path: {}", path.display());
                    return Some(icon);
                }
                Err(e) => {
                    warn!("Failed to load image-path {}: {}", path.display(), e);
                }
            }
        }
    }

    // 3. Try app_icon (can be theme name or file path)
    if !notif.app_icon.is_empty() {
        let app_icon = &notif.app_icon;

        // Check if it's a file path
        if app_icon.starts_with('/') || app_icon.starts_with("file://") {
            let path = if let Some(p) = app_icon.strip_prefix("file://") {
                PathBuf::from(p)
            } else {
                PathBuf::from(app_icon)
            };

            if path.exists() {
                match load_from_file(&path, target_size) {
                    Ok(icon) => {
                        debug!("Loaded icon from app_icon path: {}", path.display());
                        return Some(icon);
                    }
                    Err(e) => {
                        warn!("Failed to load app_icon path {}: {}", path.display(), e);
                    }
                }
            }
        }

        // Try as theme icon name
        if let Some(path) = find_theme_icon(app_icon, target_size) {
            match load_from_file(&path, target_size) {
                Ok(icon) => {
                    debug!("Loaded theme icon '{}' from {}", app_icon, path.display());
                    return Some(icon);
                }
                Err(e) => {
                    warn!("Failed to load theme icon {}: {}", path.display(), e);
                }
            }
        }
    }

    // 4. Try desktop entry lookup
    if let Some(ref desktop_entry) = notif.hints.desktop_entry {
        if let Some(icon) = load_from_desktop_entry(desktop_entry, target_size) {
            debug!("Loaded icon from desktop entry: {}", desktop_entry);
            return Some(icon);
        }
    }

    // 5. Try app_name as fallback theme lookup
    if !notif.app_name.is_empty() {
        let app_name_lower = notif.app_name.to_lowercase();
        if let Some(path) = find_theme_icon(&app_name_lower, target_size) {
            match load_from_file(&path, target_size) {
                Ok(icon) => {
                    debug!("Loaded icon from app_name '{}': {}", notif.app_name, path.display());
                    return Some(icon);
                }
                Err(e) => {
                    warn!("Failed to load app_name icon {}: {}", path.display(), e);
                }
            }
        }
    }

    debug!("No icon found for notification {}", notif.id);
    None
}

/// Load icon from D-Bus image-data hint
fn load_from_image_data(img: &ImageData, target_size: u32) -> Result<LoadedIcon> {
    // The image-data format from D-Bus is:
    // (width, height, rowstride, has_alpha, bits_per_sample, channels, data)
    // Data is typically RGBA or RGB with 8 bits per sample

    if img.width <= 0 || img.height <= 0 {
        anyhow::bail!("Invalid image dimensions: {}x{}", img.width, img.height);
    }

    let has_alpha = img.has_alpha;
    let channels = img.channels as usize;
    let expected_channels = if has_alpha { 4 } else { 3 };

    if channels != expected_channels {
        warn!("Unexpected channel count: {} (expected {})", channels, expected_channels);
    }

    // Calculate expected data size
    let expected_size = (img.rowstride as usize) * (img.height as usize);
    if img.data.len() < expected_size {
        anyhow::bail!(
            "Image data too short: {} bytes, expected at least {}",
            img.data.len(),
            expected_size
        );
    }

    // Convert to BGRA with premultiplied alpha
    let mut rgba_data = Vec::with_capacity((img.width * img.height * 4) as usize);
    let rowstride = img.rowstride as usize;

    for y in 0..img.height as usize {
        let row_start = y * rowstride;
        for x in 0..img.width as usize {
            let pixel_start = row_start + x * channels;

            let (r, g, b, a) = if has_alpha {
                (
                    img.data[pixel_start],
                    img.data[pixel_start + 1],
                    img.data[pixel_start + 2],
                    img.data[pixel_start + 3],
                )
            } else {
                (
                    img.data[pixel_start],
                    img.data[pixel_start + 1],
                    img.data[pixel_start + 2],
                    255,
                )
            };

            // Premultiply alpha and convert to BGRA for Cairo
            let af = a as f32 / 255.0;
            let r = (r as f32 * af).round() as u8;
            let g = (g as f32 * af).round() as u8;
            let b = (b as f32 * af).round() as u8;
            rgba_data.extend_from_slice(&[b, g, r, a]);
        }
    }

    // Scale if needed
    if img.width as u32 != target_size || img.height as u32 != target_size {
        scale_bgra_image(&rgba_data, img.width as u32, img.height as u32, target_size)
    } else {
        Ok(LoadedIcon {
            width: img.width,
            height: img.height,
            data: rgba_data,
        })
    }
}

/// Load icon from file (PNG, JPEG, or SVG)
fn load_from_file(path: &Path, target_size: u32) -> Result<LoadedIcon> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext.to_lowercase().as_str() {
        "svg" => load_svg(path, target_size),
        "png" | "jpg" | "jpeg" => load_raster(path, target_size),
        _ => anyhow::bail!("Unsupported icon format: {}", ext),
    }
}

/// Load a raster image (PNG, JPEG) and scale to target size
fn load_raster(path: &Path, target_size: u32) -> Result<LoadedIcon> {
    let img = image::open(path)
        .with_context(|| format!("Failed to open image: {}", path.display()))?;

    // Calculate scaling to fit in target_size while preserving aspect ratio
    let (orig_w, orig_h) = img.dimensions();
    let scale = target_size as f32 / orig_w.max(orig_h) as f32;
    let new_w = ((orig_w as f32 * scale).round() as u32).max(1);
    let new_h = ((orig_h as f32 * scale).round() as u32).max(1);

    let resized = img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3);
    let rgba = resized.into_rgba8();

    // Convert RGBA to BGRA (premultiplied alpha) for Cairo
    let mut data: Vec<u8> = Vec::with_capacity((new_w * new_h * 4) as usize);
    for pixel in rgba.pixels() {
        let [r, g, b, a] = pixel.0;
        // Premultiply alpha for Cairo
        let af = a as f32 / 255.0;
        let r = (r as f32 * af).round() as u8;
        let g = (g as f32 * af).round() as u8;
        let b = (b as f32 * af).round() as u8;
        data.extend_from_slice(&[b, g, r, a]);
    }

    debug!("Loaded raster icon: {}x{} from {}", new_w, new_h, path.display());
    Ok(LoadedIcon {
        width: new_w as i32,
        height: new_h as i32,
        data,
    })
}

/// Load an SVG and render at target size
fn load_svg(path: &Path, target_size: u32) -> Result<LoadedIcon> {
    let svg_data = std::fs::read(path)
        .with_context(|| format!("Failed to read SVG: {}", path.display()))?;

    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(&svg_data, &options)
        .with_context(|| format!("Failed to parse SVG: {}", path.display()))?;

    let svg_size = tree.size();
    let scale = target_size as f32 / svg_size.width().max(svg_size.height());
    let new_w = ((svg_size.width() * scale).round() as u32).max(1);
    let new_h = ((svg_size.height() * scale).round() as u32).max(1);

    let mut pixmap = tiny_skia::Pixmap::new(new_w, new_h)
        .ok_or_else(|| anyhow::anyhow!("Failed to create pixmap for SVG"))?;

    // Clear to transparent
    pixmap.fill(tiny_skia::Color::TRANSPARENT);

    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // tiny-skia uses RGBA premultiplied, convert to BGRA for Cairo
    let mut data = pixmap.take();
    for chunk in data.chunks_exact_mut(4) {
        chunk.swap(0, 2); // R <-> B
    }

    debug!("Loaded SVG icon: {}x{} from {}", new_w, new_h, path.display());
    Ok(LoadedIcon {
        width: new_w as i32,
        height: new_h as i32,
        data,
    })
}

/// Find icon in freedesktop icon themes
fn find_theme_icon(name: &str, size: u32) -> Option<PathBuf> {
    // Common icon theme search paths
    let search_dirs = [
        dirs::data_dir().map(|p| p.join("icons")),
        dirs::home_dir().map(|p| p.join(".local/share/icons")),
        Some(PathBuf::from("/usr/share/icons")),
        Some(PathBuf::from("/usr/share/pixmaps")),
    ];

    // Common theme names (in priority order)
    let themes = ["Adwaita", "breeze", "Papirus", "hicolor"];

    // Sizes to try (closest to requested first, then fallbacks)
    let sizes_to_try = [size, 48, 32, 24, 22, 16];

    // Categories to search
    let categories = ["apps", "status", "devices", "actions", "places", "mimetypes", "emblems"];

    // SVG directories to search (both scalable and symbolic)
    let svg_dirs = ["scalable", "symbolic"];

    for dir in search_dirs.iter().flatten() {
        for theme in &themes {
            // Try SVGs first (best quality) in both scalable and symbolic directories
            for svg_dir in &svg_dirs {
                for category in &categories {
                    // Try exact name
                    let svg_path = dir
                        .join(theme)
                        .join(svg_dir)
                        .join(category)
                        .join(format!("{}.svg", name));
                    if svg_path.exists() {
                        debug!("Found SVG icon: {}", svg_path.display());
                        return Some(svg_path);
                    }

                    // Try symbolic variant
                    let symbolic_path = dir
                        .join(theme)
                        .join(svg_dir)
                        .join(category)
                        .join(format!("{}-symbolic.svg", name));
                    if symbolic_path.exists() {
                        debug!("Found symbolic SVG icon: {}", symbolic_path.display());
                        return Some(symbolic_path);
                    }
                }
            }

            // Try PNG at various sizes
            for &sz in &sizes_to_try {
                for category in &categories {
                    let png_path = dir
                        .join(theme)
                        .join(format!("{}x{}", sz, sz))
                        .join(category)
                        .join(format!("{}.png", name));
                    if png_path.exists() {
                        debug!("Found PNG icon: {}", png_path.display());
                        return Some(png_path);
                    }
                }
            }
        }
    }

    // Try pixmaps directory directly
    let pixmap_paths = [
        PathBuf::from(format!("/usr/share/pixmaps/{}.svg", name)),
        PathBuf::from(format!("/usr/share/pixmaps/{}.png", name)),
        PathBuf::from(format!("/usr/share/pixmaps/{}.xpm", name)),
    ];

    for path in &pixmap_paths {
        if path.exists() {
            debug!("Found pixmap icon: {}", path.display());
            return Some(path.clone());
        }
    }

    debug!("Icon not found in themes: {}", name);
    None
}

/// Try to load icon from a desktop entry
fn load_from_desktop_entry(desktop_entry: &str, target_size: u32) -> Option<LoadedIcon> {
    // Desktop entry paths
    let search_dirs = [
        dirs::data_dir().map(|p| p.join("applications")),
        dirs::home_dir().map(|p| p.join(".local/share/applications")),
        Some(PathBuf::from("/usr/share/applications")),
        Some(PathBuf::from("/usr/local/share/applications")),
    ];

    for dir in search_dirs.iter().flatten() {
        let desktop_file = dir.join(format!("{}.desktop", desktop_entry));
        if desktop_file.exists() {
            if let Ok(contents) = std::fs::read_to_string(&desktop_file) {
                // Parse Icon= line
                for line in contents.lines() {
                    if let Some(icon_name) = line.strip_prefix("Icon=") {
                        let icon_name = icon_name.trim();
                        if !icon_name.is_empty() {
                            // Check if it's a path
                            if icon_name.starts_with('/') {
                                let path = Path::new(icon_name);
                                if path.exists() {
                                    if let Ok(icon) = load_from_file(path, target_size) {
                                        return Some(icon);
                                    }
                                }
                            } else {
                                // It's a theme icon name
                                if let Some(path) = find_theme_icon(icon_name, target_size) {
                                    if let Ok(icon) = load_from_file(&path, target_size) {
                                        return Some(icon);
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    None
}

/// Scale BGRA image data to target size (simple nearest-neighbor for speed)
fn scale_bgra_image(data: &[u8], width: u32, height: u32, target_size: u32) -> Result<LoadedIcon> {
    let scale = target_size as f32 / width.max(height) as f32;
    let new_w = ((width as f32 * scale).round() as u32).max(1);
    let new_h = ((height as f32 * scale).round() as u32).max(1);

    let mut scaled = Vec::with_capacity((new_w * new_h * 4) as usize);

    for y in 0..new_h {
        let src_y = ((y as f32 / scale) as u32).min(height - 1);
        for x in 0..new_w {
            let src_x = ((x as f32 / scale) as u32).min(width - 1);
            let src_idx = ((src_y * width + src_x) * 4) as usize;
            scaled.extend_from_slice(&data[src_idx..src_idx + 4]);
        }
    }

    Ok(LoadedIcon {
        width: new_w as i32,
        height: new_h as i32,
        data: scaled,
    })
}

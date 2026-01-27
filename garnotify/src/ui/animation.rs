//! Animation system for notification popups
//!
//! Handles slide and fade animations for popup appearance/disappearance.

use std::time::{Duration, Instant};

/// Animation state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationState {
    /// Popup is animating in (appearing)
    Appearing,
    /// Popup is fully visible
    Visible,
    /// Popup is animating to a new position (reflow)
    Reflowing,
    /// Popup is animating out (disappearing)
    Disappearing,
    /// Popup is hidden (animation complete)
    Hidden,
}

/// Slide direction for animations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlideDirection {
    Up,
    Down,
    Left,
    Right,
    None,
}

impl SlideDirection {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "up" => SlideDirection::Up,
            "down" => SlideDirection::Down,
            "left" => SlideDirection::Left,
            "right" => SlideDirection::Right,
            _ => SlideDirection::None,
        }
    }

    /// Get the offset multiplier for x and y based on direction
    /// Returns (dx, dy) where values are -1, 0, or 1
    pub fn offset_multiplier(&self) -> (i32, i32) {
        match self {
            SlideDirection::Up => (0, -1),
            SlideDirection::Down => (0, 1),
            SlideDirection::Left => (-1, 0),
            SlideDirection::Right => (1, 0),
            SlideDirection::None => (0, 0),
        }
    }
}

/// Animation configuration
#[derive(Debug, Clone)]
pub struct AnimationConfig {
    /// Whether animations are enabled
    pub enabled: bool,
    /// Duration for fade-in animation
    pub fade_in_duration: Duration,
    /// Duration for fade-out animation
    pub fade_out_duration: Duration,
    /// Slide direction
    pub slide_direction: SlideDirection,
    /// Slide distance in pixels
    pub slide_distance: i32,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fade_in_duration: Duration::from_millis(150),
            fade_out_duration: Duration::from_millis(150),
            slide_direction: SlideDirection::Down,
            slide_distance: 20,
        }
    }
}

impl AnimationConfig {
    /// Create from the app's AnimationConfig
    pub fn from_config(config: &crate::config::AnimationConfig) -> Self {
        Self {
            enabled: config.enabled,
            fade_in_duration: Duration::from_millis(config.fade_in as u64),
            fade_out_duration: Duration::from_millis(config.fade_out as u64),
            slide_direction: SlideDirection::from_str(&config.slide),
            slide_distance: config.slide_distance as i32,
        }
    }
}

/// Animator for a single popup
#[derive(Debug)]
pub struct Animator {
    /// Current animation state
    state: AnimationState,
    /// Animation configuration
    config: AnimationConfig,
    /// When the current animation started
    start_time: Instant,
    /// Target position (final x, y when animation completes)
    target_x: i32,
    target_y: i32,
    /// Start position for reflow animation
    reflow_start_x: i32,
    reflow_start_y: i32,
}

impl Animator {
    /// Create a new animator
    pub fn new(config: AnimationConfig, target_x: i32, target_y: i32) -> Self {
        Self {
            state: AnimationState::Hidden,
            config,
            start_time: Instant::now(),
            target_x,
            target_y,
            reflow_start_x: target_x,
            reflow_start_y: target_y,
        }
    }

    /// Start the appear animation
    pub fn start_appear(&mut self) {
        if !self.config.enabled {
            self.state = AnimationState::Visible;
            return;
        }
        self.state = AnimationState::Appearing;
        self.start_time = Instant::now();
    }

    /// Start the disappear animation
    pub fn start_disappear(&mut self) {
        if !self.config.enabled {
            self.state = AnimationState::Hidden;
            return;
        }
        self.state = AnimationState::Disappearing;
        self.start_time = Instant::now();
    }

    /// Start a reflow animation to a new position
    pub fn start_reflow(&mut self, current_x: i32, current_y: i32, new_target_x: i32, new_target_y: i32) {
        if !self.config.enabled {
            self.target_x = new_target_x;
            self.target_y = new_target_y;
            return;
        }
        self.reflow_start_x = current_x;
        self.reflow_start_y = current_y;
        self.target_x = new_target_x;
        self.target_y = new_target_y;
        self.state = AnimationState::Reflowing;
        self.start_time = Instant::now();
    }

    /// Update the animation state, returns true if animation is still in progress
    pub fn update(&mut self) -> bool {
        match self.state {
            AnimationState::Appearing => {
                let elapsed = self.start_time.elapsed();
                if elapsed >= self.config.fade_in_duration {
                    self.state = AnimationState::Visible;
                    false
                } else {
                    true
                }
            }
            AnimationState::Reflowing => {
                let elapsed = self.start_time.elapsed();
                // Use fade_in duration for reflow animation
                if elapsed >= self.config.fade_in_duration {
                    self.state = AnimationState::Visible;
                    false
                } else {
                    true
                }
            }
            AnimationState::Disappearing => {
                let elapsed = self.start_time.elapsed();
                if elapsed >= self.config.fade_out_duration {
                    self.state = AnimationState::Hidden;
                    false
                } else {
                    true
                }
            }
            _ => false,
        }
    }

    /// Get the current animation state
    pub fn state(&self) -> AnimationState {
        self.state
    }

    /// Check if the popup should be visible (not hidden)
    pub fn is_visible(&self) -> bool {
        self.state != AnimationState::Hidden
    }

    /// Check if animation is in progress
    pub fn is_animating(&self) -> bool {
        matches!(self.state, AnimationState::Appearing | AnimationState::Reflowing | AnimationState::Disappearing)
    }

    /// Get the current animation progress (0.0 to 1.0)
    pub fn progress(&self) -> f64 {
        match self.state {
            AnimationState::Appearing | AnimationState::Reflowing => {
                let elapsed = self.start_time.elapsed();
                let duration = self.config.fade_in_duration;
                if duration.is_zero() {
                    1.0
                } else {
                    (elapsed.as_secs_f64() / duration.as_secs_f64()).min(1.0)
                }
            }
            AnimationState::Disappearing => {
                let elapsed = self.start_time.elapsed();
                let duration = self.config.fade_out_duration;
                if duration.is_zero() {
                    1.0
                } else {
                    (elapsed.as_secs_f64() / duration.as_secs_f64()).min(1.0)
                }
            }
            AnimationState::Visible => 1.0,
            AnimationState::Hidden => 0.0,
        }
    }

    /// Get the current opacity (0.0 to 1.0)
    pub fn opacity(&self) -> f64 {
        match self.state {
            AnimationState::Appearing => ease_out_cubic(self.progress()),
            AnimationState::Disappearing => 1.0 - ease_out_cubic(self.progress()),
            AnimationState::Visible | AnimationState::Reflowing => 1.0,
            AnimationState::Hidden => 0.0,
        }
    }

    /// Get the current position offset from target (for slide animation)
    pub fn position_offset(&self) -> (i32, i32) {
        let (dx, dy) = self.config.slide_direction.offset_multiplier();
        let distance = self.config.slide_distance;

        match self.state {
            AnimationState::Appearing => {
                // Start offset, move toward target
                let progress = ease_out_cubic(self.progress());
                let remaining = 1.0 - progress;
                (
                    (dx as f64 * distance as f64 * remaining) as i32,
                    (dy as f64 * distance as f64 * remaining) as i32,
                )
            }
            AnimationState::Disappearing => {
                // Move away from target
                let progress = ease_out_cubic(self.progress());
                (
                    (dx as f64 * distance as f64 * progress) as i32,
                    (dy as f64 * distance as f64 * progress) as i32,
                )
            }
            _ => (0, 0),
        }
    }

    /// Get the current position (target + offset)
    pub fn current_position(&self) -> (i32, i32) {
        match self.state {
            AnimationState::Reflowing => {
                // Interpolate between start and target positions
                let progress = ease_out_cubic(self.progress());
                let x = self.reflow_start_x + ((self.target_x - self.reflow_start_x) as f64 * progress) as i32;
                let y = self.reflow_start_y + ((self.target_y - self.reflow_start_y) as f64 * progress) as i32;
                (x, y)
            }
            _ => {
                let (offset_x, offset_y) = self.position_offset();
                (self.target_x + offset_x, self.target_y + offset_y)
            }
        }
    }

    /// Update the target position (e.g., when stack reflows)
    pub fn set_target_position(&mut self, x: i32, y: i32) {
        self.target_x = x;
        self.target_y = y;
    }

    /// Get target position
    pub fn target_position(&self) -> (i32, i32) {
        (self.target_x, self.target_y)
    }
}

/// Ease-out cubic easing function for smooth animations
fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slide_direction() {
        assert_eq!(SlideDirection::from_str("up"), SlideDirection::Up);
        assert_eq!(SlideDirection::from_str("DOWN"), SlideDirection::Down);
        assert_eq!(SlideDirection::from_str("invalid"), SlideDirection::None);
    }

    #[test]
    fn test_offset_multiplier() {
        assert_eq!(SlideDirection::Up.offset_multiplier(), (0, -1));
        assert_eq!(SlideDirection::Down.offset_multiplier(), (0, 1));
        assert_eq!(SlideDirection::Left.offset_multiplier(), (-1, 0));
        assert_eq!(SlideDirection::Right.offset_multiplier(), (1, 0));
    }

    #[test]
    fn test_animator_disabled() {
        let config = AnimationConfig {
            enabled: false,
            ..Default::default()
        };
        let mut animator = Animator::new(config, 100, 100);
        animator.start_appear();
        assert_eq!(animator.state(), AnimationState::Visible);
    }

    #[test]
    fn test_ease_out_cubic() {
        assert!((ease_out_cubic(0.0) - 0.0).abs() < 0.001);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < 0.001);
        // Should be > linear at midpoint
        assert!(ease_out_cubic(0.5) > 0.5);
    }
}

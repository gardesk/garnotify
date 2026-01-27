//! Rule actions for modifying notifications

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::notification::{Notification, Urgency};

/// Actions that can be performed on a matched notification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleAction {
    /// Suppress the notification (don't show it)
    Suppress,

    /// Set the urgency level
    SetUrgency {
        urgency: String, // "low", "normal", "critical"
    },

    /// Override the timeout (in milliseconds, 0 = use default, -1 = never expire)
    SetTimeout { timeout: i32 },

    /// Replace the summary text
    SetSummary { summary: String },

    /// Replace the body text
    SetBody { body: String },

    /// Append text to the body
    AppendBody { text: String },

    /// Prepend text to the summary
    PrependSummary { text: String },

    /// Set a custom sound (hint)
    SetSound { sound: String },

    /// Mark as transient (won't be saved to history)
    SetTransient { transient: bool },

    /// Execute a shell command (the notification data is available as env vars)
    Exec { command: String },
}

impl RuleAction {
    /// Apply this action to a notification
    /// Returns None if the notification should be suppressed
    pub fn apply(&self, notification: &mut Notification) -> Option<()> {
        match self {
            RuleAction::Suppress => {
                debug!("Suppressing notification {}", notification.id);
                return None;
            }

            RuleAction::SetUrgency { urgency } => {
                let new_urgency = match urgency.to_lowercase().as_str() {
                    "low" => Urgency::Low,
                    "normal" => Urgency::Normal,
                    "critical" => Urgency::Critical,
                    _ => {
                        debug!("Invalid urgency '{}', ignoring", urgency);
                        return Some(());
                    }
                };
                debug!(
                    "Setting urgency for notification {} to {:?}",
                    notification.id, new_urgency
                );
                notification.hints.urgency = new_urgency;
            }

            RuleAction::SetTimeout { timeout } => {
                debug!(
                    "Setting timeout for notification {} to {}ms",
                    notification.id, timeout
                );
                notification.expire_timeout = *timeout;
            }

            RuleAction::SetSummary { summary } => {
                debug!(
                    "Setting summary for notification {} to '{}'",
                    notification.id, summary
                );
                notification.summary = summary.clone();
            }

            RuleAction::SetBody { body } => {
                debug!(
                    "Setting body for notification {} to '{}'",
                    notification.id, body
                );
                notification.body = body.clone();
            }

            RuleAction::AppendBody { text } => {
                debug!("Appending to body of notification {}", notification.id);
                notification.body.push_str(text);
            }

            RuleAction::PrependSummary { text } => {
                debug!("Prepending to summary of notification {}", notification.id);
                notification.summary = format!("{}{}", text, notification.summary);
            }

            RuleAction::SetSound { sound } => {
                debug!(
                    "Setting sound for notification {} to '{}'",
                    notification.id, sound
                );
                notification.hints.sound_name = Some(sound.clone());
            }

            RuleAction::SetTransient { transient } => {
                debug!(
                    "Setting transient for notification {} to {}",
                    notification.id, transient
                );
                notification.hints.transient = *transient;
            }

            RuleAction::Exec { command } => {
                debug!(
                    "Executing command for notification {}: {}",
                    notification.id, command
                );
                // Execute command in background with notification data as env vars
                let cmd = command.clone();
                let app_name = notification.app_name.clone();
                let summary = notification.summary.clone();
                let body = notification.body.clone();
                let id = notification.id;

                std::thread::spawn(move || {
                    let result = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(&cmd)
                        .env("NOTIFICATION_ID", id.to_string())
                        .env("NOTIFICATION_APP", &app_name)
                        .env("NOTIFICATION_SUMMARY", &summary)
                        .env("NOTIFICATION_BODY", &body)
                        .output();

                    if let Err(e) = result {
                        tracing::warn!("Failed to execute rule command '{}': {}", cmd, e);
                    }
                });
            }
        }

        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suppress_action() {
        let action = RuleAction::Suppress;
        let mut notif = Notification::default();
        assert!(action.apply(&mut notif).is_none());
    }

    #[test]
    fn test_set_urgency() {
        let action = RuleAction::SetUrgency {
            urgency: "critical".to_string(),
        };
        let mut notif = Notification::default();
        assert!(action.apply(&mut notif).is_some());
        assert_eq!(notif.hints.urgency, Urgency::Critical);
    }

    #[test]
    fn test_set_timeout() {
        let action = RuleAction::SetTimeout { timeout: 10000 };
        let mut notif = Notification::default();
        action.apply(&mut notif);
        assert_eq!(notif.expire_timeout, 10000);
    }

    #[test]
    fn test_set_summary() {
        let action = RuleAction::SetSummary {
            summary: "New Summary".to_string(),
        };
        let mut notif = Notification::default();
        action.apply(&mut notif);
        assert_eq!(notif.summary, "New Summary");
    }

    #[test]
    fn test_prepend_summary() {
        let action = RuleAction::PrependSummary {
            text: "[Important] ".to_string(),
        };
        let mut notif = Notification::default();
        notif.summary = "Original".to_string();
        action.apply(&mut notif);
        assert_eq!(notif.summary, "[Important] Original");
    }
}

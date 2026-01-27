//! Notification rules system
//!
//! Allows filtering and modifying notifications based on pattern matching.

mod actions;
mod matcher;

pub use actions::RuleAction;
pub use matcher::Matcher;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::notification::Notification;

/// A notification rule that can match and modify notifications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Rule name (for enable/disable)
    pub name: String,
    /// Whether this rule is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Matcher conditions (all must match)
    #[serde(default)]
    pub match_app_name: Option<String>,
    #[serde(default)]
    pub match_summary: Option<String>,
    #[serde(default)]
    pub match_body: Option<String>,
    #[serde(default)]
    pub match_urgency: Option<String>,
    /// Actions to perform when matched
    #[serde(default)]
    pub actions: Vec<RuleAction>,
}

fn default_enabled() -> bool {
    true
}

impl Rule {
    /// Check if this rule matches a notification
    pub fn matches(&self, notification: &Notification) -> bool {
        if !self.enabled {
            return false;
        }

        // All specified conditions must match
        if let Some(ref pattern) = self.match_app_name {
            if !Matcher::matches(pattern, &notification.app_name) {
                return false;
            }
        }

        if let Some(ref pattern) = self.match_summary {
            if !Matcher::matches(pattern, &notification.summary) {
                return false;
            }
        }

        if let Some(ref pattern) = self.match_body {
            if !Matcher::matches(pattern, &notification.body) {
                return false;
            }
        }

        if let Some(ref pattern) = self.match_urgency {
            let urgency_str = format!("{:?}", notification.hints.urgency).to_lowercase();
            if !Matcher::matches(pattern, &urgency_str) {
                return false;
            }
        }

        true
    }

    /// Apply this rule's actions to a notification
    /// Returns None if the notification should be suppressed
    pub fn apply(&self, notification: &mut Notification) -> Option<()> {
        for action in &self.actions {
            match action.apply(notification) {
                Some(()) => {}
                None => {
                    debug!("Rule '{}' suppressed notification {}", self.name, notification.id);
                    return None;
                }
            }
        }
        Some(())
    }
}

/// Rule engine that manages and applies rules
#[derive(Debug, Default)]
pub struct RuleEngine {
    rules: Vec<Rule>,
}

impl RuleEngine {
    /// Create a new rule engine
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Create a rule engine with initial rules
    pub fn with_rules(rules: Vec<Rule>) -> Self {
        info!("Loaded {} notification rules", rules.len());
        Self { rules }
    }

    /// Add a rule
    pub fn add_rule(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    /// Get all rules
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Get mutable reference to rules
    pub fn rules_mut(&mut self) -> &mut Vec<Rule> {
        &mut self.rules
    }

    /// Enable a rule by name
    pub fn enable_rule(&mut self, name: &str) -> bool {
        for rule in &mut self.rules {
            if rule.name == name {
                rule.enabled = true;
                info!("Enabled rule '{}'", name);
                return true;
            }
        }
        false
    }

    /// Disable a rule by name
    pub fn disable_rule(&mut self, name: &str) -> bool {
        for rule in &mut self.rules {
            if rule.name == name {
                rule.enabled = false;
                info!("Disabled rule '{}'", name);
                return true;
            }
        }
        false
    }

    /// Process a notification through all rules
    /// Returns None if the notification should be suppressed
    pub fn process(&self, notification: &mut Notification) -> Option<()> {
        for rule in &self.rules {
            if rule.matches(notification) {
                debug!(
                    "Rule '{}' matched notification {} (app='{}', summary='{}')",
                    rule.name, notification.id, notification.app_name, notification.summary
                );
                if rule.apply(notification).is_none() {
                    return None;
                }
            }
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_matching() {
        let rule = Rule {
            name: "test".to_string(),
            enabled: true,
            match_app_name: Some("Firefox".to_string()),
            match_summary: None,
            match_body: None,
            match_urgency: None,
            actions: vec![],
        };

        let mut notif = Notification::default();
        notif.app_name = "Firefox".to_string();
        assert!(rule.matches(&notif));

        notif.app_name = "Chrome".to_string();
        assert!(!rule.matches(&notif));
    }

    #[test]
    fn test_disabled_rule() {
        let rule = Rule {
            name: "test".to_string(),
            enabled: false,
            match_app_name: Some("Firefox".to_string()),
            match_summary: None,
            match_body: None,
            match_urgency: None,
            actions: vec![],
        };

        let mut notif = Notification::default();
        notif.app_name = "Firefox".to_string();
        assert!(!rule.matches(&notif));
    }
}

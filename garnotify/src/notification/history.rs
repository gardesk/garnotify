//! Notification history management

use std::collections::VecDeque;
use tracing::debug;

use super::types::Notification;

/// Notification history with circular buffer
pub struct History {
    /// Stored notifications (most recent at back)
    items: VecDeque<Notification>,
    /// Maximum number of items to keep
    max_length: usize,
}

impl History {
    /// Create a new history with the given maximum length
    pub fn new(max_length: usize) -> Self {
        Self {
            items: VecDeque::with_capacity(max_length.min(100)),
            max_length,
        }
    }

    /// Add a notification to history
    ///
    /// If history is at capacity, the oldest item is removed.
    pub fn push(&mut self, notification: Notification) {
        // Check if notification is transient (should not be stored)
        if notification.hints.transient {
            debug!(
                "Skipping transient notification {} from history",
                notification.id
            );
            return;
        }

        debug!(
            "Adding notification {} to history (current size: {})",
            notification.id,
            self.items.len()
        );

        // Remove oldest if at capacity
        if self.items.len() >= self.max_length {
            self.items.pop_front();
        }

        self.items.push_back(notification);
    }

    /// Pop the most recent notification from history
    pub fn pop(&mut self) -> Option<Notification> {
        let notification = self.items.pop_back();
        if let Some(ref n) = notification {
            debug!("Popped notification {} from history", n.id);
        }
        notification
    }

    /// Get the most recent notification without removing it
    pub fn peek(&self) -> Option<&Notification> {
        self.items.back()
    }

    /// Clear all history
    pub fn clear(&mut self) {
        debug!("Clearing {} notifications from history", self.items.len());
        self.items.clear();
    }

    /// Get the number of items in history
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Check if history is empty
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Get all items in history (oldest first)
    pub fn list(&self) -> &VecDeque<Notification> {
        &self.items
    }

    /// Get items as a vector (most recent first)
    pub fn list_recent_first(&self) -> Vec<&Notification> {
        self.items.iter().rev().collect()
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::types::{Action, Hints};

    fn make_notification(id: u32, summary: &str) -> Notification {
        Notification::new(
            id,
            "test".into(),
            0,
            "".into(),
            summary.into(),
            "body".into(),
            vec![],
            Hints::default(),
            5000,
        )
    }

    #[test]
    fn test_push_pop() {
        let mut history = History::new(10);

        history.push(make_notification(1, "first"));
        history.push(make_notification(2, "second"));

        assert_eq!(history.len(), 2);

        let popped = history.pop().unwrap();
        assert_eq!(popped.id, 2);
        assert_eq!(popped.summary, "second");

        let popped = history.pop().unwrap();
        assert_eq!(popped.id, 1);

        assert!(history.is_empty());
    }

    #[test]
    fn test_max_length() {
        let mut history = History::new(3);

        history.push(make_notification(1, "first"));
        history.push(make_notification(2, "second"));
        history.push(make_notification(3, "third"));
        history.push(make_notification(4, "fourth"));

        assert_eq!(history.len(), 3);

        // Oldest (id=1) should have been removed
        let items: Vec<_> = history.list().iter().map(|n| n.id).collect();
        assert_eq!(items, vec![2, 3, 4]);
    }

    #[test]
    fn test_transient_not_stored() {
        let mut history = History::new(10);

        let mut notification = make_notification(1, "transient");
        notification.hints.transient = true;

        history.push(notification);

        assert!(history.is_empty());
    }
}

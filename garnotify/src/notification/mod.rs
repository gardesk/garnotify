//! Notification types, storage, and history management

mod history;
pub mod store;
mod types;

pub use history::History;
pub use store::{new_shared_store, NotificationEvent, SharedNotificationStore};
pub use types::{Action, CloseReason, Hints, Notification, Urgency, UrgencyTimeouts};

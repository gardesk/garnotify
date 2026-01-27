//! Pattern matching for notification rules
//!
//! Supports glob patterns (* and ?) and regex (prefixed with ~)

use regex::Regex;

/// Pattern matcher for rule conditions
pub struct Matcher;

impl Matcher {
    /// Check if a pattern matches a string
    ///
    /// Pattern types:
    /// - Simple string: exact match (case-insensitive)
    /// - Glob pattern: * matches any chars, ? matches single char
    /// - Regex: prefix with ~ (e.g., "~.*error.*")
    pub fn matches(pattern: &str, text: &str) -> bool {
        if pattern.is_empty() {
            return true;
        }

        // Regex pattern (starts with ~)
        if let Some(regex_pattern) = pattern.strip_prefix('~') {
            return Self::matches_regex(regex_pattern, text);
        }

        // Glob pattern (contains * or ?)
        if pattern.contains('*') || pattern.contains('?') {
            return Self::matches_glob(pattern, text);
        }

        // Simple case-insensitive substring match
        text.to_lowercase().contains(&pattern.to_lowercase())
    }

    /// Match using a regex pattern
    fn matches_regex(pattern: &str, text: &str) -> bool {
        match Regex::new(pattern) {
            Ok(re) => re.is_match(text),
            Err(_) => {
                // Invalid regex, fall back to literal match
                text.contains(pattern)
            }
        }
    }

    /// Match using a glob pattern
    fn matches_glob(pattern: &str, text: &str) -> bool {
        let pattern = pattern.to_lowercase();
        let text = text.to_lowercase();

        // Convert glob to regex
        let mut regex_pattern = String::from("^");
        for ch in pattern.chars() {
            match ch {
                '*' => regex_pattern.push_str(".*"),
                '?' => regex_pattern.push('.'),
                // Escape regex special characters
                '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                    regex_pattern.push('\\');
                    regex_pattern.push(ch);
                }
                _ => regex_pattern.push(ch),
            }
        }
        regex_pattern.push('$');

        match Regex::new(&regex_pattern) {
            Ok(re) => re.is_match(&text),
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_match() {
        assert!(Matcher::matches("firefox", "Firefox"));
        assert!(Matcher::matches("Firefox", "Mozilla Firefox"));
        assert!(!Matcher::matches("chrome", "Firefox"));
    }

    #[test]
    fn test_glob_star() {
        assert!(Matcher::matches("fire*", "Firefox"));
        assert!(Matcher::matches("*fox", "Firefox"));
        assert!(Matcher::matches("*ire*", "Firefox"));
        assert!(!Matcher::matches("chrome*", "Firefox"));
    }

    #[test]
    fn test_glob_question() {
        assert!(Matcher::matches("Firefo?", "Firefox"));
        assert!(Matcher::matches("?irefox", "Firefox"));
        assert!(!Matcher::matches("Firefo?", "Firefoxx"));
    }

    #[test]
    fn test_regex() {
        assert!(Matcher::matches("~[Ff]irefox", "Firefox"));
        assert!(Matcher::matches("~[Ff]irefox", "firefox"));
        assert!(Matcher::matches("~.*error.*", "An error occurred"));
        assert!(!Matcher::matches("~^Chrome$", "Firefox"));
    }

    #[test]
    fn test_empty_pattern() {
        assert!(Matcher::matches("", "anything"));
    }
}

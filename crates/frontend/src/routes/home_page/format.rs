//! Pure display helpers for the account list / folder modal. No renderer, no
//! server deps — unit-testable.

/// Friendly label for an AccountStatus wire string.
pub(crate) fn status_label(status: &str) -> &'static str {
    match status {
        "PendingFirstSync" => "Pending first sync",
        "Syncing" => "Syncing",
        "Idle" => "Idle",
        "Error" => "Error",
        "Disabled" => "Disabled",
        _ => "Unknown",
    }
}

/// Tailwind background class for the status dot.
pub(crate) fn status_dot_class(status: &str) -> &'static str {
    match status {
        "Syncing" => "bg-blue-500",
        "Idle" => "bg-green-500",
        "Error" => "bg-red-500",
        "Disabled" => "bg-gray-400",
        "PendingFirstSync" => "bg-amber-500",
        _ => "bg-gray-400",
    }
}

/// Sort rank for special-use folders:
/// Inbox→Sent→Drafts→Archive→Trash→Junk→All→other.
pub(crate) fn special_use_rank(special_use: Option<&str>) -> u8 {
    match special_use {
        Some("inbox") => 0,
        Some("sent") => 1,
        Some("drafts") => 2,
        Some("archive") => 3,
        Some("trash") => 4,
        Some("junk") => 5,
        Some("all") => 6,
        _ => 7,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_dots() {
        assert_eq!(status_label("PendingFirstSync"), "Pending first sync");
        assert_eq!(status_label("bogus"), "Unknown");
        assert_eq!(status_dot_class("Idle"), "bg-green-500");
        assert_eq!(status_dot_class("Error"), "bg-red-500");
    }

    #[test]
    fn special_use_ordering() {
        let mut v = vec![Some("sent"), None, Some("inbox"), Some("trash")];
        v.sort_by_key(|s| special_use_rank(*s));
        assert_eq!(v, vec![Some("inbox"), Some("sent"), Some("trash"), None]);
    }
}

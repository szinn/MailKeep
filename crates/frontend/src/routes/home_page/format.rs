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

/// Tailwind text-color class for the status glyph (paired with the SVG which
/// uses `stroke: currentColor`).
pub(crate) fn status_icon_color(status: &str) -> &'static str {
    match status {
        "Syncing" => "text-blue-500",
        "Idle" => "text-green-500",
        "Error" => "text-red-500",
        "PendingFirstSync" => "text-amber-500",
        // Disabled + unknown → gray
        _ => "text-gray-400",
    }
}

/// Hover-tooltip text for the status glyph. Base is the status label (or
/// `Error: <message>` for the Error state), with ` · last synced <relative>`
/// appended whenever a last-sync time is present. `last_synced` is already a
/// preformatted relative string (e.g. "2m ago").
pub(crate) fn status_tooltip(status: &str, last_synced: Option<&str>, last_error: Option<&str>) -> String {
    let base = if status == "Error" {
        match last_error {
            Some(e) if !e.is_empty() => format!("Error: {e}"),
            _ => "Error".to_string(),
        }
    } else {
        status_label(status).to_string()
    };
    match last_synced {
        Some(t) if !t.is_empty() => format!("{base} · last synced {t}"),
        _ => base,
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

/// Human-readable byte size. Whole bytes below 1 KB; otherwise one decimal
/// place in the largest fitting unit (KB/MB/GB/TB, base 1000).
pub(crate) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Group an integer with thousands separators (e.g. 12431 -> "12,431").
pub(crate) fn thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let first = bytes.len() % 3;
    for (i, b) in bytes.iter().enumerate() {
        if i != 0 && i >= first && (i - first).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_colors() {
        assert_eq!(status_label("PendingFirstSync"), "Pending first sync");
        assert_eq!(status_label("bogus"), "Unknown");
        assert_eq!(status_icon_color("Idle"), "text-green-500");
        assert_eq!(status_icon_color("Error"), "text-red-500");
        assert_eq!(status_icon_color("bogus"), "text-gray-400");
    }

    #[test]
    fn tooltip_composition() {
        assert_eq!(status_tooltip("Idle", Some("2m ago"), None), "Idle · last synced 2m ago");
        assert_eq!(
            status_tooltip("Error", Some("1h ago"), Some("authentication failed")),
            "Error: authentication failed · last synced 1h ago"
        );
        assert_eq!(status_tooltip("Error", None, None), "Error");
        assert_eq!(status_tooltip("PendingFirstSync", None, None), "Pending first sync");
        assert_eq!(status_tooltip("Disabled", Some("3d ago"), None), "Disabled · last synced 3d ago");
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1000), "1.0 KB");
        assert_eq!(format_bytes(1_500), "1.5 KB");
        assert_eq!(format_bytes(4_509_715_660), "4.5 GB");
    }

    #[test]
    fn thousands_groups() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(12_431), "12,431");
        assert_eq!(thousands(1_000_000), "1,000,000");
    }

    #[test]
    fn special_use_ordering() {
        let mut v = vec![Some("sent"), None, Some("inbox"), Some("trash")];
        v.sort_by_key(|s| special_use_rank(*s));
        assert_eq!(v, vec![Some("inbox"), Some("sent"), Some("trash"), None]);
    }
}

use dioxus::prelude::*;

/// The search box contents — drives the input value and ghost-text completion.
/// Global so it survives NavBar re-mounts across navigation.
pub(crate) static SEARCH_QUERY: GlobalSignal<String> = Signal::global(String::new);

/// The *submitted* query the results key on. `Some` ⇒ the HomePage right panel
/// shows search results; `None` ⇒ normal stats / message-list. Set on Enter;
/// cleared on clear-× / Esc / account selection / Home.
pub(crate) static ACTIVE_SEARCH: GlobalSignal<Option<String>> = Signal::global(|| None);

/// MK-20's hinted field grammar. The UI completes these names; it never parses
/// or executes the grammar — the raw query string goes straight to the backend.
const FIELD_NAMES: [&str; 10] = ["account", "folder", "subject", "from", "to", "before", "after", "date", "attachments", "has"];

/// Rotating placeholder tips shown while the box is focused and empty.
pub(crate) const PLACEHOLDER_TIPS: [&str; 6] = [
    "Try: from:amazon",
    "Try: subject:\"order confirmation\"",
    "Try: folder:orders !filters",
    "Try: after:2025-01-01",
    "Try: has:attachment",
    "Try: account:Fastmail coffee",
];

/// Fixed value set for a field that has one, else `None` (open-ended).
fn fixed_values(field: &str) -> Option<&'static [&'static str]> {
    if field.eq_ignore_ascii_case("attachments") {
        Some(&["none", "some"])
    } else if field.eq_ignore_ascii_case("has") {
        Some(&["attachment"])
    } else {
        None
    }
}

/// Returns the ghost-text suffix to display for the last token in `input`.
///
/// Field-name tokens (no colon) complete to the remaining field name + `:`
/// (e.g. `"fol"` → `"der:"`). `attachments:`/`has:` value tokens complete the
/// partial value (e.g. `"has:att"` → `"achment"`); other fields are open-ended
/// (empty suffix after `:`). A leading `!` on the token is transparent to the
/// suffix (it stays on the token; `apply_completion` re-appends onto it).
///
/// Empty when: input is empty or ends in whitespace; the token is a lone `!`;
/// the field after `:` is open-ended; a fixed-value field has no value typed;
/// or nothing matches. `cycle_idx` selects among matches (alphabetical) and
/// wraps.
pub(crate) fn compute_completion(input: &str, cycle_idx: usize) -> String {
    if input.is_empty() || input.ends_with(|c: char| c.is_whitespace()) {
        return String::new();
    }
    let last_token = input.split_whitespace().last().unwrap_or("");
    // A leading '!' negates the token; completion works on the field part after it.
    let bare = last_token.strip_prefix('!').unwrap_or(last_token);
    if bare.is_empty() {
        return String::new();
    }

    if let Some(colon_pos) = bare.find(':') {
        let field = &bare[..colon_pos];
        let partial_value = &bare[colon_pos + 1..];
        let Some(values) = fixed_values(field) else {
            return String::new(); // open-ended field: no value completion
        };
        if partial_value.is_empty() {
            return String::new(); // require >= 1 char before ghosting a value
        }
        let lower = partial_value.to_lowercase();
        let matches: Vec<&str> = values.iter().copied().filter(|v| v.starts_with(lower.as_str())).collect();
        if matches.is_empty() {
            return String::new();
        }
        return matches[cycle_idx % matches.len()][lower.len()..].to_string();
    }

    let lower = bare.to_lowercase();
    let field_matches: Vec<&str> = FIELD_NAMES.iter().copied().filter(|name| name.starts_with(lower.as_str())).collect();
    if field_matches.is_empty() {
        return String::new();
    }
    format!("{}:", &field_matches[cycle_idx % field_matches.len()][lower.len()..])
}

/// Appends a completion suffix to the last token of `input`.
/// `apply_completion("fol", "der:")` → `"folder:"`;
/// `apply_completion("!fol", "der:")` → `"!folder:"` (the `!` rides on the
/// token).
pub(crate) fn apply_completion(input: &str, suffix: &str) -> String {
    let last_space = input.rfind(|c: char| c.is_whitespace()).map_or(0, |i| i + 1);
    format!("{}{}{}", &input[..last_space], &input[last_space..], suffix)
}

/// Produces the next cycled input when Tab is pressed after a completion has
/// already been applied (shell-style cycling). A leading `!` on `cycle_prefix`
/// is preserved. Returns `None` when there is only one match (no cycling).
pub(crate) fn next_cycle_input(current_input: &str, cycle_prefix: &str, cycle_idx: usize) -> Option<String> {
    let last_space = current_input.rfind(|c: char| c.is_whitespace()).map_or(0, |i| i + 1);
    let head = &current_input[..last_space];
    let (neg, prefix_bare) = match cycle_prefix.strip_prefix('!') {
        Some(rest) => ("!", rest),
        None => ("", cycle_prefix),
    };
    let lower = prefix_bare.to_lowercase();

    if let Some(colon_pos) = lower.find(':') {
        let field = &lower[..colon_pos];
        let partial_value = &lower[colon_pos + 1..];
        let values = fixed_values(field)?;
        let matches: Vec<&str> = values.iter().copied().filter(|v| v.starts_with(partial_value)).collect();
        if matches.len() < 2 {
            return None;
        }
        let next_value = matches[cycle_idx % matches.len()];
        return Some(format!("{head}{neg}{field}:{next_value}"));
    }

    let field_matches: Vec<&str> = FIELD_NAMES.iter().copied().filter(|name| name.starts_with(lower.as_str())).collect();
    if field_matches.len() < 2 {
        return None;
    }
    let next_field = field_matches[cycle_idx % field_matches.len()];
    Some(format!("{head}{neg}{next_field}:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_empty_and_trailing_space() {
        assert_eq!(compute_completion("", 0), "");
        assert_eq!(compute_completion("from ", 0), "");
        assert_eq!(compute_completion("!", 0), ""); // lone negation, no field yet
    }

    #[test]
    fn completion_field_name_prefix() {
        assert_eq!(compute_completion("fol", 0), "der:");
        assert_eq!(compute_completion("subj", 0), "ect:");
        assert_eq!(compute_completion("account", 0), ":"); // exact name → just colon
    }

    #[test]
    fn completion_ambiguous_field_prefix_cycles_alphabetical() {
        // "a" matches account, after, attachments (alphabetical).
        assert_eq!(compute_completion("a", 0), "ccount:");
        assert_eq!(compute_completion("a", 1), "fter:");
        assert_eq!(compute_completion("a", 2), "ttachments:");
        assert_eq!(compute_completion("a", 3), "ccount:"); // wraps
    }

    #[test]
    fn completion_open_ended_field_after_colon_is_empty() {
        assert_eq!(compute_completion("from:", 0), "");
        assert_eq!(compute_completion("from:amazon", 0), "");
        assert_eq!(compute_completion("subject:order", 0), "");
    }

    #[test]
    fn completion_fixed_value_fields() {
        assert_eq!(compute_completion("has:att", 0), "achment");
        assert_eq!(compute_completion("has:", 0), ""); // require >= 1 char
        assert_eq!(compute_completion("attachments:n", 0), "one");
        assert_eq!(compute_completion("attachments:s", 0), "ome");
        assert_eq!(compute_completion("attachments:x", 0), ""); // no match
    }

    #[test]
    fn completion_negated_field_prefix() {
        assert_eq!(compute_completion("!fol", 0), "der:");
        assert_eq!(compute_completion("!has:att", 0), "achment");
    }

    #[test]
    fn completion_uses_last_word() {
        assert_eq!(compute_completion("from:amazon fol", 0), "der:");
    }

    #[test]
    fn apply_completion_appends_to_last_token() {
        assert_eq!(apply_completion("fol", "der:"), "folder:");
        assert_eq!(apply_completion("!fol", "der:"), "!folder:");
        assert_eq!(apply_completion("from:amazon fol", "der:"), "from:amazon folder:");
        assert_eq!(apply_completion("has:att", "achment"), "has:attachment");
        assert_eq!(apply_completion("from:amazon", ""), "from:amazon"); // empty suffix unchanged
    }

    #[test]
    fn next_cycle_input_cycles_fields() {
        assert_eq!(next_cycle_input("account:", "a", 1), Some("after:".to_string()));
        assert_eq!(next_cycle_input("after:", "a", 2), Some("attachments:".to_string()));
        assert_eq!(next_cycle_input("attachments:", "a", 3), Some("account:".to_string())); // wraps
    }

    #[test]
    fn next_cycle_input_single_match_is_none() {
        assert_eq!(next_cycle_input("folder:", "fol", 1), None);
    }

    #[test]
    fn next_cycle_input_preserves_prefix_tokens_and_negation() {
        assert_eq!(next_cycle_input("from:x account:", "a", 1), Some("from:x after:".to_string()));
        assert_eq!(next_cycle_input("!account:", "!a", 1), Some("!after:".to_string()));
    }

    #[test]
    fn next_cycle_input_attachments_values_cycle() {
        assert_eq!(next_cycle_input("attachments:none", "attachments:n", 1), None); // only "none" starts with n
        assert_eq!(next_cycle_input("attachments:none", "attachments:", 1), Some("attachments:some".to_string()));
        assert_eq!(next_cycle_input("attachments:some", "attachments:", 2), Some("attachments:none".to_string()));
    }
}

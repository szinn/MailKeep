//! Password validation helpers shared across registration and login flows.

pub(crate) const SPECIAL_CHARS: &str = "!@#$%^&*()_+-=[]{}|;:,.<>?";
pub(crate) const MIN_PASSWORD_LEN: usize = 12;

pub(crate) fn password_requirements(pw: &str) -> Vec<(String, bool)> {
    vec![
        (format!("At least {MIN_PASSWORD_LEN} characters"), pw.len() >= MIN_PASSWORD_LEN),
        ("One uppercase letter (A–Z)".to_string(), pw.chars().any(char::is_uppercase)),
        ("One lowercase letter (a–z)".to_string(), pw.chars().any(char::is_lowercase)),
        ("One digit (0–9)".to_string(), pw.chars().any(|c| c.is_ascii_digit())),
        ("One special character (!@#$%^&*…)".to_string(), pw.chars().any(|c| SPECIAL_CHARS.contains(c))),
    ]
}

pub(crate) fn password_is_valid(pw: &str) -> bool {
    password_requirements(pw).iter().all(|(_, ok)| *ok)
}

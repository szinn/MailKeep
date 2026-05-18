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

/// Server-side password strength validation. Returns `Err` with a user-facing
/// message if the password does not satisfy all requirements.
#[cfg(feature = "server")]
pub(crate) fn validate_password_strength(password: &str) -> Result<(), dioxus::prelude::ServerFnError> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(dioxus::prelude::ServerFnError::new(format!(
            "Password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    if !password.chars().any(char::is_uppercase) {
        return Err(dioxus::prelude::ServerFnError::new("Password must contain at least one uppercase letter"));
    }
    if !password.chars().any(char::is_lowercase) {
        return Err(dioxus::prelude::ServerFnError::new("Password must contain at least one lowercase letter"));
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return Err(dioxus::prelude::ServerFnError::new("Password must contain at least one digit"));
    }
    if !password.chars().any(|c| SPECIAL_CHARS.contains(c)) {
        return Err(dioxus::prelude::ServerFnError::new("Password must contain at least one special character"));
    }
    Ok(())
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageFlags {
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub draft: bool,
    pub deleted: bool,
    pub recent: bool,
    /// Server-defined keywords (e.g. `"$Important"`, `"$Phishing"`).
    pub custom: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_json_round_trip() {
        let flags = MessageFlags {
            seen: true,
            answered: false,
            flagged: true,
            draft: false,
            deleted: false,
            recent: false,
            custom: vec!["$Important".into(), "$Phishing".into()],
        };
        let json = serde_json::to_string(&flags).unwrap();
        let back: MessageFlags = serde_json::from_str(&json).unwrap();
        assert_eq!(back, flags);
    }
}

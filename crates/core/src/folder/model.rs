use chrono::{DateTime, Utc};
use derive_builder::Builder;
use mk_utils::{define_token_prefix, token::Token};

use crate::account::AccountId;

define_token_prefix!(FolderTokenPrefix, "F_");
pub type FolderId = u64;
pub type FolderToken = Token<FolderTokenPrefix, FolderId, { i64::MAX as u128 }>;

#[derive(Debug, Clone, PartialEq, Eq, Builder)]
pub struct Folder {
    pub id: FolderId,
    pub version: u64,
    pub token: FolderToken,
    pub account_id: AccountId,
    pub path: String,
    #[builder(default = "None")]
    pub display_name: Option<String>,
    #[builder(default = "None")]
    pub special_use: Option<SpecialUse>,
    #[builder(default = "true")]
    pub enabled: bool,
    #[builder(default = "false")]
    pub idle_enabled: bool,
    #[builder(default = "None")]
    pub uidvalidity: Option<u32>,
    #[builder(default = "0")]
    pub last_uid: u32,
    #[builder(default = "None")]
    pub last_synced_at: Option<DateTime<Utc>>,
    #[builder(default = "None")]
    pub last_error: Option<String>,
    #[builder(default = "Utc::now()")]
    pub created_at: DateTime<Utc>,
    #[builder(default = "Utc::now()")]
    pub updated_at: DateTime<Utc>,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialUse {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Archive,
    Junk,
    All,
}

impl SpecialUse {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Sent => "sent",
            Self::Drafts => "drafts",
            Self::Trash => "trash",
            Self::Archive => "archive",
            Self::Junk => "junk",
            Self::All => "all",
        }
    }
}

impl std::str::FromStr for SpecialUse {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "inbox" => Ok(Self::Inbox),
            "sent" => Ok(Self::Sent),
            "drafts" => Ok(Self::Drafts),
            "trash" => Ok(Self::Trash),
            "archive" => Ok(Self::Archive),
            "junk" => Ok(Self::Junk),
            "all" => Ok(Self::All),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFolderRequest {
    pub path: String,
    pub display_name: Option<String>,
    pub special_use: Option<SpecialUse>,
    pub uidvalidity: Option<u32>,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn special_use_round_trips() {
        for v in [
            SpecialUse::Inbox,
            SpecialUse::Sent,
            SpecialUse::Drafts,
            SpecialUse::Trash,
            SpecialUse::Archive,
            SpecialUse::Junk,
            SpecialUse::All,
        ] {
            assert_eq!(SpecialUse::from_str(v.as_str()).unwrap(), v);
        }
    }

    #[test]
    fn special_use_unknown_returns_err() {
        SpecialUse::from_str("flagged").unwrap_err();
        SpecialUse::from_str("").unwrap_err();
        SpecialUse::from_str("INBOX").unwrap_err();
    }

    #[test]
    fn folder_token_id_matches_folder_id() {
        let token = FolderToken::generate();
        let id: FolderId = token.id();
        assert!(id > 0);
    }

    #[test]
    fn folder_token_display_starts_with_f_prefix() {
        let token = FolderToken::generate();
        assert!(token.to_string().starts_with("F_"));
    }

    #[test]
    fn folder_token_round_trips() {
        let token = FolderToken::generate();
        let s = token.to_string();
        let parsed = FolderToken::parse(&s).unwrap();
        assert_eq!(parsed.id(), token.id());
    }
}

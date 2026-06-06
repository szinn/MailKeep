use chrono::{DateTime, Utc};
use derive_builder::Builder;
use mk_utils::{define_token_prefix, token::Token};
use serde::{Deserialize, Serialize};

use crate::{
    account::AccountId,
    types::{ContentHash, EmailAddress},
};

define_token_prefix!(MessageTokenPrefix, "M_");
pub type MessageId = u64;
pub type MessageToken = Token<MessageTokenPrefix, MessageId, { i64::MAX as u128 }>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedAddress {
    pub address: EmailAddress,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Builder)]
pub struct Message {
    pub id: MessageId,
    pub version: u64,
    pub token: MessageToken,
    pub account_id: AccountId,
    pub rfc822_message_id: String,
    pub content_hash: ContentHash,
    #[builder(default = "None")]
    pub subject: Option<String>,
    pub from_address: EmailAddress,
    #[builder(default = "None")]
    pub from_name: Option<String>,
    #[builder(default = "Vec::new()")]
    pub to_addresses: Vec<NamedAddress>,
    #[builder(default = "Vec::new()")]
    pub cc_addresses: Vec<NamedAddress>,
    #[builder(default = "Vec::new()")]
    pub bcc_addresses: Vec<NamedAddress>,
    #[builder(default = "Vec::new()")]
    pub reply_to_addresses: Vec<NamedAddress>,
    #[builder(default = "None")]
    pub sent_date: Option<DateTime<Utc>>,
    #[builder(default = "None")]
    pub in_reply_to: Option<String>,
    #[builder(default = "Vec::new()")]
    pub references: Vec<String>,
    pub snippet: String,
    pub size_bytes: i64,
    #[builder(default = "false")]
    pub has_attachments: bool,
    #[builder(default = "0")]
    pub attachment_count: i32,
    #[builder(default = "Utc::now()")]
    pub created_at: DateTime<Utc>,
    #[builder(default = "Utc::now()")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedMessage {
    pub message_id: MessageId,
    pub created: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_address_json_round_trip() {
        let addr = NamedAddress {
            address: EmailAddress::new("alice@example.com").unwrap(),
            name: Some("Alice Example".into()),
        };
        let json = serde_json::to_string(&addr).unwrap();
        let back: NamedAddress = serde_json::from_str(&json).unwrap();
        assert_eq!(back, addr);
    }

    #[test]
    fn message_token_id_matches_message_id() {
        let token = MessageToken::generate();
        let id: MessageId = token.id();
        assert!(id > 0);
    }

    #[test]
    fn message_token_display_starts_with_m_prefix() {
        let token = MessageToken::generate();
        assert!(token.to_string().starts_with("M_"));
    }

    #[test]
    fn message_token_round_trips() {
        let token = MessageToken::generate();
        let s = token.to_string();
        let parsed = MessageToken::parse(&s).unwrap();
        assert_eq!(parsed.id(), token.id());
    }
}

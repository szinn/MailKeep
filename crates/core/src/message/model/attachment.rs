use chrono::{DateTime, Utc};
use derive_builder::Builder;
use mk_utils::{define_token_prefix, token::Token};

use crate::{account::AccountId, message::MessageId, types::ContentHash};

define_token_prefix!(MessageAttachmentTokenPrefix, "MA_");
pub type MessageAttachmentId = u64;
pub type MessageAttachmentToken = Token<MessageAttachmentTokenPrefix, MessageAttachmentId, { i64::MAX as u128 }>;

#[derive(Debug, Clone, PartialEq, Eq, Builder)]
pub struct MessageAttachment {
    pub id: MessageAttachmentId,
    pub version: u64,
    pub token: MessageAttachmentToken,
    pub message_id: MessageId,
    /// Denormalized from `Message.account_id` to power the
    /// `(account_id, content_hash)` dedup-stats index.
    pub account_id: AccountId,
    pub content_hash: ContentHash,
    #[builder(default = "None")]
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
    #[builder(default = "false")]
    pub is_inline: bool,
    #[builder(default = "None")]
    pub content_id: Option<String>,
    #[builder(default = "Utc::now()")]
    pub created_at: DateTime<Utc>,
    #[builder(default = "Utc::now()")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_attachment_token_round_trips() {
        let token = MessageAttachmentToken::generate();
        let s = token.to_string();
        assert!(s.starts_with("MA_"));
        let parsed = MessageAttachmentToken::parse(&s).unwrap();
        assert_eq!(parsed.id(), token.id());
    }
}

use chrono::{DateTime, Utc};
use derive_builder::Builder;
use mk_utils::{define_token_prefix, token::Token};

use crate::{
    folder::FolderId,
    message::{MessageFlags, MessageId},
};

define_token_prefix!(MessageLocationTokenPrefix, "ML_");
pub type MessageLocationId = u64;
pub type MessageLocationToken = Token<MessageLocationTokenPrefix, MessageLocationId, { i64::MAX as u128 }>;

#[derive(Debug, Clone, PartialEq, Eq, Builder)]
pub struct MessageLocation {
    pub id: MessageLocationId,
    pub version: u64,
    pub token: MessageLocationToken,
    pub message_id: MessageId,
    pub folder_id: FolderId,
    pub uid: u32,
    pub uidvalidity: u32,
    #[builder(default = "MessageFlags::default()")]
    pub flags: MessageFlags,
    pub internal_date: DateTime<Utc>,
    #[builder(default = "Utc::now()")]
    pub first_seen_at: DateTime<Utc>,
    #[builder(default = "Utc::now()")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_location_token_round_trips() {
        let token = MessageLocationToken::generate();
        let s = token.to_string();
        assert!(s.starts_with("ML_"));
        let parsed = MessageLocationToken::parse(&s).unwrap();
        assert_eq!(parsed.id(), token.id());
    }
}

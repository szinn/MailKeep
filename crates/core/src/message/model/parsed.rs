use chrono::{DateTime, Utc};

use crate::{
    message::NamedAddress,
    types::{ContentHash, EmailAddress},
};

/// Contract type from the parser (MK-5) to
/// `MessageService::record_parsed_message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMessage {
    /// Populated by parser (synthesized if absent in source).
    pub rfc822_message_id: String,
    /// Hash of the raw .eml plaintext.
    pub content_hash: ContentHash,
    pub subject: Option<String>,
    pub from_address: EmailAddress,
    pub from_name: Option<String>,
    pub to_addresses: Vec<NamedAddress>,
    pub cc_addresses: Vec<NamedAddress>,
    pub bcc_addresses: Vec<NamedAddress>,
    pub reply_to_addresses: Vec<NamedAddress>,
    pub sent_date: Option<DateTime<Utc>>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub snippet: String,
    pub size_bytes: i64,
    pub attachments: Vec<ParsedAttachment>,
}

/// Per-attachment contract from the parser. The parser has already written
/// bytes to `AttachmentStore` by the time this is constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAttachment {
    pub content_hash: ContentHash,
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
    pub is_inline: bool,
    pub content_id: Option<String>,
}

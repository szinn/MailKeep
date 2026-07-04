//! Tantivy schema definition and `Message` → document mapping.
//!
//! The schema is **immutable and versioned** by [`SCHEMA_VERSION`]. Any change
//! to the fields, their options, or the tokenizer must bump the version so the
//! index is rebuilt from source on next start (see the version sidecar in
//! `index.rs`).

use mk_core::message::{Message, NamedAddress};
use tantivy::{
    TantivyDocument,
    schema::{FAST, Field, INDEXED, IndexRecordOption, STORED, Schema, TextFieldIndexing, TextOptions},
    tokenizer::{LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer, TextAnalyzer},
};

/// Version of the on-disk index schema.
///
/// Bump this whenever the field layout, options, or tokenizer change. The
/// indexer compares it against the `schema_version` sidecar to decide whether a
/// full rebuild is required.
pub const SCHEMA_VERSION: u32 = 1;

/// Name of the registered English-stemming analyzer used by the stemmed TEXT
/// fields. The same name must be registered on the index's tokenizer manager
/// (see `index.rs`), otherwise queries against those fields fail.
pub const EN_STEM: &str = "en_stem";

/// Maximum token length kept by the stemmer analyzer. Longer tokens (typically
/// base64 blobs or hashes leaking from bodies) are dropped.
const MAX_TOKEN_LEN: usize = 40;

/// Handles to every [`Field`] in the schema, resolved once at build time so the
/// read service and indexer never look fields up by name on the hot path.
#[derive(Debug, Clone, Copy)]
pub struct Fields {
    /// `u64`, STORED + INDEXED — join key back to the database row and the
    /// delete-term key used by the indexer.
    pub message_id: Field,
    /// `u64`, INDEXED + FAST — account scoping / filtering.
    pub account_id: Field,
    /// TEXT (stemmed) + STORED — the message subject.
    pub subject: Field,
    /// TEXT (stemmed), not stored — the extracted plain-text body.
    pub body: Field,
    /// TEXT (stemmed) — sender address and display name, concatenated.
    pub from: Field,
    /// TEXT (stemmed) — all `to` + `cc` addresses and names, concatenated.
    pub to: Field,
    /// DATE, FAST — the message sent date; omitted from the document when
    /// absent.
    pub sent_date: Field,
    /// `u64`, INDEXED — `1` when the message has attachments, else `0`.
    pub has_attachments: Field,
    /// STORED (stored-only) — precomputed snippet for result display without a
    /// database round-trip.
    pub snippet: Field,
}

/// Build the [`Schema`] and its resolved [`Fields`] handles.
///
/// The returned schema is deterministic for a given [`SCHEMA_VERSION`]; the
/// order in which fields are added defines their internal ids, so never reorder
/// without bumping the version.
#[must_use]
pub fn build_schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();

    // Stemmed-and-stored (subject) and stemmed-only (body/from/to) text options,
    // both driven by the `en_stem` analyzer registered on the index.
    let stem_indexing = TextFieldIndexing::default()
        .set_tokenizer(EN_STEM)
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let stemmed_stored = TextOptions::default().set_indexing_options(stem_indexing.clone()).set_stored();
    let stemmed = TextOptions::default().set_indexing_options(stem_indexing);

    let message_id = builder.add_u64_field("message_id", STORED | INDEXED);
    let account_id = builder.add_u64_field("account_id", INDEXED | FAST);
    let subject = builder.add_text_field("subject", stemmed_stored);
    let body = builder.add_text_field("body", stemmed.clone());
    let from = builder.add_text_field("from", stemmed.clone());
    let to = builder.add_text_field("to", stemmed);
    let sent_date = builder.add_date_field("sent_date", FAST);
    let has_attachments = builder.add_u64_field("has_attachments", INDEXED);
    // STORED only: kept for display, never tokenized or searched.
    let snippet = builder.add_text_field("snippet", STORED);

    let schema = builder.build();
    let fields = Fields {
        message_id,
        account_id,
        subject,
        body,
        from,
        to,
        sent_date,
        has_attachments,
        snippet,
    };
    (schema, fields)
}

/// Construct the English-stemming analyzer registered under [`EN_STEM`].
///
/// Pipeline: simple tokenizer → drop over-long tokens → lowercase → English
/// stemmer. Must be registered on the index before searching or indexing the
/// stemmed fields.
#[must_use]
pub fn en_stem_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(MAX_TOKEN_LEN))
        .filter(LowerCaser)
        .filter(Stemmer::new(tantivy::tokenizer::Language::English))
        .build()
}

/// Map a [`Message`] and its extracted `body_text` into a [`TantivyDocument`].
///
/// The `sent_date` field is omitted entirely when the message has no sent date,
/// so range queries never match a sentinel value.
#[must_use]
pub fn to_document(fields: &Fields, msg: &Message, body_text: &str) -> TantivyDocument {
    let mut doc = TantivyDocument::default();

    doc.add_u64(fields.message_id, msg.id);
    doc.add_u64(fields.account_id, msg.account_id);
    doc.add_u64(fields.has_attachments, u64::from(msg.has_attachments));

    doc.add_text(fields.subject, msg.subject.clone().unwrap_or_default());
    doc.add_text(fields.body, body_text);
    doc.add_text(fields.snippet, &msg.snippet);

    doc.add_text(fields.from, from_text(msg));
    doc.add_text(fields.to, to_text(msg));

    if let Some(sent) = msg.sent_date {
        doc.add_date(fields.sent_date, tantivy::DateTime::from_timestamp_secs(sent.timestamp()));
    }

    doc
}

/// Sender address plus display name (when present), space-joined.
fn from_text(msg: &Message) -> String {
    let mut parts = vec![msg.from_address.as_str().to_string()];
    if let Some(name) = &msg.from_name {
        parts.push(name.clone());
    }
    parts.join(" ")
}

/// All recipient (`to` + `cc`) addresses and display names, space-joined.
fn to_text(msg: &Message) -> String {
    let mut parts = Vec::new();
    for addr in msg.to_addresses.iter().chain(msg.cc_addresses.iter()) {
        push_named_address(&mut parts, addr);
    }
    parts.join(" ")
}

fn push_named_address(parts: &mut Vec<String>, addr: &NamedAddress) {
    parts.push(addr.address.as_str().to_string());
    if let Some(name) = &addr.name {
        parts.push(name.clone());
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use mk_core::{
        message::{MessageBuilder, MessageToken},
        types::{ContentHash, EmailAddress},
    };
    use tantivy::schema::Value;

    use super::*;

    fn sample_message() -> Message {
        MessageBuilder::default()
            .id(42u64)
            .version(1u64)
            .token(MessageToken::generate())
            .account_id(7u64)
            .rfc822_message_id("<abc@example.com>".to_string())
            .content_hash(ContentHash::compute(b"body"))
            .subject(Some("Quarterly running report".to_string()))
            .from_address(EmailAddress::new("alice@example.com").unwrap())
            .from_name(Some("Alice Example".to_string()))
            .to_addresses(vec![NamedAddress {
                address: EmailAddress::new("bob@example.com").unwrap(),
                name: Some("Bob Builder".to_string()),
            }])
            .cc_addresses(vec![NamedAddress {
                address: EmailAddress::new("carol@example.com").unwrap(),
                name: None,
            }])
            .sent_date(Some(Utc.with_ymd_and_hms(2024, 3, 1, 12, 0, 0).unwrap()))
            .snippet("A short preview of the message".to_string())
            .size_bytes(1234)
            .has_attachments(true)
            .build()
            .unwrap()
    }

    #[test]
    fn schema_has_all_nine_fields() {
        let (schema, _fields) = build_schema();
        assert_eq!(schema.fields().count(), 9);
    }

    #[test]
    fn to_document_maps_scalar_fields() {
        let (_schema, fields) = build_schema();
        let msg = sample_message();
        let doc = to_document(&fields, &msg, "the body text");

        assert_eq!(doc.get_first(fields.message_id).and_then(|v| v.as_u64()), Some(42));
        assert_eq!(doc.get_first(fields.account_id).and_then(|v| v.as_u64()), Some(7));
        assert_eq!(doc.get_first(fields.has_attachments).and_then(|v| v.as_u64()), Some(1));
        assert_eq!(doc.get_first(fields.subject).and_then(|v| v.as_str()), Some("Quarterly running report"));
        assert_eq!(doc.get_first(fields.body).and_then(|v| v.as_str()), Some("the body text"));
        assert_eq!(doc.get_first(fields.snippet).and_then(|v| v.as_str()), Some("A short preview of the message"));
    }

    #[test]
    fn to_document_concatenates_from_and_to() {
        let (_schema, fields) = build_schema();
        let msg = sample_message();
        let doc = to_document(&fields, &msg, "");

        let from = doc.get_first(fields.from).and_then(|v| v.as_str()).unwrap();
        assert!(from.contains("alice@example.com"));
        assert!(from.contains("Alice Example"));

        let to = doc.get_first(fields.to).and_then(|v| v.as_str()).unwrap();
        assert!(to.contains("bob@example.com"));
        assert!(to.contains("Bob Builder"));
        // cc address with no name is still included
        assert!(to.contains("carol@example.com"));
    }

    #[test]
    fn to_document_has_attachments_zero_when_absent() {
        let (_schema, fields) = build_schema();
        let mut msg = sample_message();
        msg.has_attachments = false;
        let doc = to_document(&fields, &msg, "");
        assert_eq!(doc.get_first(fields.has_attachments).and_then(|v| v.as_u64()), Some(0));
    }

    #[test]
    fn to_document_omits_sent_date_when_none() {
        let (_schema, fields) = build_schema();
        let mut msg = sample_message();
        msg.sent_date = None;
        let doc = to_document(&fields, &msg, "");
        assert!(doc.get_first(fields.sent_date).is_none());
    }

    #[test]
    fn to_document_subject_defaults_to_empty_when_none() {
        let (_schema, fields) = build_schema();
        let mut msg = sample_message();
        msg.subject = None;
        let doc = to_document(&fields, &msg, "");
        assert_eq!(doc.get_first(fields.subject).and_then(|v| v.as_str()), Some(""));
    }
}

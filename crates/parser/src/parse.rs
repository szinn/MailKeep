use chrono::{DateTime, Utc};
use mail_parser::{MessageParser, MimeHeaders};
use mk_core::{
    Error,
    message::NamedAddress,
    types::{ContentHash, EmailAddress},
};

/// Sentinel address used when a message has no parseable `From:` header.
const SENTINEL_FROM: &str = "unknown@mailkeep.invalid";
const SNIPPET_MAX_CHARS: usize = 200;

/// One extracted attachment: decoded bytes + metadata. The handler writes the
/// bytes to `AttachmentStore` and pairs the returned hash with this metadata.
#[derive(Debug)]
pub struct ExtractedAttachment {
    /// Decoded attachment bytes.
    pub bytes: Vec<u8>,
    /// Filename from `Content-Disposition` or `Content-Type` `name` parameter.
    pub filename: Option<String>,
    /// MIME content type string (e.g. `application/pdf`).
    pub content_type: String,
    /// Whether this part has a `Content-ID` (i.e. is inline).
    pub is_inline: bool,
    /// The `Content-ID` value, if present.
    pub content_id: Option<String>,
}

/// Fully parsed message minus attachment content hashes.
///
/// The handler uses this to write attachment blobs and record the message.
#[derive(Debug)]
pub struct ParsedEml {
    /// RFC 822 `Message-ID` header, or a synthesized
    /// `<{hex}@mailkeep.invalid>`.
    pub rfc822_message_id: String,
    /// `Subject` header, if present.
    pub subject: Option<String>,
    /// First `From` address (or sentinel `unknown@mailkeep.invalid`).
    pub from_address: EmailAddress,
    /// Display name of the first `From` address, if present.
    pub from_name: Option<String>,
    /// All `To` recipients.
    pub to_addresses: Vec<NamedAddress>,
    /// All `Cc` recipients.
    pub cc_addresses: Vec<NamedAddress>,
    /// All `Bcc` recipients.
    pub bcc_addresses: Vec<NamedAddress>,
    /// All `Reply-To` addresses.
    pub reply_to_addresses: Vec<NamedAddress>,
    /// Parsed `Date` header converted to UTC, if present and valid.
    pub sent_date: Option<DateTime<Utc>>,
    /// `In-Reply-To` header value, if present.
    pub in_reply_to: Option<String>,
    /// `References` header values.
    pub references: Vec<String>,
    /// Whitespace-collapsed, tag-stripped snippet ≤ 200 chars.
    pub snippet: String,
    /// Size of the raw bytes.
    pub size_bytes: i64,
    /// Decoded attachments with metadata.
    pub attachments: Vec<ExtractedAttachment>,
}

/// Canonicalize a Message-ID to the bracketed `<id>` form used across the
/// codebase. `mail-parser` strips the angle brackets from `message_id()`.
fn canonical_message_id(id: &str) -> String {
    let trimmed = id.trim();
    if trimmed.starts_with('<') && trimmed.ends_with('>') {
        trimmed.to_string()
    } else {
        format!("<{trimmed}>")
    }
}

/// Parse raw `.eml` bytes. `content_hash` is used to synthesize a stable
/// `Message-ID` when the header is absent.
///
/// Returns `Err(Error::Validation)` (non-transient — terminal for the job) if
/// the bytes are not a parseable message.
pub fn parse_eml(content_hash: ContentHash, raw: &[u8]) -> Result<ParsedEml, Error> {
    let msg = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| Error::Validation("mail-parser could not parse message".into()))?;

    let rfc822_message_id = msg
        .message_id()
        .map_or_else(|| format!("<{}@mailkeep.invalid>", content_hash.as_hex()), canonical_message_id);

    let (from_address, from_name) = first_address(msg.from()).unwrap_or_else(|| (EmailAddress::new(SENTINEL_FROM).expect("sentinel is valid"), None));

    let snippet = build_snippet(&msg);

    let attachments = msg
        .attachments()
        .map(|part| ExtractedAttachment {
            bytes: part.contents().to_vec(),
            filename: part.attachment_name().map(str::to_string),
            content_type: content_type_string(part),
            content_id: part.content_id().map(str::to_string),
            is_inline: part.content_id().is_some(),
        })
        .collect();

    Ok(ParsedEml {
        rfc822_message_id,
        subject: msg.subject().map(str::to_string),
        from_address,
        from_name,
        to_addresses: address_list(msg.to()),
        cc_addresses: address_list(msg.cc()),
        bcc_addresses: address_list(msg.bcc()),
        reply_to_addresses: address_list(msg.reply_to()),
        sent_date: msg.date().and_then(mail_dt_to_chrono),
        in_reply_to: msg.in_reply_to().as_text().map(str::to_string),
        references: header_text_list(msg.references()),
        snippet,
        size_bytes: raw.len() as i64,
        attachments,
    })
}

/// First address of an address header as `(EmailAddress, name)`.
fn first_address(addr: Option<&mail_parser::Address<'_>>) -> Option<(EmailAddress, Option<String>)> {
    let a = addr?.first()?;
    let email = a.address()?;
    Some((EmailAddress::new(email).ok()?, a.name().map(str::to_string)))
}

/// All addresses of an address header as `NamedAddress`, skipping unparseable
/// ones.
fn address_list(addr: Option<&mail_parser::Address<'_>>) -> Vec<NamedAddress> {
    let Some(list) = addr else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|a| {
            let email = a.address()?;
            Some(NamedAddress {
                address: EmailAddress::new(email).ok()?,
                name: a.name().map(str::to_string),
            })
        })
        .collect()
}

pub(crate) fn content_type_string(part: &mail_parser::MessagePart<'_>) -> String {
    part.content_type().map_or_else(
        || "application/octet-stream".to_string(),
        |ct| match ct.subtype() {
            Some(sub) => format!("{}/{}", ct.ctype(), sub),
            None => ct.ctype().to_string(),
        },
    )
}

fn header_text_list(value: &mail_parser::HeaderValue<'_>) -> Vec<String> {
    match value {
        mail_parser::HeaderValue::Text(t) => vec![t.to_string()],
        mail_parser::HeaderValue::TextList(list) => list.iter().map(ToString::to_string).collect(),
        _ => Vec::new(),
    }
}

/// Convert a `mail_parser::DateTime` to a chrono `DateTime<Utc>`.
///
/// Returns `None` if the timestamp is out of range for chrono.
fn mail_dt_to_chrono(dt: &mail_parser::DateTime) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(dt.to_timestamp(), 0)
}

/// Snippet: prefer text/plain body, else tag-stripped HTML; collapse whitespace
/// and take the first 200 chars on a char boundary.
fn build_snippet(msg: &mail_parser::Message<'_>) -> String {
    let text = msg
        .body_text(0)
        .map(std::borrow::Cow::into_owned)
        .or_else(|| msg.body_html(0).map(|h| html_to_text(&h)))
        .unwrap_or_default();
    normalize_snippet(&text)
}

/// Minimal HTML-to-text: drop tags. Snippet-only fidelity, no entity table.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn normalize_snippet(text: &str) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(SNIPPET_MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash() -> ContentHash {
        ContentHash::compute(b"fixture")
    }

    #[test]
    fn unparseable_is_terminal_validation_error() {
        let err = parse_eml(hash(), b"").unwrap_err();
        assert!(!err.is_transient(), "parse failure must be terminal");
    }

    #[test]
    fn missing_message_id_is_synthesized_from_hash() {
        let raw = include_bytes!("../fixtures/no_message_id.eml");
        let parsed = parse_eml(hash(), raw).unwrap();
        assert_eq!(parsed.rfc822_message_id, format!("<{}@mailkeep.invalid>", hash().as_hex()));
    }

    #[test]
    fn missing_from_uses_sentinel() {
        let raw = include_bytes!("../fixtures/no_from.eml");
        let parsed = parse_eml(hash(), raw).unwrap();
        assert_eq!(parsed.from_address.as_str(), "unknown@mailkeep.invalid");
    }

    #[test]
    fn alternative_snippet_prefers_plain_text() {
        let raw = include_bytes!("../fixtures/alternative.eml");
        let parsed = parse_eml(hash(), raw).unwrap();
        assert!(parsed.snippet.len() <= 200);
        assert!(!parsed.snippet.contains('<'), "snippet must not contain HTML tags");
    }

    #[test]
    fn html_only_snippet_strips_tags() {
        let raw = include_bytes!("../fixtures/html_only.eml");
        let parsed = parse_eml(hash(), raw).unwrap();
        assert!(parsed.snippet.len() <= 200);
        assert!(!parsed.snippet.contains('<'), "snippet must not contain HTML tags");
    }

    #[test]
    fn real_message_id_is_canonicalized_with_brackets() {
        let raw = include_bytes!("../fixtures/mixed_attachment.eml");
        let parsed = parse_eml(hash(), raw).unwrap();
        assert_eq!(
            parsed.rfc822_message_id, "<mixed001@example.com>",
            "message id must be bracketed, got {}",
            parsed.rfc822_message_id
        );
    }

    #[test]
    fn mixed_extracts_attachment() {
        let raw = include_bytes!("../fixtures/mixed_attachment.eml");
        let parsed = parse_eml(hash(), raw).unwrap();
        assert_eq!(parsed.attachments.len(), 1);
        assert!(parsed.attachments[0].filename.is_some());
        assert!(!parsed.attachments[0].bytes.is_empty());
    }

    #[test]
    fn snapshot_parsed_shape() {
        let raw = include_bytes!("../fixtures/mixed_attachment.eml");
        let parsed = parse_eml(ContentHash::compute(b"snapshot-fixture"), raw).unwrap();
        // Render a stable, snapshot-friendly view (avoid raw bytes).
        let view = format!(
            "message_id: {}\nsubject: {:?}\nfrom: {} ({:?})\nto: {:?}\nsnippet: {}\nsize_bytes: {}\nattachments: {:?}",
            parsed.rfc822_message_id,
            parsed.subject,
            parsed.from_address.as_str(),
            parsed.from_name,
            parsed.to_addresses.iter().map(|a| a.address.as_str().to_string()).collect::<Vec<_>>(),
            parsed.snippet,
            parsed.size_bytes,
            parsed
                .attachments
                .iter()
                .map(|a| (a.filename.clone(), a.content_type.clone(), a.is_inline))
                .collect::<Vec<_>>(),
        );
        insta::assert_snapshot!(view);
    }
}

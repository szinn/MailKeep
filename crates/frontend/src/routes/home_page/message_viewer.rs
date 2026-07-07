use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
#[cfg(feature = "server")]
use {
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    mk_core::{CoreServices, Error, message::MessageToken},
    mk_parser::render_message_for_display,
    std::sync::Arc,
};

/// One attachment row for the viewer (metadata only; download is MK-24).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AttachmentDto {
    pub token: String,
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
}

/// Everything the viewer renders for one message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct MessageViewDto {
    pub token: String,
    pub subject: Option<String>,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub date_display: Option<String>,
    /// Sanitized HTML for the iframe srcdoc; `None` when there is no HTML body
    /// or the raw bytes are unavailable.
    pub body_html: Option<String>,
    /// Plain-text body when there is no HTML.
    pub body_text: Option<String>,
    pub has_remote_content: bool,
    pub attachments: Vec<AttachmentDto>,
}

/// Fetch, decrypt, render, and map one message for the authenticated user.
#[post(
    "/api/v1/home/message",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn view_message(token: String, load_remote: bool) -> Result<MessageViewDto, ServerFnError> {
    let user = authenticated_user(&auth_session)?;
    let msg_token: MessageToken = token.parse().map_err(to_server_err)?;

    let Some((message, attachments)) = core_services
        .message_service
        .get_message_with_attachments(user.id(), msg_token)
        .await
        .map_err(to_server_err)?
    else {
        return Err(ServerFnError::new("message not found"));
    };

    // Decrypt-on-demand. A missing blob yields a headers-only view rather than
    // an error, so the user still sees metadata + attachments.
    let rendered = match core_services.raw_storage_service.get(message.account_id, &message.content_hash).await {
        Ok(raw) => Some(render_message_for_display(&raw, load_remote).map_err(to_server_err)?),
        Err(Error::BlobNotFound { .. }) => None,
        Err(e) => return Err(to_server_err(e)),
    };
    Ok(build_dto(&message, &attachments, rendered))
}

/// Pure DTO assembly, unit-tested without axum plumbing. `rendered` is `None`
/// on the blob-missing path (headers + attachments, no body).
#[cfg(feature = "server")]
fn build_dto(
    message: &mk_core::message::Message,
    attachments: &[mk_core::message::MessageAttachment],
    rendered: Option<mk_parser::RenderedBody>,
) -> MessageViewDto {
    use chrono::Utc;

    use crate::routes::account_add_page::dtos::relative_time;

    let display_addr = |a: &mk_core::message::NamedAddress| a.name.clone().unwrap_or_else(|| a.address.as_str().to_string());
    let from = message.from_name.clone().unwrap_or_else(|| message.from_address.as_str().to_string());
    let (body_html, body_text, has_remote_content) = match rendered {
        Some(r) => (r.html, r.text, r.has_remote_content),
        None => (None, None, false),
    };

    MessageViewDto {
        token: message.token.to_string(),
        subject: message.subject.clone(),
        from,
        to: message.to_addresses.iter().map(display_addr).collect(),
        cc: message.cc_addresses.iter().map(display_addr).collect(),
        date_display: message.sent_date.map(|t| relative_time(t, Utc::now())),
        body_html,
        body_text,
        has_remote_content,
        attachments: attachments
            .iter()
            .map(|a| AttachmentDto {
                token: a.token.to_string(),
                filename: a.filename.clone(),
                content_type: a.content_type.clone(),
                size_bytes: a.size_bytes,
            })
            .collect(),
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use mk_core::{
        message::{MessageAttachment, MessageAttachmentToken, MessageBuilder, NamedAddress},
        types::{ContentHash, EmailAddress},
    };
    use mk_parser::RenderedBody;

    use super::*;

    fn sample_message() -> mk_core::message::Message {
        MessageBuilder::default()
            .id(7u64)
            .version(1u64)
            .token(MessageToken::new(7))
            .account_id(3u64)
            .rfc822_message_id("<m@x.com>".to_string())
            .content_hash(ContentHash::compute(b"body"))
            .subject(Some("Hi".to_string()))
            .sent_date(Some(chrono::Utc::now() - chrono::Duration::minutes(5)))
            .from_address(EmailAddress::new("alice@example.com").unwrap())
            .from_name(Some("Alice".to_string()))
            .to_addresses(vec![
                NamedAddress {
                    address: EmailAddress::new("bob@example.com").unwrap(),
                    name: Some("Bob Jones".to_string()),
                },
                NamedAddress {
                    address: EmailAddress::new("carol@example.com").unwrap(),
                    name: None,
                },
            ])
            .cc_addresses(vec![NamedAddress {
                address: EmailAddress::new("dan@example.com").unwrap(),
                name: None,
            }])
            .snippet("preview".to_string())
            .size_bytes(10i64)
            .build()
            .unwrap()
    }

    fn sample_attachment() -> MessageAttachment {
        MessageAttachment {
            id: 9,
            version: 1,
            token: MessageAttachmentToken::new(9),
            message_id: 7,
            account_id: 3,
            content_hash: ContentHash::compute(b"att"),
            filename: Some("report.pdf".into()),
            content_type: "application/pdf".into(),
            size_bytes: 2048,
            is_inline: false,
            content_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn build_dto_maps_headers_body_and_attachments() {
        let rendered = RenderedBody {
            html: Some("<p>hi</p>".into()),
            text: None,
            has_remote_content: true,
        };
        let dto = build_dto(&sample_message(), &[sample_attachment()], Some(rendered));
        assert_eq!(dto.subject.as_deref(), Some("Hi"));
        assert_eq!(dto.from, "Alice");
        // `display_addr` prefers the name when present, else falls back to the address.
        assert_eq!(dto.to, vec!["Bob Jones".to_string(), "carol@example.com".to_string()]);
        assert_eq!(dto.cc, vec!["dan@example.com".to_string()]);
        // `sent_date` is present, so the relative-time mapping runs.
        assert_eq!(dto.date_display.as_deref(), Some("5m ago"));
        assert_eq!(dto.body_html.as_deref(), Some("<p>hi</p>"));
        assert!(dto.has_remote_content);
        assert_eq!(dto.attachments.len(), 1);
        assert_eq!(dto.attachments[0].filename.as_deref(), Some("report.pdf"));
    }

    #[test]
    fn build_dto_from_falls_back_to_address_without_name() {
        let mut message = sample_message();
        message.from_name = None;
        let dto = build_dto(&message, &[], None);
        assert_eq!(dto.from, "alice@example.com");
    }

    #[test]
    fn build_dto_blob_missing_is_headers_only() {
        let dto = build_dto(&sample_message(), &[sample_attachment()], None);
        assert!(dto.body_html.is_none());
        assert!(dto.body_text.is_none());
        assert!(!dto.has_remote_content);
        assert_eq!(dto.attachments.len(), 1, "attachments still listed without a body");
    }
}

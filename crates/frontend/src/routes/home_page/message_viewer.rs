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

/// Assemble the iframe `srcdoc`: a minimal document wrapping the sanitized body
/// with a strict CSP and a new-tab base target. `img-src` widens only when the
/// user has opted into remote content.
///
/// Emails are always rendered as a light document, independent of MailKeep's
/// own light/dark theme: the sanitizer strips all author CSS (`<style>` and
/// `style=`), so a body with no explicit surface is transparent and lets the
/// app's dark backdrop bleed through behind the email's default-black text.
/// `color-scheme: light` alone only pins the UA scheme — it does not guarantee
/// an opaque surface. We force an opaque white background and dark default text
/// with the legacy `bgcolor`/`text` body attributes: these are presentational
/// HTML hints, not CSS, so they take effect under the strict `default-src
/// 'none'` CSP (which would otherwise block an injected `<style>`).
fn build_srcdoc(body_html: &str, load_remote: bool) -> String {
    let img_src = if load_remote {
        "img-src 'self' data: https: http:"
    } else {
        "img-src 'self' data:"
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; base-uri 'none'; \
         {img_src}; font-src 'self'\"><base target=\"_blank\"><meta name=\"color-scheme\" content=\"light\"></head><body bgcolor=\"#ffffff\" \
         text=\"#111827\">{body_html}</body></html>"
    )
}

#[component]
pub(crate) fn MessageViewer(token: String) -> Element {
    let mut load_remote = use_signal(|| false);

    // Keyed on token + load_remote so a re-fetch (opt-in) re-runs cleanly.
    let view = use_resource({
        let token = token.clone();
        move || {
            let token = token.clone();
            async move { view_message(token, load_remote()).await.map_err(|e| e.to_string()) }
        }
    });

    let iframe_id = format!("mk-msg-frame-{token}");

    rsx! {
        div { class: "flex h-full flex-col",
            // Header bar with a ✕ that closes the viewer.
            div { class: "flex items-center justify-between border-b border-gray-200 px-4 py-3 dark:border-slate-700",
                h2 { class: "truncate text-sm font-semibold text-gray-900 dark:text-slate-100", "Message" }
                button {
                    class: "shrink-0 rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-slate-700",
                    title: "Close",
                    onclick: move |_| { *crate::components::OPEN_MESSAGE.write() = None; },
                    "✕"
                }
            }
            div { class: "flex-1 overflow-auto",
                match view() {
                    None => rsx! {
                        div { class: "px-4 py-6 text-sm text-gray-400 dark:text-slate-500", "Loading…" }
                    },
                    Some(Err(e)) => rsx! {
                        div { class: "px-4 py-3 text-sm text-red-600 dark:text-red-400", "{e}" }
                    },
                    Some(Ok(msg)) => rsx! {
                        // Headers.
                        div { class: "space-y-1 border-b border-gray-100 px-4 py-3 dark:border-slate-700",
                            div { class: "text-base font-semibold text-gray-900 dark:text-slate-100",
                                "{msg.subject.clone().unwrap_or_else(|| \"(no subject)\".to_string())}"
                            }
                            div { class: "text-sm text-gray-600 dark:text-slate-300", "From: {msg.from}" }
                            if !msg.to.is_empty() {
                                div { class: "text-xs text-gray-500 dark:text-slate-400", "To: {msg.to.join(\", \")}" }
                            }
                            if !msg.cc.is_empty() {
                                div { class: "text-xs text-gray-500 dark:text-slate-400", "Cc: {msg.cc.join(\", \")}" }
                            }
                            if let Some(when) = msg.date_display.clone() {
                                div { class: "text-xs text-gray-400 dark:text-slate-500", "{when}" }
                            }
                        }
                        // Remote-content opt-in bar.
                        if msg.has_remote_content && !load_remote() {
                            div { class: "flex items-center justify-between gap-2 bg-amber-50 px-4 py-2 text-xs text-amber-800 dark:bg-amber-900/30 dark:text-amber-200",
                                span { "Remote images are blocked to protect your privacy." }
                                button {
                                    class: "shrink-0 rounded border border-amber-300 px-2 py-1 font-medium hover:bg-amber-100 dark:border-amber-700 dark:hover:bg-amber-900/50",
                                    onclick: move |_| load_remote.set(true),
                                    "Load remote content"
                                }
                            }
                        }
                        // Body.
                        if let Some(html) = msg.body_html.clone() {
                            iframe {
                                id: "{iframe_id}",
                                class: "w-full border-0",
                                title: "Message content",
                                srcdoc: build_srcdoc(&html, load_remote()),
                                "sandbox": "allow-popups allow-popups-to-escape-sandbox allow-same-origin",
                                referrerpolicy: "no-referrer",
                                // Auto-height once the srcdoc document parses. No
                                // scripts run inside the frame; allow-same-origin
                                // permits reading scrollHeight from the parent.
                                // For an inline srcdoc the load event can fire
                                // before onload is attached, so also size
                                // immediately if the frame is already complete.
                                onmounted: {
                                    let id = iframe_id.clone();
                                    move |_| {
                                        let id = id.clone();
                                        spawn(async move {
                                            let js = format!(
                                                "const f=document.getElementById('{id}');\
                                                 if(f){{const a=()=>{{try{{f.style.height=(f.contentDocument.body.scrollHeight+24)+'px';}}catch(e){{}}}};\
                                                 f.onload=a;\
                                                 if(f.contentDocument&&f.contentDocument.readyState==='complete')a();}}"
                                            );
                                            let _ = document::eval(&js).await;
                                        });
                                    }
                                },
                            }
                        } else if let Some(text) = msg.body_text.clone() {
                            pre { class: "whitespace-pre-wrap px-4 py-3 text-sm text-gray-800 dark:text-slate-200", "{text}" }
                        } else {
                            div { class: "px-4 py-6 text-sm text-gray-400 dark:text-slate-500", "This message has no displayable content." }
                        }
                        // Attachments.
                        if !msg.attachments.is_empty() {
                            div { class: "border-t border-gray-100 px-4 py-3 dark:border-slate-700",
                                div { class: "mb-2 text-xs font-semibold uppercase text-gray-500 dark:text-slate-400", "Attachments" }
                                ul { class: "space-y-1",
                                    for att in msg.attachments.iter().cloned() {
                                        li {
                                            key: "{att.token}",
                                            class: "flex items-center justify-between gap-2 text-sm text-gray-700 dark:text-slate-200",
                                            span { class: "truncate",
                                                "📎 {att.filename.clone().unwrap_or_else(|| att.content_type.clone())}"
                                            }
                                            button {
                                                class: "shrink-0 rounded border border-gray-200 px-2 py-1 text-xs text-gray-500 hover:bg-gray-50 dark:border-slate-700 dark:text-slate-300 dark:hover:bg-slate-700",
                                                title: "Download (coming soon)",
                                                // MK-24 download seam — no-op for now.
                                                onclick: move |_| {},
                                                "Download"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
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
    fn build_srcdoc_pins_security_contract() {
        // Blocked (default) mode: strict CSP, no remote schemes, new-tab base,
        // and the body passed through verbatim.
        let blocked = build_srcdoc("<p>x</p>", false);
        assert!(blocked.contains("default-src 'none'"));
        assert!(blocked.contains("base-uri 'none'"));
        assert!(blocked.contains("img-src 'self' data:"));
        assert!(!blocked.contains("https:"), "blocked mode must not permit remote image schemes");
        assert!(blocked.contains("<base target=\"_blank\">"));
        assert!(blocked.contains("<p>x</p>"));

        // Emails render as an opaque light document regardless of MailKeep's
        // theme: pin the UA scheme and force a white surface with dark text via
        // presentational body attributes (CSP-safe; a `<style>` would be blocked
        // by `default-src 'none'`). Without this the dark app backdrop bleeds
        // through the transparent frame behind the email's default-black text.
        assert!(blocked.contains("<meta name=\"color-scheme\" content=\"light\">"));
        assert!(blocked.contains("bgcolor=\"#ffffff\""), "email frame must have an opaque light background");
        assert!(blocked.contains("text=\"#111827\""), "email frame must set a dark default text color");
        assert!(!blocked.contains("<style"), "no injected <style>: it would be blocked by the strict CSP");

        // Opt-in mode widens img-src to remote schemes.
        let allowed = build_srcdoc("<p>x</p>", true);
        assert!(allowed.contains("img-src 'self' data: https: http:"));
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

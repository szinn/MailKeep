use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// Wire-safe shape for one message row. Shared contract consumed by the account
/// message list (MK-21) and by search results (MK-22, which composes this
/// inside its own result DTO), and carrying the `token` the viewer (MK-23)
/// opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct MessageRowDto {
    /// M_… message token — row key and viewer navigation.
    pub token: String,
    /// Sender display name when present; the row renders name-or-address.
    pub from_name: Option<String>,
    pub from_address: String,
    /// None when the message has no subject; the row renders "(no subject)".
    pub subject: Option<String>,
    pub snippet: String,
    /// Pre-formatted on the server; None when the message has no sent_date.
    pub sent_at_display: Option<String>,
    pub has_attachments: bool,
    pub attachment_count: i32,
}

#[cfg(feature = "server")]
pub(crate) use mapping::*;

#[cfg(feature = "server")]
mod mapping {
    use chrono::Utc;
    use mk_core::message::Message;

    use super::MessageRowDto;
    use crate::routes::account_add_page::dtos::relative_time;

    /// Map a domain `Message` to its wire row, pre-formatting the sent date as
    /// a relative string (same idiom as `account_to_summary`'s
    /// `last_synced`).
    pub(crate) fn message_to_row(m: &Message) -> MessageRowDto {
        MessageRowDto {
            token: m.token.to_string(),
            from_name: m.from_name.clone(),
            from_address: m.from_address.as_str().to_string(),
            subject: m.subject.clone(),
            snippet: m.snippet.clone(),
            sent_at_display: m.sent_date.map(|t| relative_time(t, Utc::now())),
            has_attachments: m.has_attachments,
            attachment_count: m.attachment_count,
        }
    }
}

/// One message row: from · subject · snippet · time · attachment indicator.
/// Navigation-agnostic — clicking fires `on_open` with the message token so the
/// parent decides where to go (the viewer route lands in MK-23).
#[component]
pub(crate) fn MessageRow(row: MessageRowDto, on_open: EventHandler<String>) -> Element {
    let from = row.from_name.clone().unwrap_or_else(|| row.from_address.clone());
    let subject = row.subject.clone().unwrap_or_else(|| "(no subject)".to_string());
    let token = row.token.clone();
    rsx! {
        li {
            class: "flex cursor-pointer items-start gap-3 px-4 py-3 hover:bg-gray-50 dark:hover:bg-slate-700/50",
            onclick: move |_| on_open.call(token.clone()),
            div { class: "min-w-0 flex-1",
                div { class: "flex items-center justify-between gap-2",
                    span { class: "truncate text-sm font-medium text-gray-900 dark:text-slate-100", "{from}" }
                    if let Some(when) = row.sent_at_display.clone() {
                        span { class: "shrink-0 text-xs text-gray-400 dark:text-slate-500", "{when}" }
                    }
                }
                div { class: "truncate text-sm text-gray-700 dark:text-slate-200", "{subject}" }
                div { class: "flex items-center gap-1",
                    if row.has_attachments {
                        span {
                            class: "shrink-0 text-xs text-gray-400 dark:text-slate-500",
                            title: "Has attachments",
                            "📎"
                        }
                    }
                    span { class: "truncate text-xs text-gray-400 dark:text-slate-500", "{row.snippet}" }
                }
            }
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use mk_core::{
        message::{Message, MessageBuilder, MessageToken},
        types::{ContentHash, EmailAddress},
    };

    use super::*;

    fn sample() -> Message {
        MessageBuilder::default()
            .id(7u64)
            .version(1u64)
            .token(MessageToken::new(7))
            .account_id(3u64)
            .rfc822_message_id("<m@x.com>".to_string())
            .content_hash(ContentHash::compute(b"body"))
            .from_address(EmailAddress::new("alice@example.com").unwrap())
            .snippet("preview".to_string())
            .size_bytes(10i64)
            .build()
            .unwrap()
    }

    #[test]
    fn message_to_row_uses_address_when_no_name_and_keeps_none_subject() {
        let mut m = sample();
        m.from_name = None;
        m.subject = None;
        m.sent_date = None;
        let row = message_to_row(&m);
        assert_eq!(row.from_address, "alice@example.com");
        assert_eq!(row.from_name, None);
        assert_eq!(row.subject, None); // the component supplies the "(no subject)" fallback
        assert_eq!(row.sent_at_display, None);
        assert_eq!(row.token, MessageToken::new(7).to_string());
    }

    #[test]
    fn message_to_row_formats_sent_date_when_present() {
        let mut m = sample();
        m.sent_date = Some(chrono::Utc::now() - chrono::Duration::minutes(5));
        let row = message_to_row(&m);
        assert_eq!(row.sent_at_display.as_deref(), Some("5m ago"));
    }
}

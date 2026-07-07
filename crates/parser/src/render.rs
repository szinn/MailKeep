use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use mail_parser::{MessageParser, MimeHeaders, PartType};
use mk_core::Error;

use crate::parse::content_type_string;

/// Display-ready message body produced from raw `.eml` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedBody {
    /// Sanitized HTML ready for the iframe srcdoc, when the message has a
    /// usable HTML body.
    pub html: Option<String>,
    /// Plain-text body (already text, not derived from HTML), when there is no
    /// usable HTML.
    pub text: Option<String>,
    /// True when the render stripped at least one remote resource because
    /// `load_remote` was false — drives the "Load remote content" affordance.
    pub has_remote_content: bool,
}

/// Re-parse the raw `.eml` and produce a display-ready body.
///
/// Total contract on content: sanitization never errors (ammonia can't). The
/// only `Err` is an unparseable message. `load_remote=false` blocks remote
/// resources (dropping their `src`) and sets `has_remote_content`; `true`
/// keeps the real remote `src`. Inline `cid:` images are always resolved to
/// `data:` URIs and are never gated.
pub fn render_message_for_display(raw: &[u8], load_remote: bool) -> Result<RenderedBody, Error> {
    let msg = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| Error::Validation("mail-parser could not parse message".into()))?;

    // Prefer a genuine HTML body part. `mail-parser` back-fills `html_body`
    // from the text part when a message has text but no HTML alternative
    // (see `body_html`/`body_text` docs), so we must check the underlying
    // part type rather than trust `body_html(0)` returning `Some` — otherwise
    // a plain-text-only message would be misreported as having HTML.
    let has_real_html = matches!(msg.html_part(0).map(|p| &p.body), Some(PartType::Html(_)));
    if !has_real_html {
        let text = msg.body_text(0).map(Cow::into_owned);
        return Ok(RenderedBody {
            html: None,
            text,
            has_remote_content: false,
        });
    }
    // Safe: `has_real_html` confirms `html_body[0]` resolves to an actual
    // HTML part, so `body_html(0)` returns `Some`.
    let html = msg.body_html(0).expect("checked has_real_html above");

    let cid_map = build_cid_map(&msg);
    let removed_remote = Arc::new(AtomicBool::new(false));
    let sanitized = sanitize_html(&html, load_remote, cid_map, &removed_remote);

    // Sanitizing to whitespace-only (e.g. HTML that was entirely script/style)
    // is treated as "no HTML" — fall back to text/plain.
    if sanitized.trim().is_empty() {
        let text = msg.body_text(0).map(Cow::into_owned);
        return Ok(RenderedBody {
            html: None,
            text,
            has_remote_content: false,
        });
    }

    Ok(RenderedBody {
        html: Some(sanitized),
        text: None,
        has_remote_content: removed_remote.load(Ordering::Relaxed),
    })
}

/// Strip angle brackets and surrounding whitespace from a Content-ID / `cid:`
/// reference so the two forms compare equal.
fn normalize_cid(id: &str) -> String {
    id.trim().trim_start_matches('<').trim_end_matches('>').to_string()
}

/// Map each inline part's normalized Content-ID to a `data:` URI of its bytes.
fn build_cid_map(msg: &mail_parser::Message<'_>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in msg.attachments() {
        let Some(cid) = part.content_id() else { continue };
        let mime = content_type_string(part);
        let encoded = BASE64.encode(part.contents());
        map.insert(normalize_cid(cid), format!("data:{mime};base64,{encoded}"));
    }
    map
}

/// Rewrite an `<img src>` value: resolve `cid:` to a `data:` URI; block or keep
/// remote URLs per `load_remote`; pass everything else through.
fn rewrite_img_src(value: &str, load_remote: bool, cid_map: &HashMap<String, String>, removed_remote: &Arc<AtomicBool>) -> Option<String> {
    let trimmed = value.trim();
    // The `cid` URI scheme is case-insensitive (RFC 2392), so match the prefix
    // case-insensitively while preserving the original substring after it.
    if trimmed.len() >= 4 && trimmed[..4].eq_ignore_ascii_case("cid:") {
        let cid = &trimmed[4..];
        // Unknown cid -> None drops the src (neutral broken-image placeholder).
        return cid_map.get(&normalize_cid(cid)).cloned();
    }
    let lower = trimmed.to_ascii_lowercase();
    let is_remote = lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("//");
    if is_remote {
        if load_remote {
            return Some(value.to_string());
        }
        removed_remote.store(true, Ordering::Relaxed);
        return None; // block: drop src
    }
    // data: URIs (already inline) and anything else the scheme filter allows.
    Some(value.to_string())
}

/// Sanitize with ammonia: tighten URL schemes, force link hardening, delete
/// `<style>`/`<script>` content, and rewrite `<img src>` via the attribute
/// filter. The default ammonia allowlist already excludes `script`, `style`,
/// `iframe`, `object`, `embed`, `form`, `input`, `svg`, `math` and the inline
/// `style=` attribute, so we do not re-add them.
fn sanitize_html(html: &str, load_remote: bool, cid_map: HashMap<String, String>, removed_remote: &Arc<AtomicBool>) -> String {
    let removed_remote = Arc::clone(removed_remote);
    // `data`/`cid` are included so the attribute filter can resolve inline
    // images to data: URIs regardless of ammonia's scheme-check ordering.
    let schemes: HashSet<&str> = ["http", "https", "mailto", "tel", "data", "cid"].into_iter().collect();
    let clean_content: HashSet<&str> = ["script", "style"].into_iter().collect();

    let mut builder = ammonia::Builder::default();
    builder
        .url_schemes(schemes)
        .clean_content_tags(clean_content)
        .link_rel(Some("noopener noreferrer"))
        .add_tag_attributes("a", &["target"])
        .set_tag_attribute_value("a", "target", "_blank")
        .attribute_filter(move |element, attribute, value| {
            if element == "img" && attribute == "src" {
                return rewrite_img_src(value, load_remote, &cid_map, &removed_remote).map(Cow::Owned);
            }
            Some(Cow::Borrowed(value))
        });
    builder.clean(html).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(fixture: &[u8], load_remote: bool) -> RenderedBody {
        render_message_for_display(fixture, load_remote).unwrap()
    }

    #[test]
    fn unparseable_is_err() {
        render_message_for_display(b"", false).unwrap_err();
    }

    #[test]
    fn text_only_message_has_no_html() {
        let out = render(include_bytes!("../fixtures/render_text_only.eml"), false);
        assert!(out.html.is_none());
        assert!(out.text.as_deref().unwrap().contains("plain body"));
        assert!(!out.has_remote_content);
    }

    #[test]
    fn script_and_handlers_are_stripped() {
        let out = render(include_bytes!("../fixtures/render_script.eml"), false);
        let html = out.html.unwrap();
        assert!(!html.contains("<script"), "script tag removed");
        assert!(!html.to_ascii_lowercase().contains("alert("), "script text removed");
        assert!(!html.to_ascii_lowercase().contains("onclick"), "event handler removed");
        assert!(!html.to_ascii_lowercase().contains("javascript:"), "javascript: URL removed");
    }

    #[test]
    fn remote_image_blocked_by_default_and_flagged() {
        let out = render(include_bytes!("../fixtures/render_html_remote.eml"), false);
        let html = out.html.unwrap();
        assert!(!html.contains("http://track.example.com"), "remote src stripped");
        assert!(out.has_remote_content, "remote content flag set");
    }

    #[test]
    fn remote_image_kept_when_load_remote() {
        let out = render(include_bytes!("../fixtures/render_html_remote.eml"), true);
        let html = out.html.unwrap();
        assert!(html.contains("http://track.example.com"), "remote src retained on opt-in");
    }

    #[test]
    fn cid_image_rewritten_to_data_uri() {
        let out = render(include_bytes!("../fixtures/render_html_cid.eml"), false);
        let html = out.html.unwrap();
        assert!(html.contains("data:image/png;base64,"), "cid rewritten to data URI");
        assert!(!html.contains("cid:"), "no cid: reference remains");
        assert!(!out.has_remote_content, "inline cid images are not remote content");
    }

    #[test]
    fn unknown_cid_drops_src() {
        // Reference a cid with no matching inline part: the src is dropped
        // entirely rather than leaking a cid: or fabricating a data: URI.
        let raw = b"From: alice@example.com\n\
                    To: bob@example.com\n\
                    Subject: Missing inline\n\
                    Content-Type: text/html; charset=utf-8\n\
                    \n\
                    <html><body><img src=\"cid:missing@x\" alt=\"gone\"></body></html>\n";
        let out = render(raw, false);
        let html = out.html.unwrap();
        assert!(!html.contains("cid:"), "unresolved cid: must not leak");
        assert!(!html.contains("data:"), "no data: URI fabricated for unknown cid");
        assert!(!out.has_remote_content, "dropped inline cid is not remote content");
    }

    #[test]
    fn protocol_relative_src_is_blocked_and_flagged() {
        let raw = b"From: alice@example.com\n\
                    To: bob@example.com\n\
                    Subject: Protocol relative\n\
                    Content-Type: text/html; charset=utf-8\n\
                    \n\
                    <html><body><img src=\"//track.example.com/p.gif\"></body></html>\n";
        let blocked = render(raw, false);
        let html = blocked.html.unwrap();
        assert!(!html.contains("track.example.com"), "protocol-relative src stripped");
        assert!(blocked.has_remote_content, "protocol-relative remote flagged");

        let kept = render(raw, true);
        assert!(kept.html.unwrap().contains("track.example.com"), "protocol-relative src retained on opt-in");
    }

    #[test]
    fn whitespace_only_html_falls_back_to_text() {
        // multipart/alternative: the HTML part is entirely a <script>, which
        // ammonia strips to empty, so we must fall back to the text/plain part.
        let raw = b"From: alice@example.com\n\
                    To: bob@example.com\n\
                    Subject: Script-only html\n\
                    Content-Type: multipart/alternative; boundary=\"b1\"\n\
                    \n\
                    --b1\n\
                    Content-Type: text/plain; charset=utf-8\n\
                    \n\
                    fallback plain text\n\
                    --b1\n\
                    Content-Type: text/html; charset=utf-8\n\
                    \n\
                    <html><body><script>alert('x')</script></body></html>\n\
                    --b1--\n";
        let out = render(raw, false);
        assert!(out.html.is_none(), "empty sanitized html falls back to none");
        assert!(out.text.as_deref().unwrap().contains("fallback plain text"), "text/plain body surfaced");
        assert!(!out.has_remote_content);
    }
}

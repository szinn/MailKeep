use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
#[cfg(feature = "server")]
use {
    crate::routes::account_add_page::dtos::relative_time,
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    chrono::Utc,
    mk_core::CoreServices,
    std::sync::Arc,
};

use crate::routes::home_page::format::{format_bytes, thousands};

/// Global archive statistics for the logged-in user. `storage_bytes` is raw
/// (formatting happens in the UI); `last_synced` is a pre-formatted relative
/// string ("4m ago") or `None` when nothing has synced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ArchiveStatsDto {
    pub message_count: u64,
    pub attachment_count: u64,
    pub storage_bytes: u64,
    pub account_count: u64,
    pub last_synced: Option<String>,
}

#[get(
    "/api/v1/stats/archive",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn archive_stats() -> Result<ArchiveStatsDto, ServerFnError> {
    let user = authenticated_user(&auth_session)?;
    let stats = core_services.stats_service.archive_stats(user.id()).await.map_err(to_server_err)?;
    Ok(ArchiveStatsDto {
        message_count: stats.message_count,
        attachment_count: stats.attachment_count,
        storage_bytes: stats.storage_bytes,
        account_count: stats.account_count,
        last_synced: stats.last_synced_at.map(|t| relative_time(t, Utc::now())),
    })
}

/// The four labelled display values for the stat cards, in render order.
pub(crate) fn stat_cards(dto: &ArchiveStatsDto) -> [(&'static str, String); 4] {
    [
        ("Messages", thousands(dto.message_count)),
        ("Storage", format_bytes(dto.storage_bytes)),
        ("Accounts", thousands(dto.account_count)),
        ("Attachments", thousands(dto.attachment_count)),
    ]
}

#[component]
pub(crate) fn StatsPanel() -> Element {
    let stats = use_resource(move || async move { archive_stats().await });

    rsx! {
        div { class: "mx-auto max-w-3xl",
            h2 { class: "mb-6 text-lg font-semibold text-gray-700 dark:text-slate-200", "Archive overview" }
            match stats() {
                None => rsx! {
                    div { class: "text-sm text-gray-400 dark:text-slate-500", "Loading…" }
                },
                Some(Err(e)) => rsx! {
                    div { class: "text-sm text-red-600 dark:text-red-400", "Couldn't load statistics: {e}" }
                },
                Some(Ok(dto)) => {
                    let cards = stat_cards(&dto);
                    let last = dto
                        .last_synced
                        .clone()
                        .map_or_else(|| "never synced".to_string(), |t| format!("updated {t}"));
                    rsx! {
                        div { class: "grid grid-cols-2 gap-4 sm:grid-cols-4",
                            for (label , value) in cards {
                                div { class: "rounded-xl border border-gray-200 bg-white p-4 dark:border-slate-700 dark:bg-slate-800",
                                    div { class: "text-2xl font-semibold text-gray-900 dark:text-slate-100", "{value}" }
                                    div { class: "mt-1 text-xs uppercase tracking-wide text-gray-400 dark:text-slate-500", "{label}" }
                                }
                            }
                        }
                        p { class: "mt-4 text-xs text-gray-400 dark:text-slate-500", "{last}" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod card_tests {
    use super::{ArchiveStatsDto, stat_cards};

    #[test]
    fn stat_cards_values() {
        let dto = ArchiveStatsDto {
            message_count: 12_431,
            attachment_count: 480,
            storage_bytes: 4_509_715_660,
            account_count: 3,
            last_synced: Some("2m ago".into()),
        };
        let cards = stat_cards(&dto);
        assert_eq!(cards[0], ("Messages", "12,431".to_string()));
        assert_eq!(cards[1], ("Storage", "4.5 GB".to_string()));
        assert_eq!(cards[2], ("Accounts", "3".to_string()));
        assert_eq!(cards[3], ("Attachments", "480".to_string()));
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use chrono::{TimeZone, Utc};
    use mk_core::stats::ArchiveStats;

    use super::ArchiveStatsDto;
    use crate::routes::account_add_page::dtos::relative_time;

    // Pure mapping mirror of the server fn body (the fn itself needs a live
    // CoreServices; the mapping is what we assert).
    fn to_dto(s: &ArchiveStats, now: chrono::DateTime<Utc>) -> ArchiveStatsDto {
        ArchiveStatsDto {
            message_count: s.message_count,
            attachment_count: s.attachment_count,
            storage_bytes: s.storage_bytes,
            account_count: s.account_count,
            last_synced: s.last_synced_at.map(|t| relative_time(t, now)),
        }
    }

    #[test]
    fn stats_to_dto_maps_fields() {
        let now = Utc.timestamp_opt(1_700_000_120, 0).unwrap();
        let s = ArchiveStats {
            message_count: 12_431,
            attachment_count: 480,
            storage_bytes: 4_509_715_660,
            account_count: 3,
            last_synced_at: Some(Utc.timestamp_opt(1_700_000_000, 0).unwrap()),
        };
        let dto = to_dto(&s, now);
        assert_eq!(dto.message_count, 12_431);
        assert_eq!(dto.storage_bytes, 4_509_715_660);
        assert_eq!(dto.account_count, 3);
        assert_eq!(dto.last_synced.as_deref(), Some("2m ago"));

        let never = ArchiveStats { last_synced_at: None, ..s };
        assert_eq!(to_dto(&never, now).last_synced, None);
    }
}

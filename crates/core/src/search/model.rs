use crate::{account::AccountId, message::MessageId};

/// One ranked search result. The frontend joins on `message_id` to fetch full
/// message metadata; `snippet` is the index's stored snippet for quick display.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub message_id: MessageId,
    pub account_id: AccountId,
    pub score: f32,
    pub snippet: String,
}

/// A page of search results. `total` is the total number of matching messages
/// (for pagination); `hits` is the current page, bounded by the caller's
/// limit/offset.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResults {
    pub total: usize,
    pub hits: Vec<SearchHit>,
}

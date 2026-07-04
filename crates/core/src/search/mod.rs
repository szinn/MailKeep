pub mod model;
pub mod service;

pub use model::{SearchHit, SearchResults};
#[cfg(any(test, feature = "test-support"))]
pub use service::MockSearchService;
pub use service::SearchService;

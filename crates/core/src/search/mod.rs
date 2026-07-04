pub mod model;
pub mod query;
pub mod service;

pub use model::{SearchHit, SearchResults};
pub use query::{Clause, DateBound, Query, Term, TextField, parse};
#[cfg(any(test, feature = "test-support"))]
pub use service::MockSearchService;
pub use service::SearchService;

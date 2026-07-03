pub mod model;
pub mod repository;
pub mod service;

pub use model::ArchiveStats;
pub use repository::StatsRepository;
#[cfg(any(test, feature = "test-support"))]
pub use service::MockStatsService;
pub use service::StatsService;
pub(crate) use service::StatsServiceImpl;

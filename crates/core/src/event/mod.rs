mod model;
mod service;

pub use model::AppEvent;
#[cfg(any(test, feature = "test-support"))]
pub use service::MockEventService;
pub use service::{EventService, create_event_service};

pub mod model;
pub mod repository;
pub mod service;

pub use model::{
    Message, MessageAttachment, MessageAttachmentBuilder, MessageAttachmentId, MessageAttachmentToken, MessageAttachmentTokenPrefix, MessageBuilder,
    MessageFlags, MessageId, MessageLocation, MessageLocationBuilder, MessageLocationId, MessageLocationToken, MessageLocationTokenPrefix, MessageToken,
    MessageTokenPrefix, NamedAddress, ParsedAttachment, ParsedMessage, RecordedMessage,
};
pub use repository::{
    MessageAttachmentRepository, MessageLocationRepository, MessageRepository, NewMessageAttachmentRow, NewMessageLocationRow, NewMessageRow,
};
pub use service::MessageService;
pub(crate) use service::MessageServiceImpl;

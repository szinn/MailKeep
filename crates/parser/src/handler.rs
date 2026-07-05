use std::sync::Arc;

use mk_core::{
    Error,
    ingest::ParseMessageJob,
    jobs::{JobHandler, JobServiceExt},
    message::{MessageService, ParsedAttachment, ParsedMessage},
    storage::{AttachmentStorageService, RawStorageService},
};

use crate::parse::parse_eml;

pub struct ParseMessageHandler {
    raw_storage_service: Arc<dyn RawStorageService>,
    attachment_storage_service: Arc<dyn AttachmentStorageService>,
    message_service: Arc<dyn MessageService>,
}

impl ParseMessageHandler {
    #[must_use]
    pub fn new(
        raw_storage_service: Arc<dyn RawStorageService>,
        attachment_storage_service: Arc<dyn AttachmentStorageService>,
        message_service: Arc<dyn MessageService>,
    ) -> Self {
        Self {
            raw_storage_service,
            attachment_storage_service,
            message_service,
        }
    }
}

impl JobHandler for ParseMessageHandler {
    const JOB_TYPE: &'static str = "parse_message";
    const DISPLAY_NAME: &'static str = "Parse message";
    const QUIET: bool = true;
    type Payload = ParseMessageJob;

    async fn handle(&self, payload: ParseMessageJob) -> Result<(), Error> {
        let raw = self.raw_storage_service.get(payload.account_id, &payload.content_hash).await?;

        // Terminal on parse failure (Error::Validation is non-transient).
        let parsed = parse_eml(payload.content_hash, &raw).map_err(|e| {
            tracing::warn!(account_id = payload.account_id, content_hash = %payload.content_hash, error = %e, "failed to parse message; marking job terminal");
            e
        })?;

        let mut attachments = Vec::with_capacity(parsed.attachments.len());
        for att in parsed.attachments {
            let size_bytes = att.bytes.len() as i64;
            let hash = self.attachment_storage_service.put_if_absent(payload.account_id, &att.bytes).await?;
            attachments.push(ParsedAttachment {
                content_hash: hash,
                filename: att.filename,
                content_type: att.content_type,
                size_bytes,
                is_inline: att.is_inline,
                content_id: att.content_id,
            });
        }

        let message = ParsedMessage {
            rfc822_message_id: parsed.rfc822_message_id,
            content_hash: payload.content_hash,
            subject: parsed.subject,
            from_address: parsed.from_address,
            from_name: parsed.from_name,
            to_addresses: parsed.to_addresses,
            cc_addresses: parsed.cc_addresses,
            bcc_addresses: parsed.bcc_addresses,
            reply_to_addresses: parsed.reply_to_addresses,
            sent_date: parsed.sent_date,
            in_reply_to: parsed.in_reply_to,
            references: parsed.references,
            snippet: parsed.snippet,
            size_bytes: parsed.size_bytes,
            attachments,
        };

        self.message_service
            .record_parsed_message(
                payload.account_id,
                payload.folder_id,
                payload.uid,
                payload.uidvalidity,
                payload.internal_date,
                payload.flags,
                message,
            )
            .await?;

        Ok(())
    }
}

/// Register the parser's job handlers with the job service. Call after
/// `CoreServices` is built and before the job worker subsystem starts.
pub fn register_handlers(core: &Arc<mk_core::CoreServices>) {
    core.job_service.register(ParseMessageHandler::new(
        core.raw_storage_service.clone(),
        core.attachment_storage_service.clone(),
        core.message_service.clone(),
    ));
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use mk_core::{
        account::AccountId,
        folder::FolderId,
        message::{MessageFlags, RecordedMessage},
        storage::{MockAttachmentStorageService, MockRawStorageService},
        types::ContentHash,
    };

    use super::*;

    const RAW: &[u8] = b"Message-ID: <m1@example.com>\r\nFrom: Alice <alice@example.com>\r\nTo: bob@example.com\r\nSubject: Hi\r\nDate: Tue, 1 Nov 2022 10:00:00 +0000\r\n\r\nHello body\r\n";

    type Captured = Arc<Mutex<Option<(AccountId, FolderId, u32, u32, ParsedMessage)>>>;

    /// Hand-rolled fake MessageService that captures what was passed to
    /// `record_parsed_message` for later assertion.
    struct FakeMessageService {
        captured: Captured,
    }

    impl FakeMessageService {
        fn new() -> (Self, Captured) {
            let captured = Arc::new(Mutex::new(None));
            (Self { captured: captured.clone() }, captured)
        }
    }

    #[async_trait]
    impl MessageService for FakeMessageService {
        async fn record_parsed_message(
            &self,
            account_id: AccountId,
            folder_id: FolderId,
            uid: u32,
            uidvalidity: u32,
            _internal_date: DateTime<Utc>,
            _flags: MessageFlags,
            parsed: ParsedMessage,
        ) -> Result<RecordedMessage, Error> {
            *self.captured.lock().unwrap() = Some((account_id, folder_id, uid, uidvalidity, parsed));
            Ok(RecordedMessage { message_id: 1, created: true })
        }

        async fn delete_locations_for_folder(&self, _folder_id: FolderId) -> Result<u64, Error> {
            unimplemented!("not needed in this test")
        }

        async fn get_message_for_account(
            &self,
            _account_id: AccountId,
            _message_id: mk_core::message::MessageId,
        ) -> Result<Option<mk_core::message::Message>, Error> {
            unimplemented!("not needed in this test")
        }

        async fn list_messages_for_account(&self, _account_id: AccountId, _limit: u32, _offset: u32) -> Result<Vec<mk_core::message::Message>, Error> {
            unimplemented!("not needed in this test")
        }

        async fn get_messages_by_ids(
            &self,
            _user_id: mk_core::user::UserId,
            _ids: &[mk_core::message::MessageId],
        ) -> Result<Vec<mk_core::message::Message>, Error> {
            unimplemented!("not needed in this test")
        }
    }

    #[tokio::test]
    async fn handle_parses_and_records() {
        let content_hash = ContentHash::compute(RAW);

        let mut raw = MockRawStorageService::new();
        raw.expect_get().times(1).returning(move |_, _| Box::pin(async move { Ok(RAW.to_vec()) }));

        let attach = MockAttachmentStorageService::new(); // no attachments in RAW

        let (fake_msg, captured) = FakeMessageService::new();

        let handler = ParseMessageHandler::new(Arc::new(raw), Arc::new(attach), Arc::new(fake_msg));
        handler
            .handle(ParseMessageJob {
                account_id: 5,
                folder_id: 9,
                uid: 42,
                uidvalidity: 1000,
                content_hash,
                internal_date: Utc::now(),
                flags: MessageFlags::default(),
            })
            .await
            .unwrap();

        let locked = captured.lock().unwrap();
        let (account_id, folder_id, uid, uidvalidity, parsed) = locked.as_ref().expect("record_parsed_message must have been called");
        assert_eq!(*account_id, 5);
        assert_eq!(*folder_id, 9);
        assert_eq!(*uid, 42);
        assert_eq!(*uidvalidity, 1000);
        assert_eq!(parsed.rfc822_message_id, "<m1@example.com>");
        assert_eq!(parsed.from_address.as_str(), "alice@example.com");
        assert!(parsed.attachments.is_empty());
    }
}

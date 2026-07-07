use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::{
    Error,
    account::AccountId,
    error::RepositoryError,
    folder::FolderId,
    message::{
        Message, MessageAttachment, MessageAttachmentToken, MessageFlags, MessageId, MessageLocationToken, MessageToken, ParsedMessage, RecordedMessage,
        repository::{NewMessageAttachmentRow, NewMessageLocationRow, NewMessageRow},
    },
    repository::RepositoryService,
    user::UserId,
    with_read_only_transaction, with_transaction,
};

#[async_trait::async_trait]
pub trait MessageService: Send + Sync {
    async fn record_parsed_message(
        &self,
        account_id: AccountId,
        folder_id: FolderId,
        uid: u32,
        uidvalidity: u32,
        internal_date: DateTime<Utc>,
        flags: MessageFlags,
        parsed: ParsedMessage,
    ) -> Result<RecordedMessage, Error>;

    async fn delete_locations_for_folder(&self, folder_id: FolderId) -> Result<u64, Error>;

    async fn get_message_for_account(&self, account_id: AccountId, message_id: MessageId) -> Result<Option<Message>, Error>;

    async fn list_messages_for_account(&self, account_id: AccountId, limit: u32, offset: u32) -> Result<Vec<Message>, Error>;

    async fn get_messages_by_ids(&self, user_id: UserId, ids: &[MessageId]) -> Result<Vec<Message>, Error>;

    /// Fetch a message by token together with its attachment rows, scoped to
    /// the requesting user (the message's account must belong to `user_id`).
    /// Returns `None` when the token is unknown or not owned by the user.
    async fn get_message_with_attachments(&self, user_id: UserId, token: MessageToken) -> Result<Option<(Message, Vec<MessageAttachment>)>, Error>;
}

pub(crate) struct MessageServiceImpl {
    repository_service: Arc<RepositoryService>,
}

impl MessageServiceImpl {
    #[must_use]
    pub(crate) fn new(repository_service: Arc<RepositoryService>) -> Self {
        Self { repository_service }
    }
}

#[async_trait::async_trait]
impl MessageService for MessageServiceImpl {
    async fn record_parsed_message(
        &self,
        account_id: AccountId,
        folder_id: FolderId,
        uid: u32,
        uidvalidity: u32,
        internal_date: DateTime<Utc>,
        flags: MessageFlags,
        parsed: ParsedMessage,
    ) -> Result<RecordedMessage, Error> {
        with_transaction!(self, message_repository, message_location_repository, message_attachment_repository, |tx| {
            // Identity is the raw-content hash: identical bytes are the same
            // archived message (dedup), even if the Message-ID differs; distinct
            // bytes are distinct messages even when the Message-ID matches.
            let existing = message_repository.find_by_account_and_content_hash(tx, account_id, parsed.content_hash).await?;

            let (message_id, created) = if let Some(existing) = existing {
                (existing.id, false)
            } else {
                let new_message_row = NewMessageRow {
                    token: MessageToken::generate(),
                    account_id,
                    rfc822_message_id: parsed.rfc822_message_id.clone(),
                    content_hash: parsed.content_hash,
                    subject: parsed.subject.clone(),
                    from_address: parsed.from_address.clone(),
                    from_name: parsed.from_name.clone(),
                    to_addresses: parsed.to_addresses.clone(),
                    cc_addresses: parsed.cc_addresses.clone(),
                    bcc_addresses: parsed.bcc_addresses.clone(),
                    reply_to_addresses: parsed.reply_to_addresses.clone(),
                    sent_date: parsed.sent_date,
                    in_reply_to: parsed.in_reply_to.clone(),
                    references: parsed.references.clone(),
                    snippet: parsed.snippet.clone(),
                    size_bytes: parsed.size_bytes,
                    has_attachments: !parsed.attachments.is_empty(),
                    attachment_count: i32::try_from(parsed.attachments.len()).unwrap_or(i32::MAX),
                };
                match message_repository.create(tx, new_message_row).await {
                    Ok(message) => {
                        let attachment_rows: Vec<NewMessageAttachmentRow> = parsed
                            .attachments
                            .iter()
                            .map(|a| NewMessageAttachmentRow {
                                token: MessageAttachmentToken::generate(),
                                message_id: message.id,
                                account_id,
                                content_hash: a.content_hash,
                                filename: a.filename.clone(),
                                content_type: a.content_type.clone(),
                                size_bytes: a.size_bytes,
                                is_inline: a.is_inline,
                                content_id: a.content_id.clone(),
                            })
                            .collect();
                        message_attachment_repository.create_many(tx, attachment_rows).await?;

                        (message.id, true)
                    }
                    // Concurrent insert race on the (account_id, content_hash) unique
                    // index: another worker archived this exact content (and its
                    // attachments) first. Dedup to the winner rather than failing.
                    Err(Error::RepositoryError(RepositoryError::Constraint(_))) => {
                        let winner = message_repository
                            .find_by_account_and_content_hash(tx, account_id, parsed.content_hash)
                            .await?
                            .ok_or(Error::RepositoryError(RepositoryError::Conflict))?;
                        (winner.id, false)
                    }
                    Err(e) => return Err(e),
                }
            };

            let location_row = NewMessageLocationRow {
                token: MessageLocationToken::generate(),
                message_id,
                folder_id,
                uid,
                uidvalidity,
                flags,
                internal_date,
            };
            message_location_repository.upsert(tx, location_row).await?;

            Ok(RecordedMessage { message_id, created })
        })
    }

    async fn delete_locations_for_folder(&self, folder_id: FolderId) -> Result<u64, Error> {
        with_transaction!(self, message_location_repository, |tx| message_location_repository
            .delete_by_folder_id(tx, folder_id)
            .await)
    }

    async fn get_message_for_account(&self, account_id: AccountId, message_id: MessageId) -> Result<Option<Message>, Error> {
        with_read_only_transaction!(self, message_repository, |tx| message_repository
            .find_by_id_for_account(tx, account_id, message_id)
            .await)
    }

    async fn list_messages_for_account(&self, account_id: AccountId, limit: u32, offset: u32) -> Result<Vec<Message>, Error> {
        with_read_only_transaction!(self, message_repository, |tx| message_repository
            .list_for_account(tx, account_id, limit, offset)
            .await)
    }

    async fn get_messages_by_ids(&self, user_id: UserId, ids: &[MessageId]) -> Result<Vec<Message>, Error> {
        // Own the ids: the macro moves the body into a `'static` future, so a
        // borrowed slice can't be captured.
        let ids = ids.to_vec();
        with_read_only_transaction!(self, message_repository, |tx| message_repository.list_by_ids_for_user(tx, user_id, &ids).await)
    }

    async fn get_message_with_attachments(&self, user_id: UserId, token: MessageToken) -> Result<Option<(Message, Vec<MessageAttachment>)>, Error> {
        let message_id = token.id();
        with_read_only_transaction!(self, message_repository, message_attachment_repository, |tx| {
            let Some(message) = message_repository.find_by_id_for_user(tx, user_id, message_id).await? else {
                return Ok(None);
            };
            let attachments = message_attachment_repository.list_for_message(tx, message.id).await?;
            Ok(Some((message, attachments)))
        })
    }
}

#[cfg(test)]
mod tests {
    use mockall::predicate::*;

    use super::*;
    use crate::{
        message::{
            MessageLocation, ParsedAttachment,
            repository::{MockMessageAttachmentRepository, MockMessageLocationRepository, MockMessageRepository},
        },
        repository::testing::default_repository_service_builder,
        types::{ContentHash, EmailAddress},
    };

    fn setup_message_service(
        message_repo: MockMessageRepository,
        location_repo: MockMessageLocationRepository,
        attachment_repo: MockMessageAttachmentRepository,
    ) -> MessageServiceImpl {
        let rs = default_repository_service_builder()
            .message_repository(Arc::new(message_repo))
            .message_location_repository(Arc::new(location_repo))
            .message_attachment_repository(Arc::new(attachment_repo))
            .build()
            .expect("all fields provided");
        MessageServiceImpl::new(Arc::new(rs))
    }

    fn sample_content_hash() -> ContentHash {
        ContentHash::from_hex("a".repeat(64)).unwrap()
    }

    fn attachment_content_hash() -> ContentHash {
        ContentHash::from_hex("c".repeat(64)).unwrap()
    }

    fn sample_new_row() -> NewMessageRow {
        NewMessageRow {
            token: MessageToken::generate(),
            account_id: 1,
            rfc822_message_id: "<abc@example.com>".into(),
            content_hash: sample_content_hash(),
            subject: Some("Hello".into()),
            from_address: EmailAddress::new("alice@example.com").unwrap(),
            from_name: Some("Alice".into()),
            to_addresses: vec![],
            cc_addresses: vec![],
            bcc_addresses: vec![],
            reply_to_addresses: vec![],
            sent_date: Some(Utc::now()),
            in_reply_to: None,
            references: vec![],
            snippet: "Hello there".into(),
            size_bytes: 1024,
            has_attachments: false,
            attachment_count: 0,
        }
    }

    fn sample_parsed_message() -> ParsedMessage {
        ParsedMessage {
            rfc822_message_id: "<abc@example.com>".into(),
            content_hash: sample_content_hash(),
            subject: Some("Hello".into()),
            from_address: EmailAddress::new("alice@example.com").unwrap(),
            from_name: Some("Alice".into()),
            to_addresses: vec![],
            cc_addresses: vec![],
            bcc_addresses: vec![],
            reply_to_addresses: vec![],
            sent_date: Some(Utc::now()),
            in_reply_to: None,
            references: vec![],
            snippet: "Hello there".into(),
            size_bytes: 1024,
            attachments: vec![],
        }
    }

    fn sample_parsed_message_with_one_attachment() -> ParsedMessage {
        let mut m = sample_parsed_message();
        m.attachments.push(ParsedAttachment {
            content_hash: attachment_content_hash(),
            filename: Some("file.pdf".into()),
            content_type: "application/pdf".into(),
            size_bytes: 2048,
            is_inline: false,
            content_id: None,
        });
        m
    }

    fn make_message(id: MessageId, row: NewMessageRow) -> Message {
        Message {
            id,
            version: 0,
            token: row.token,
            account_id: row.account_id,
            rfc822_message_id: row.rfc822_message_id,
            content_hash: row.content_hash,
            subject: row.subject,
            from_address: row.from_address,
            from_name: row.from_name,
            to_addresses: row.to_addresses,
            cc_addresses: row.cc_addresses,
            bcc_addresses: row.bcc_addresses,
            reply_to_addresses: row.reply_to_addresses,
            sent_date: row.sent_date,
            in_reply_to: row.in_reply_to,
            references: row.references,
            snippet: row.snippet,
            size_bytes: row.size_bytes,
            has_attachments: row.has_attachments,
            attachment_count: row.attachment_count,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_existing_message(id: MessageId, account_id: AccountId, rfc822_message_id: String, content_hash: ContentHash) -> Message {
        Message {
            id,
            version: 0,
            token: MessageToken::generate(),
            account_id,
            rfc822_message_id,
            content_hash,
            subject: None,
            from_address: EmailAddress::new("alice@example.com").unwrap(),
            from_name: None,
            to_addresses: vec![],
            cc_addresses: vec![],
            bcc_addresses: vec![],
            reply_to_addresses: vec![],
            sent_date: None,
            in_reply_to: None,
            references: vec![],
            snippet: String::new(),
            size_bytes: 0,
            has_attachments: false,
            attachment_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_attachment(row: NewMessageAttachmentRow) -> MessageAttachment {
        MessageAttachment {
            id: 1,
            version: 0,
            token: row.token,
            message_id: row.message_id,
            account_id: row.account_id,
            content_hash: row.content_hash,
            filename: row.filename,
            content_type: row.content_type,
            size_bytes: row.size_bytes,
            is_inline: row.is_inline,
            content_id: row.content_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_location(row: NewMessageLocationRow) -> MessageLocation {
        MessageLocation {
            id: 1,
            version: 0,
            token: row.token,
            message_id: row.message_id,
            folder_id: row.folder_id,
            uid: row.uid,
            uidvalidity: row.uidvalidity,
            flags: row.flags,
            internal_date: row.internal_date,
            first_seen_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn record_parsed_message_fresh_insert() {
        let mut message_repo = MockMessageRepository::new();
        let mut location_repo = MockMessageLocationRepository::new();
        let mut attachment_repo = MockMessageAttachmentRepository::new();

        message_repo
            .expect_find_by_account_and_content_hash()
            .times(1)
            .returning(|_, _, _| Box::pin(async { Ok(None) }));
        message_repo
            .expect_create()
            .withf(|_, row: &NewMessageRow| {
                row.account_id == 1
                    && row.rfc822_message_id == "<abc@example.com>"
                    && row.content_hash == sample_content_hash()
                    && row.has_attachments
                    && row.attachment_count == 1
            })
            .times(1)
            .returning(|_, row| Box::pin(async move { Ok(make_message(42, row)) }));
        attachment_repo
            .expect_create_many()
            .withf(|_, rows: &Vec<NewMessageAttachmentRow>| rows.len() == 1 && rows[0].message_id == 42 && rows[0].filename.as_deref() == Some("file.pdf"))
            .times(1)
            .returning(|_, rows| Box::pin(async move { Ok(rows.into_iter().map(make_attachment).collect()) }));
        location_repo
            .expect_upsert()
            .withf(|_, row: &NewMessageLocationRow| row.message_id == 42 && row.folder_id == 2 && row.uid == 100 && row.uidvalidity == 1000)
            .times(1)
            .returning(|_, row| Box::pin(async move { Ok(make_location(row)) }));

        let svc = setup_message_service(message_repo, location_repo, attachment_repo);
        let result = svc
            .record_parsed_message(
                1,
                2,
                100,
                1000,
                Utc::now(),
                MessageFlags::default(),
                sample_parsed_message_with_one_attachment(),
            )
            .await
            .unwrap();

        assert!(result.created);
        assert_eq!(result.message_id, 42);
    }

    #[tokio::test]
    async fn record_parsed_message_idempotent_same_hash() {
        let mut message_repo = MockMessageRepository::new();
        let mut location_repo = MockMessageLocationRepository::new();
        let attachment_repo = MockMessageAttachmentRepository::new();

        let hash = sample_content_hash();
        message_repo
            .expect_find_by_account_and_content_hash()
            .times(1)
            .returning(move |_, _, _| Box::pin(async move { Ok(Some(make_existing_message(99, 1, "<abc@example.com>".into(), hash))) }));
        // No create, no create_many: assert by leaving no
        // expect_create/expect_create_many.
        location_repo
            .expect_upsert()
            .withf(|_, row: &NewMessageLocationRow| row.message_id == 99)
            .times(1)
            .returning(|_, row| Box::pin(async move { Ok(make_location(row)) }));

        let svc = setup_message_service(message_repo, location_repo, attachment_repo);
        let result = svc
            .record_parsed_message(1, 2, 100, 1000, Utc::now(), MessageFlags::default(), sample_parsed_message())
            .await
            .unwrap();

        assert!(!result.created);
        assert_eq!(result.message_id, 99);
    }

    #[tokio::test]
    async fn record_parsed_message_same_msgid_different_content_archives_both() {
        // A second email shares an existing Message-ID but has different raw bytes.
        // Identity is the content hash, so the lookup misses and the message is
        // archived as a distinct row rather than rejected as a conflict.
        let mut message_repo = MockMessageRepository::new();
        let mut location_repo = MockMessageLocationRepository::new();
        let mut attachment_repo = MockMessageAttachmentRepository::new();

        message_repo
            .expect_find_by_account_and_content_hash()
            .times(1)
            .returning(|_, _, _| Box::pin(async { Ok(None) }));
        message_repo
            .expect_create()
            .times(1)
            .returning(|_, row| Box::pin(async move { Ok(make_message(88, row)) }));
        attachment_repo
            .expect_create_many()
            .times(1)
            .returning(|_, rows| Box::pin(async move { Ok(rows.into_iter().map(make_attachment).collect()) }));
        location_repo
            .expect_upsert()
            .times(1)
            .returning(|_, row| Box::pin(async move { Ok(make_location(row)) }));

        let svc = setup_message_service(message_repo, location_repo, attachment_repo);
        let result = svc
            .record_parsed_message(1, 2, 100, 1000, Utc::now(), MessageFlags::default(), sample_parsed_message())
            .await
            .unwrap();

        assert!(result.created);
        assert_eq!(result.message_id, 88);
    }

    #[tokio::test]
    async fn record_parsed_message_two_folders_share_message() {
        // First call: fresh insert into folder A. Second call: same parsed message
        // but folder B — finds existing, upserts a new location, no create/create_many.
        let mut message_repo = MockMessageRepository::new();
        let mut location_repo = MockMessageLocationRepository::new();
        let mut attachment_repo = MockMessageAttachmentRepository::new();

        // First call returns None (no existing), second call returns Some(existing with
        // id=55).
        let hash = sample_content_hash();
        let mut call_count = 0;
        message_repo.expect_find_by_account_and_content_hash().times(2).returning(move |_, _, _| {
            call_count += 1;
            let n = call_count;
            Box::pin(async move {
                if n == 1 {
                    Ok(None)
                } else {
                    Ok(Some(make_existing_message(55, 1, "<abc@example.com>".into(), hash)))
                }
            })
        });
        // Create called exactly once (only first call, fresh path).
        message_repo
            .expect_create()
            .times(1)
            .returning(|_, row| Box::pin(async move { Ok(make_message(55, row)) }));
        // create_many called exactly once with empty rows (no attachments in sample).
        attachment_repo
            .expect_create_many()
            .withf(|_, rows: &Vec<NewMessageAttachmentRow>| rows.is_empty())
            .times(1)
            .returning(|_, rows| Box::pin(async move { Ok(rows.into_iter().map(make_attachment).collect()) }));
        // Upsert called twice — once per call — with different folder_ids.
        location_repo
            .expect_upsert()
            .times(2)
            .returning(|_, row| Box::pin(async move { Ok(make_location(row)) }));

        let svc = setup_message_service(message_repo, location_repo, attachment_repo);

        let r1 = svc
            .record_parsed_message(1, 10, 100, 1000, Utc::now(), MessageFlags::default(), sample_parsed_message())
            .await
            .unwrap();
        assert!(r1.created);
        assert_eq!(r1.message_id, 55);

        let r2 = svc
            .record_parsed_message(1, 20, 200, 1000, Utc::now(), MessageFlags::default(), sample_parsed_message())
            .await
            .unwrap();
        assert!(!r2.created);
        assert_eq!(r2.message_id, 55);
    }

    #[tokio::test]
    async fn record_parsed_message_concurrent_insert_dedups_to_winner() {
        // Two workers insert the same content concurrently. The loser hits the
        // (account_id, content_hash) unique constraint; the service re-fetches the
        // winner and dedups to it (created=false), recording only the location —
        // it does not fail or re-create attachments.
        let mut message_repo = MockMessageRepository::new();
        let mut location_repo = MockMessageLocationRepository::new();
        let attachment_repo = MockMessageAttachmentRepository::new();

        let hash = sample_content_hash();
        let mut find_calls = 0;
        message_repo.expect_find_by_account_and_content_hash().times(2).returning(move |_, _, _| {
            find_calls += 1;
            let n = find_calls;
            Box::pin(async move {
                if n == 1 {
                    Ok(None)
                } else {
                    Ok(Some(make_existing_message(66, 1, "<abc@example.com>".into(), hash)))
                }
            })
        });
        message_repo.expect_create().times(1).returning(|_, _| {
            Box::pin(async {
                Err(Error::RepositoryError(RepositoryError::Constraint(
                    "unique violation on (account_id, content_hash)".into(),
                )))
            })
        });
        // create_many must NOT be called on the dedup path.
        location_repo
            .expect_upsert()
            .withf(|_, row: &NewMessageLocationRow| row.message_id == 66)
            .times(1)
            .returning(|_, row| Box::pin(async move { Ok(make_location(row)) }));

        let svc = setup_message_service(message_repo, location_repo, attachment_repo);
        let result = svc
            .record_parsed_message(1, 2, 100, 1000, Utc::now(), MessageFlags::default(), sample_parsed_message())
            .await
            .unwrap();

        assert!(!result.created);
        assert_eq!(result.message_id, 66);
    }

    #[tokio::test]
    async fn delete_locations_for_folder_delegates() {
        let mut location_repo = MockMessageLocationRepository::new();
        location_repo
            .expect_delete_by_folder_id()
            .withf(|_, folder_id| *folder_id == 42)
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(7u64) }));

        let svc = setup_message_service(MockMessageRepository::new(), location_repo, MockMessageAttachmentRepository::new());
        let count = svc.delete_locations_for_folder(42).await.unwrap();
        assert_eq!(count, 7);
    }

    #[tokio::test]
    async fn get_message_for_account_delegates() {
        let mut message_repo = MockMessageRepository::new();
        message_repo
            .expect_find_by_id_for_account()
            .with(always(), eq(1u64), eq(42u64))
            .times(1)
            .returning(|_, _, _| Box::pin(async { Ok(None) }));

        let svc = setup_message_service(message_repo, MockMessageLocationRepository::new(), MockMessageAttachmentRepository::new());
        let result = svc.get_message_for_account(1, 42).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_messages_for_account_delegates() {
        let mut message_repo = MockMessageRepository::new();
        message_repo
            .expect_list_for_account()
            .withf(|_, account_id, limit, offset| *account_id == 1 && *limit == 50 && *offset == 10)
            .times(1)
            .returning(|_, _, _, _| Box::pin(async { Ok(vec![]) }));

        let svc = setup_message_service(message_repo, MockMessageLocationRepository::new(), MockMessageAttachmentRepository::new());
        let messages = svc.list_messages_for_account(1, 50, 10).await.unwrap();
        assert!(messages.is_empty());
    }

    #[tokio::test]
    async fn get_messages_by_ids_delegates() {
        let mut message_repo = MockMessageRepository::new();
        message_repo
            .expect_list_by_ids_for_user()
            .withf(|_, user_id, ids| *user_id == 7 && ids == [10u64, 20u64])
            .times(1)
            .returning(|_, _, _| Box::pin(async { Ok(vec![]) }));

        let svc = setup_message_service(message_repo, MockMessageLocationRepository::new(), MockMessageAttachmentRepository::new());
        let out = svc.get_messages_by_ids(7, &[10, 20]).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn get_message_with_attachments_returns_message_and_attachments() {
        let mut message_repo = MockMessageRepository::new();
        let mut attachment_repo = MockMessageAttachmentRepository::new();

        let row = NewMessageRow {
            token: MessageToken::new(42),
            ..sample_new_row()
        };
        message_repo
            .expect_find_by_id_for_user()
            .withf(|_, user_id, message_id| *user_id == 7 && *message_id == 42)
            .times(1)
            .returning(move |_, _, _| {
                let row = row.clone();
                Box::pin(async move { Ok(Some(make_message(42, row))) })
            });
        attachment_repo
            .expect_list_for_message()
            .withf(|_, message_id| *message_id == 42)
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(vec![]) }));

        let svc = setup_message_service(message_repo, MockMessageLocationRepository::new(), attachment_repo);
        let out = svc.get_message_with_attachments(7, MessageToken::new(42)).await.unwrap();
        let (message, attachments) = out.expect("owner sees the message");
        assert_eq!(message.id, 42);
        assert!(attachments.is_empty());
    }

    #[tokio::test]
    async fn get_message_with_attachments_none_for_foreign_token() {
        let mut message_repo = MockMessageRepository::new();
        message_repo
            .expect_find_by_id_for_user()
            .times(1)
            .returning(|_, _, _| Box::pin(async { Ok(None) }));
        // list_for_message must NOT be called when the message is not found.
        let svc = setup_message_service(message_repo, MockMessageLocationRepository::new(), MockMessageAttachmentRepository::new());
        let out = svc.get_message_with_attachments(7, MessageToken::new(42)).await.unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn snapshot_recorded_message_shape() {
        let value = RecordedMessage { message_id: 42, created: true };
        insta::assert_yaml_snapshot!(value);
    }
}

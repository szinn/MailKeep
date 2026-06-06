use chrono::Utc;
use mk_core::message::MessageToken;
use sea_orm::{ActiveValue::Set, entity::prelude::*};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "messages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i64,
    pub version: i64,
    #[sea_orm(unique)]
    pub token: String,
    pub account_id: i64,
    pub rfc822_message_id: String,
    pub content_hash: String,
    pub subject: Option<String>,
    pub from_address: String,
    pub from_name: Option<String>,
    pub to_addresses: Json,
    pub cc_addresses: Json,
    pub bcc_addresses: Json,
    pub reply_to_addresses: Json,
    pub sent_date: Option<DateTimeWithTimeZone>,
    pub in_reply_to: Option<String>,
    pub references: Json,
    pub snippet: String,
    pub size_bytes: i64,
    pub has_attachments: bool,
    pub attachment_count: i32,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    fn new() -> Self {
        let token = MessageToken::generate();
        Self {
            id: Set(token.id() as i64),
            token: Set(token.to_string()),
            created_at: Set(Utc::now().into()),
            updated_at: Set(Utc::now().into()),
            ..ActiveModelTrait::default()
        }
    }

    async fn before_save<C>(mut self, _db: &C, _insert: bool) -> Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if self.is_changed() {
            self.version = Set(self.version.unwrap() + 1);
            self.updated_at = Set(Utc::now().into());
        }
        Ok(self)
    }
}

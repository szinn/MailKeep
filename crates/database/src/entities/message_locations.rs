use chrono::Utc;
use mk_core::message::MessageLocationToken;
use sea_orm::{ActiveValue::Set, entity::prelude::*};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "message_locations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i64,
    pub version: i64,
    #[sea_orm(unique)]
    pub token: String,
    pub message_id: i64,
    pub folder_id: i64,
    pub uid: i64,
    pub uidvalidity: i64,
    pub flags: Json,
    pub internal_date: DateTimeWithTimeZone,
    pub first_seen_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    fn new() -> Self {
        let token = MessageLocationToken::generate();
        let now = Utc::now();
        Self {
            id: Set(token.id() as i64),
            token: Set(token.to_string()),
            first_seen_at: Set(now.into()),
            updated_at: Set(now.into()),
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

use chrono::Utc;
use mk_core::account::AccountToken;
use sea_orm::{ActiveValue::Set, entity::prelude::*};
use serde::{Deserialize, Serialize};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "accounts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i64,
    pub version: i64,
    #[sea_orm(unique)]
    pub token: String,
    pub user_id: i64,
    pub display_name: String,
    pub email_address: String,
    pub server: String,
    pub username: String,
    pub credentials: Vec<u8>,
    pub enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub last_synced_at: Option<DateTimeWithTimeZone>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    fn new() -> Self {
        let token = AccountToken::generate();
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

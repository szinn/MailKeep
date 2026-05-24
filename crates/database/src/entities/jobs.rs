use chrono::Utc;
use mk_core::jobs::{Job, JobStatus};
use sea_orm::{ActiveValue::Set, entity::prelude::*};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "jobs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    pub job_type: String,
    pub payload: Json,
    pub status: String,
    pub priority: i16,
    pub attempt: i16,
    pub max_attempts: i16,
    pub version: i32,
    pub scheduled_at: DateTimeWithTimeZone,
    pub started_at: Option<DateTimeWithTimeZone>,
    pub completed_at: Option<DateTimeWithTimeZone>,
    pub error_message: Option<String>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    fn new() -> Self {
        let now = Utc::now();
        Self {
            status: Set(job_status_to_str(&JobStatus::Pending).to_string()),
            priority: Set(0),
            attempt: Set(0),
            max_attempts: Set(3),
            version: Set(0),
            scheduled_at: Set(now.into()),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            ..ActiveModelTrait::default()
        }
    }
}

pub(crate) fn job_status_to_str(status: &JobStatus) -> &'static str {
    match status {
        JobStatus::Pending => "Pending",
        JobStatus::Running => "Running",
        JobStatus::Completed => "Completed",
        JobStatus::Failed => "Failed",
    }
}

pub(crate) fn str_to_job_status(s: &str) -> JobStatus {
    match s {
        "Pending" => JobStatus::Pending,
        "Running" => JobStatus::Running,
        "Completed" => JobStatus::Completed,
        "Failed" => JobStatus::Failed,
        other => panic!("unknown job status '{other}' in database — schema invariant violated"),
    }
}

impl From<Model> for Job {
    fn from(m: Model) -> Self {
        Self {
            id: m.id,
            job_type: m.job_type,
            payload: m.payload,
            status: str_to_job_status(&m.status),
            priority: m.priority,
            attempt: m.attempt,
            max_attempts: m.max_attempts,
            version: m.version,
            scheduled_at: m.scheduled_at.with_timezone(&Utc),
            started_at: m.started_at.map(|t| t.with_timezone(&Utc)),
            completed_at: m.completed_at.map(|t| t.with_timezone(&Utc)),
            error_message: m.error_message,
            created_at: m.created_at.with_timezone(&Utc),
            updated_at: m.updated_at.with_timezone(&Utc),
        }
    }
}

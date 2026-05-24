use std::pin::Pin;

use serde::de::DeserializeOwned;

use crate::Error;

/// Implement this trait on your handler struct. `JOB_TYPE` must match the
/// corresponding `Enqueueable::JOB_TYPE` on the payload.
pub trait JobHandler: Send + Sync + 'static {
    const JOB_TYPE: &'static str;
    const DISPLAY_NAME: &'static str;
    type Payload: DeserializeOwned + Send;

    fn handle(&self, payload: Self::Payload) -> impl Future<Output = Result<(), Error>> + Send;
}

/// Object-safe erased version of `JobHandler` used for dynamic dispatch.
///
/// Handler authors implement [`JobHandler`] — this trait exists for type-erased
/// storage in the job registry. The blanket impl below bridges the two.
pub trait ErasedJobHandler: Send + Sync {
    /// The job type string this handler is registered for.
    fn job_type(&self) -> &str;
    /// Human-readable display name for this handler.
    fn display_name(&self) -> &str;
    fn handle<'a>(&'a self, payload: serde_json::Value) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;
}

impl<H: JobHandler> ErasedJobHandler for H {
    fn job_type(&self) -> &str {
        H::JOB_TYPE
    }

    fn display_name(&self) -> &str {
        H::DISPLAY_NAME
    }

    fn handle<'a>(&'a self, payload: serde_json::Value) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let typed: H::Payload = serde_json::from_value(payload).map_err(|e| Error::Infrastructure(format!("job payload deserialize failed: {e}")))?;
            JobHandler::handle(self, typed).await
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    struct TestPayload {
        value: i32,
    }

    struct TestHandler {
        observed: Arc<Mutex<Vec<i32>>>,
    }

    impl JobHandler for TestHandler {
        const JOB_TYPE: &'static str = "test.handler";
        const DISPLAY_NAME: &'static str = "Test Handler";
        type Payload = TestPayload;

        async fn handle(&self, payload: TestPayload) -> Result<(), Error> {
            self.observed.lock().unwrap().push(payload.value);
            Ok(())
        }
    }

    #[tokio::test]
    async fn erased_handler_deserialises_and_forwards() {
        let observed = Arc::new(Mutex::new(vec![]));
        let handler = TestHandler { observed: observed.clone() };
        let erased: &dyn ErasedJobHandler = &handler;

        erased.handle(serde_json::json!({ "value": 7 })).await.unwrap();

        assert_eq!(*observed.lock().unwrap(), vec![7]);
        assert_eq!(erased.job_type(), "test.handler");
        assert_eq!(erased.display_name(), "Test Handler");
    }

    #[tokio::test]
    async fn erased_handler_deserialise_failure_surfaces_as_infrastructure() {
        let observed = Arc::new(Mutex::new(vec![]));
        let handler = TestHandler { observed };
        let erased: &dyn ErasedJobHandler = &handler;

        let result = erased.handle(serde_json::json!({})).await;

        assert!(matches!(result, Err(Error::Infrastructure(_))));
    }
}

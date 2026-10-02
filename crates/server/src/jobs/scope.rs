//! Optional local-runner capability. Ordinary HTTP/serve has no extra scope.
use manual_core::domain::JobStage;
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

pub type CallGateFuture<'a> = Pin<Box<dyn Future<Output = Result<(), &'static str>> + Send + 'a>>;
pub trait CallGate: Send + Sync {
    /// Re-read current authorization and bound local facts before a new call.
    fn check(&self) -> CallGateFuture<'_>;
}

pub struct ExecutionScope {
    pub job_id: String,
    pub gate: Arc<dyn CallGate>,
    /// Exact stage kind, batch index and input hash; absent = no additional
    /// paid submission retry. Safe remote queries retain the existing policy.
    pub safe_retries: BTreeMap<(String, i64, String), u32>,
}
impl ExecutionScope {
    pub fn safe_retry_limit(&self, stage: &JobStage) -> u32 {
        self.safe_retries
            .get(&(
                stage.stage_kind.as_str().to_owned(),
                stage.batch_index,
                stage.input_hash.clone(),
            ))
            .copied()
            .unwrap_or(0)
    }
}

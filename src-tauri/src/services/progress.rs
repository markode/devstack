//! Shared job progress events (WordPress install, templates, backup/restore).

use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job_id: String,
    pub phase: String,
    pub message: String,
    pub bytes: u64,
    pub total: Option<u64>,
    pub percent: Option<f32>,
}

pub type JobProgressFn = Arc<dyn Fn(JobProgress) + Send + Sync>;

pub fn noop_job_progress() -> JobProgressFn {
    Arc::new(|_| {})
}

pub fn emit(
    progress: &JobProgressFn,
    job_id: &str,
    phase: &str,
    message: &str,
    bytes: u64,
    total: Option<u64>,
) {
    let percent = total.map(|t| {
        if t == 0 {
            0.0
        } else {
            ((bytes as f64 / t as f64) * 100.0).min(100.0) as f32
        }
    });
    progress(JobProgress {
        job_id: job_id.into(),
        phase: phase.into(),
        message: message.into(),
        bytes,
        total,
        percent,
    });
}

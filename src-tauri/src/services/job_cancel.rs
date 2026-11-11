//! Cooperative cancel flags for long-running jobs (site starters, etc.).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::{DevStackError, Result};

fn flags() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static FLAGS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    FLAGS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Begin (or reset) a cancellable job and return its flag.
pub fn begin(job_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut map) = flags().lock() {
        map.insert(job_id.to_string(), Arc::clone(&flag));
    }
    flag
}

/// Request cancel for an in-flight job.
pub fn request(job_id: &str) {
    if let Ok(map) = flags().lock() {
        if let Some(f) = map.get(job_id) {
            f.store(true, Ordering::SeqCst);
        }
    }
}

/// Drop the flag when the job finishes.
pub fn finish(job_id: &str) {
    if let Ok(mut map) = flags().lock() {
        map.remove(job_id);
    }
}

pub fn is_cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

pub fn check(flag: &AtomicBool) -> Result<()> {
    if is_cancelled(flag) {
        Err(DevStackError::Message("cancelled".into()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_request_finish() {
        let f = begin("unit-cancel");
        assert!(!is_cancelled(&f));
        request("unit-cancel");
        assert!(is_cancelled(&f));
        assert!(check(&f).is_err());
        finish("unit-cancel");
    }
}

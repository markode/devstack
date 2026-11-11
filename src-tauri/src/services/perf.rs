//! Tiny timing + budget helpers shared by services and the CI perf bench.

use std::time::{Duration, Instant};

/// Warm `apply_stack` budget (ms) — slightly above the <3s target to absorb CI jitter.
pub const WARM_APPLY_STACK_BUDGET_MS_UNIX: u128 = 4000;
/// Windows warm apply keeps the database up (`stop_web_stack`); budget matches the
/// <5s product target with a little CI jitter headroom removed — hard 5s gate.
pub const WARM_APPLY_STACK_BUDGET_MS_WINDOWS: u128 = 5_000;
/// HTTPS / web-server toggle budget (ms).
pub const TOGGLE_BUDGET_MS_UNIX: u128 = 3000;
/// Single-bind nginx↔apache toggles reload *both* front-ends (purge stale conf).
/// Measured ~1.4s on Windows CI; keep headroom under the unix 3s ceiling.
pub const TOGGLE_BUDGET_MS_WINDOWS: u128 = 2_500;

/// Filesystem watchers must debounce at least this long to coalesce editor/IDE bursts.
pub const MIN_WATCH_DEBOUNCE_MS: u64 = 300;

pub fn warm_apply_stack_budget_ms() -> u128 {
    if cfg!(windows) {
        WARM_APPLY_STACK_BUDGET_MS_WINDOWS
    } else {
        WARM_APPLY_STACK_BUDGET_MS_UNIX
    }
}

pub fn toggle_budget_ms() -> u128 {
    if cfg!(windows) {
        TOGGLE_BUDGET_MS_WINDOWS
    } else {
        TOGGLE_BUDGET_MS_UNIX
    }
}

/// Wall-clock stopwatch.
#[derive(Debug, Clone, Copy)]
pub struct Stopwatch(Instant);

impl Stopwatch {
    pub fn start() -> Self {
        Self(Instant::now())
    }

    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }

    pub fn elapsed_ms(&self) -> u128 {
        self.0.elapsed().as_millis()
    }
}

/// Run `f`, returning its value and elapsed milliseconds.
pub fn time_ms<T>(f: impl FnOnce() -> T) -> (T, u128) {
    let sw = Stopwatch::start();
    let out = f();
    (out, sw.elapsed_ms())
}

/// `Ok(())` when `ms <= budget`, otherwise a descriptive error string.
pub fn check_budget(label: &str, ms: u128, budget: u128) -> Result<(), String> {
    if ms > budget {
        Err(format!("{label} took {ms}ms (>{budget}ms budget)"))
    } else {
        Ok(())
    }
}

/// Poll `f` with exponential backoff (`first` doubling up to `max_step`) until it
/// returns `Some` or `deadline` elapses. Used for readiness checks so warm
/// starts return within a few ms while cold starts still get the full budget.
pub fn poll_backoff<T>(
    first: Duration,
    max_step: Duration,
    deadline: Duration,
    mut f: impl FnMut() -> Option<T>,
) -> Option<T> {
    let start = Instant::now();
    let mut step = first;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if start.elapsed() >= deadline {
            return None;
        }
        std::thread::sleep(step.min(deadline.saturating_sub(start.elapsed())));
        step = (step * 2).min(max_step);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_ms_measures_and_returns_value() {
        let (v, ms) = time_ms(|| {
            std::thread::sleep(Duration::from_millis(10));
            7
        });
        assert_eq!(v, 7);
        assert!(ms >= 10);
    }

    #[test]
    fn budget_check_flags_overruns() {
        assert!(check_budget("x", 10, 10).is_ok());
        assert!(check_budget("x", 11, 10).unwrap_err().contains("11ms"));
    }

    #[test]
    fn poll_backoff_returns_early_and_times_out() {
        let mut n = 0;
        let got = poll_backoff(
            Duration::from_millis(1),
            Duration::from_millis(4),
            Duration::from_secs(1),
            || {
                n += 1;
                (n >= 3).then_some(n)
            },
        );
        assert_eq!(got, Some(3));
        let none: Option<()> = poll_backoff(
            Duration::from_millis(1),
            Duration::from_millis(2),
            Duration::from_millis(20),
            || None,
        );
        assert!(none.is_none());
    }

    #[test]
    fn db_watcher_debounce_not_below_minimum() {
        assert!(super::super::db_files::DB_WATCH_DEBOUNCE_MS >= MIN_WATCH_DEBOUNCE_MS);
        assert!(super::super::db_files::DB_WATCH_SETTLE_MS >= MIN_WATCH_DEBOUNCE_MS);
    }

    #[test]
    fn budgets_are_ordered() {
        // Platform gates match the product targets in docs/performance.md.
        if cfg!(windows) {
            assert_eq!(warm_apply_stack_budget_ms(), WARM_APPLY_STACK_BUDGET_MS_WINDOWS);
            assert_eq!(toggle_budget_ms(), TOGGLE_BUDGET_MS_WINDOWS);
            assert!(warm_apply_stack_budget_ms() >= 5000);
            assert!(toggle_budget_ms() >= 1000);
        } else {
            assert_eq!(warm_apply_stack_budget_ms(), WARM_APPLY_STACK_BUDGET_MS_UNIX);
            assert_eq!(toggle_budget_ms(), TOGGLE_BUDGET_MS_UNIX);
            assert!(warm_apply_stack_budget_ms() >= 4000);
            assert!(toggle_budget_ms() >= 3000);
        }
    }
}

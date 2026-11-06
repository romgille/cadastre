//! When to run.
//!
//! A pure state machine: it is told what happened and when, and says when
//! the next run is due. It never reads the clock or sleeps, which keeps the
//! timing rules testable without waiting.
//!
//! Three things trigger a run:
//! - Docker events, after a quiet period (debounce), so that `compose up`
//!   with twenty containers produces one run, not twenty. A cap on the total
//!   wait keeps a constantly chattering daemon from postponing a run forever.
//! - A periodic reconcile, which bounds how stale the documentation can get
//!   if the event stream dies without anyone noticing.
//! - A retry after a failed run.

use std::time::{Duration, Instant};

/// How many debounce periods a burst of events can postpone a run.
const MAX_WAIT_FACTOR: u32 = 10;

#[derive(Debug, Clone)]
pub struct Scheduler {
    debounce: Duration,
    reconcile: Duration,
    retry: Duration,
    first_event: Option<Instant>,
    last_event: Option<Instant>,
    next_reconcile: Instant,
    retry_at: Option<Instant>,
}

impl Scheduler {
    /// The first run is due immediately.
    pub fn new(now: Instant, debounce: Duration, reconcile: Duration, retry: Duration) -> Self {
        Self {
            debounce,
            reconcile,
            retry,
            first_event: None,
            last_event: None,
            next_reconcile: now,
            retry_at: None,
        }
    }

    /// A relevant Docker event happened.
    pub fn event(&mut self, now: Instant) {
        self.first_event.get_or_insert(now);
        self.last_event = Some(now);
    }

    /// A run is starting: events seen so far are covered by it. Events that
    /// arrive while it runs are registered after it and cause another run.
    pub fn started(&mut self) {
        self.first_event = None;
        self.last_event = None;
        self.retry_at = None;
    }

    /// A run ended.
    pub fn finished(&mut self, now: Instant, succeeded: bool) {
        self.next_reconcile = now + self.reconcile;
        self.retry_at = (!succeeded).then(|| now + self.retry);
    }

    /// The instant the next run is due.
    pub fn due_at(&self) -> Instant {
        let mut due = self.next_reconcile;
        if let (Some(first), Some(last)) = (self.first_event, self.last_event) {
            let quiet = last + self.debounce;
            let capped = first + self.debounce * MAX_WAIT_FACTOR;
            due = due.min(quiet.min(capped));
        }
        if let Some(retry_at) = self.retry_at {
            due = due.min(retry_at);
        }
        due
    }

    pub fn is_due(&self, now: Instant) -> bool {
        now >= self.due_at()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEBOUNCE: Duration = Duration::from_secs(30);
    const RECONCILE: Duration = Duration::from_secs(3600);
    const RETRY: Duration = Duration::from_secs(60);

    fn after(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    fn scheduler(base: Instant) -> Scheduler {
        let mut s = Scheduler::new(base, DEBOUNCE, RECONCILE, RETRY);
        // Get past the immediate first run.
        s.started();
        s.finished(base, true);
        s
    }

    #[test]
    fn the_first_run_is_due_immediately() {
        let base = Instant::now();
        let s = Scheduler::new(base, DEBOUNCE, RECONCILE, RETRY);
        assert!(s.is_due(base));
    }

    #[test]
    fn nothing_is_due_until_the_reconcile_interval() {
        let base = Instant::now();
        let s = scheduler(base);
        assert!(!s.is_due(after(base, 3599)));
        assert!(s.is_due(after(base, 3600)));
    }

    #[test]
    fn an_event_is_acted_on_after_the_quiet_period() {
        let base = Instant::now();
        let mut s = scheduler(base);
        s.event(after(base, 100));
        assert!(!s.is_due(after(base, 129)));
        assert!(s.is_due(after(base, 130)));
    }

    #[test]
    fn a_burst_of_events_extends_the_quiet_period() {
        let base = Instant::now();
        let mut s = scheduler(base);
        for t in [100, 110, 120, 130] {
            s.event(after(base, t));
        }
        assert!(!s.is_due(after(base, 159)));
        assert!(s.is_due(after(base, 160)));
    }

    #[test]
    fn endless_events_cannot_postpone_a_run_beyond_the_cap() {
        let base = Instant::now();
        let mut s = scheduler(base);
        for t in (100..=1000).step_by(10) {
            s.event(after(base, t));
        }
        // First event at 100, cap of ten debounce periods = 300 s.
        assert_eq!(s.due_at(), after(base, 400));
    }

    #[test]
    fn a_failed_run_is_retried_sooner_than_the_reconcile() {
        let base = Instant::now();
        let mut s = scheduler(base);
        s.started();
        s.finished(after(base, 200), false);
        assert!(!s.is_due(after(base, 259)));
        assert!(s.is_due(after(base, 260)));
    }

    #[test]
    fn a_successful_run_cancels_the_retry() {
        let base = Instant::now();
        let mut s = scheduler(base);
        s.started();
        s.finished(after(base, 200), false);
        s.started();
        s.finished(after(base, 260), true);
        assert!(!s.is_due(after(base, 400)));
    }

    #[test]
    fn starting_a_run_forgets_the_events_it_covers() {
        let base = Instant::now();
        let mut s = scheduler(base);
        s.event(after(base, 100));
        s.started();
        s.finished(after(base, 105), true);
        assert!(!s.is_due(after(base, 200)));
    }

    #[test]
    fn an_event_during_a_run_causes_another_run() {
        let base = Instant::now();
        let mut s = scheduler(base);
        s.started();
        // Registered after `started`, as the daemon does for queued events.
        s.event(after(base, 110));
        s.finished(after(base, 112), true);
        assert!(s.is_due(after(base, 140)));
    }
}

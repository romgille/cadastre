//! The long-running loop: react to Docker events, reconcile periodically,
//! retry after failures, and keep the health signals current.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::config::Config;
use crate::docker::Docker;
use crate::heartbeat;
use crate::log;
use crate::metrics::{self, Status};
use crate::pipeline::run_once;
use crate::schedule::Scheduler;
use crate::timefmt::unix_now;
use crate::VERSION;

/// Intervals that tests shorten.
#[derive(Debug, Clone)]
pub struct Timing {
    /// Wait before retrying after a failed run.
    pub retry: Duration,
    /// Wait before reconnecting a lost event stream.
    pub reconnect: Duration,
    /// Longest the loop sleeps, which bounds how late a shutdown request or
    /// a heartbeat can be.
    pub tick: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            retry: Duration::from_secs(60),
            reconnect: Duration::from_secs(5),
            tick: Duration::from_secs(1),
        }
    }
}

/// Minimum time between heartbeat writes while idle.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);

/// Runs until `shutdown` is set. Returns an error only if the loop cannot
/// continue at all.
pub fn run(
    config: &Config,
    dockers: &[Docker],
    timing: &Timing,
    shutdown: &Arc<AtomicBool>,
) -> Result<()> {
    let (events, wakeups) = mpsc::channel();
    for docker in dockers {
        spawn_event_listener(
            docker.clone(),
            events.clone(),
            Arc::clone(shutdown),
            timing.reconnect,
        );
    }
    drop(events);

    let mut scheduler = Scheduler::new(
        Instant::now(),
        config.debounce,
        config.reconcile,
        timing.retry,
    );
    let mut status = Status::default();
    let mut last_run_ok = true;
    let mut last_heartbeat: Option<Instant> = None;

    while !shutdown.load(Ordering::Relaxed) {
        let wait = scheduler
            .due_at()
            .saturating_duration_since(Instant::now())
            .min(timing.tick);
        match wakeups.recv_timeout(wait) {
            Ok(()) => {
                scheduler.event(Instant::now());
                while wakeups.try_recv().is_ok() {
                    scheduler.event(Instant::now());
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => bail!("the event listener stopped unexpectedly"),
        }

        let mut ran = false;
        if scheduler.is_due(Instant::now()) {
            scheduler.started();
            last_run_ok = run_and_record(config, dockers, &mut status);
            scheduler.finished(Instant::now(), last_run_ok);
            ran = true;
        }

        let heartbeat_due = last_heartbeat.is_none_or(|at| at.elapsed() >= HEARTBEAT_EVERY);
        if ran || heartbeat_due {
            if let Err(error) = heartbeat::write(&config.heartbeat_file, unix_now(), last_run_ok) {
                log::error(format!("cannot write the heartbeat: {error:#}"));
            }
            last_heartbeat = Some(Instant::now());
        }
    }
    log::info("shutting down");
    Ok(())
}

/// One run, with its logging and metrics. Returns whether it succeeded.
fn run_and_record(config: &Config, dockers: &[Docker], status: &mut Status) -> bool {
    let now = unix_now();
    let succeeded = match run_once(config, dockers, now, false) {
        Ok(outcome) => {
            if outcome.report.has_changes() || outcome.changelog_entry {
                log::info(format!(
                    "published {} container(s): {} page(s) written, {} removed",
                    outcome.containers,
                    outcome.report.written.len(),
                    outcome.report.removed.len()
                ));
            }
            status.record_success(now, outcome.containers);
            true
        }
        Err(error) => {
            log::error(format!("run failed: {error:#}"));
            status.record_failure(now);
            false
        }
    };
    if let Some(dir) = &config.metrics_dir {
        if let Err(error) = metrics::write(dir, status, VERSION) {
            log::error(format!("cannot write metrics: {error:#}"));
        }
    }
    succeeded
}

/// Forwards relevant Docker events as wake-ups, and reconnects when the
/// stream ends. After a reconnection it also sends one wake-up, because
/// events may have been missed in the meantime.
fn spawn_event_listener(
    docker: Docker,
    wakeups: Sender<()>,
    shutdown: Arc<AtomicBool>,
    reconnect: Duration,
) {
    let label = docker
        .host()
        .map_or_else(String::new, |host| format!(" ({host})"));
    thread::spawn(move || {
        let mut first_connection = true;
        while !shutdown.load(Ordering::Relaxed) {
            match docker.events() {
                Ok(stream) => {
                    if !first_connection && wakeups.send(()).is_err() {
                        return;
                    }
                    first_connection = false;
                    for item in stream {
                        if shutdown.load(Ordering::Relaxed) {
                            return;
                        }
                        match item {
                            Ok(event) if event.is_relevant() => {
                                if wakeups.send(()).is_err() {
                                    return;
                                }
                            }
                            Ok(_) => {}
                            Err(error) => {
                                log::warn(format!("event stream{label}: {error:#}"));
                                break;
                            }
                        }
                    }
                    log::warn(format!("event stream{label} closed, reconnecting"));
                }
                Err(error) => log::warn(format!("cannot open the event stream{label}: {error:#}")),
            }
            thread::sleep(reconnect);
        }
    });
}

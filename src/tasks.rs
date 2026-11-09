//! DevTools → Tasks: every background job Files runs (thumbnails, Quick
//! Look renders, file details, folder sizes, cache trimming…) with its
//! state, timing and error, so a stuck or failing pipeline is visible.
//! Workers report here; Retry asks the job's kind to run it again.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

impl State {
    pub fn label(&self) -> &'static str {
        match self {
            State::Queued => "Queued",
            State::Running => "Running",
            State::Done => "Done",
            State::Failed(_) => "Failed",
            State::Cancelled => "Cancelled",
        }
    }

    pub fn finished(&self) -> bool {
        matches!(self, State::Done | State::Failed(_) | State::Cancelled)
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: u64,
    /// "thumbnail", "quick look", "details"…
    pub kind: &'static str,
    /// What it works on: usually a path.
    pub label: String,
    pub state: State,
    pub queued: Instant,
    pub started: Option<Instant>,
    pub finished: Option<Instant>,
}

impl Job {
    /// How long it ran (or has been running).
    pub fn duration(&self) -> Option<Duration> {
        let started = self.started?;
        Some(self.finished.unwrap_or_else(Instant::now).duration_since(started))
    }

    /// How long it waited in the queue.
    pub fn wait(&self) -> Duration {
        self.started.unwrap_or_else(Instant::now).duration_since(self.queued)
    }
}

/// Finished jobs kept for the list; running and queued ones always are.
const KEEP_FINISHED: usize = 400;

#[derive(Default)]
struct Registry {
    next: u64,
    jobs: VecDeque<Job>,
    retry: Vec<(&'static str, fn(&str))>,
}

static REGISTRY: Mutex<Option<Registry>> = Mutex::new(None);

fn with<T>(change: impl FnOnce(&mut Registry) -> T) -> T {
    let mut guard = REGISTRY.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    change(guard.get_or_insert_with(Default::default))
}

/// A job waiting for its worker; returns its id.
pub fn queue(kind: &'static str, label: impl Into<String>) -> u64 {
    with(|registry| {
        registry.next += 1;
        let id = registry.next;
        registry.jobs.push_back(Job { id, kind, label: label.into(), state: State::Queued, queued: Instant::now(), started: None, finished: None });
        trim(registry);
        id
    })
}

/// The worker takes it; `false` when it was cancelled meanwhile (skip it).
pub fn start(id: u64) -> bool {
    with(|registry| match registry.jobs.iter_mut().find(|job| job.id == id) {
        Some(job) if job.state == State::Cancelled => false,
        Some(job) => {
            job.state = State::Running;
            job.started = Some(Instant::now());
            true
        }
        None => true,
    })
}

pub fn finish(id: u64, result: Result<(), String>) {
    with(|registry| {
        if let Some(job) = registry.jobs.iter_mut().find(|job| job.id == id) {
            job.state = match result {
                Ok(()) => State::Done,
                Err(error) => State::Failed(error),
            };
            job.finished = Some(Instant::now());
        }
        trim(registry);
    });
}

/// A job run right here, start to finish.
pub fn run<T, E: std::fmt::Display>(kind: &'static str, label: impl Into<String>, work: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    let id = queue(kind, label);
    start(id);
    let result = work();
    finish(id, result.as_ref().map(|_| ()).map_err(ToString::to_string));
    result
}

/// Queued jobs are dropped before they run; a running one finishes.
pub fn cancel(id: u64) {
    with(|registry| {
        if let Some(job) = registry.jobs.iter_mut().find(|job| job.id == id && job.state == State::Queued) {
            job.state = State::Cancelled;
            job.finished = Some(Instant::now());
        }
    });
}

/// How a kind of job is run again (the label it was given).
pub fn on_retry(kind: &'static str, retry: fn(&str)) {
    with(|registry| {
        if !registry.retry.iter().any(|(known, _)| *known == kind) {
            registry.retry.push((kind, retry));
        }
    });
}

pub fn can_retry(kind: &str) -> bool {
    with(|registry| registry.retry.iter().any(|(known, _)| *known == kind))
}

pub fn retry(id: u64) {
    let found = with(|registry| {
        let job = registry.jobs.iter().find(|job| job.id == id)?.clone();
        let retry = registry.retry.iter().find(|(kind, _)| *kind == job.kind)?.1;
        Some((retry, job.label))
    });
    if let Some((retry, label)) = found {
        retry(&label);
    }
}

/// Drops finished jobs from the list.
pub fn clear_finished() {
    with(|registry| registry.jobs.retain(|job| !job.state.finished()));
}

pub fn jobs() -> Vec<Job> {
    with(|registry| registry.jobs.iter().cloned().collect())
}

fn trim(registry: &mut Registry) {
    let finished = registry.jobs.iter().filter(|job| job.state.finished()).count();
    let mut extra = finished.saturating_sub(KEEP_FINISHED);
    registry.jobs.retain(|job| {
        if extra > 0 && job.state.finished() {
            extra -= 1;
            false
        } else {
            true
        }
    });
}

// ------------------------------------------------------------- search timing

/// One search, split into where its time went (DevTools → Search).
#[derive(Clone, Debug, PartialEq)]
pub struct SearchProfile {
    /// "Folder" or "Global".
    pub scope: &'static str,
    pub query: String,
    pub candidates: usize,
    pub results: usize,
    /// Reading the folder (or the search index), filtering, matching, sorting.
    pub list: Duration,
    pub matching: Duration,
    pub sort: Duration,
}

impl SearchProfile {
    pub fn total(&self) -> Duration {
        self.list + self.matching + self.sort
    }
}

static SEARCHES: Mutex<VecDeque<SearchProfile>> = Mutex::new(VecDeque::new());

pub fn record_search(profile: SearchProfile) {
    let mut searches = SEARCHES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    searches.push_back(profile);
    while searches.len() > 50 {
        searches.pop_front();
    }
}

pub fn searches() -> Vec<SearchProfile> {
    SEARCHES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_move_through_their_states_and_can_be_cancelled_or_retried() {
        let done = queue("test-kind", "a");
        assert!(start(done));
        finish(done, Ok(()));
        let failed = run("test-kind", "b", || Err::<(), _>("broken"));
        assert!(failed.is_err());
        let skipped = queue("test-kind", "c");
        cancel(skipped);
        assert!(!start(skipped), "a cancelled job is skipped by its worker");
        let state = |id: u64| jobs().into_iter().find(|job| job.id == id).map(|job| job.state);
        assert_eq!(state(done), Some(State::Done));
        assert_eq!(state(skipped), Some(State::Cancelled));
        assert!(matches!(state(done + 1), Some(State::Failed(error)) if error == "broken"));
        static RETRIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        on_retry("test-kind", |label| RETRIED.store(label == "b", std::sync::atomic::Ordering::Relaxed));
        retry(done + 1);
        assert!(RETRIED.load(std::sync::atomic::Ordering::Relaxed), "Retry runs the job's own kind again");
    }
}

//! Background jobs, the pattern of PhotoCraft's `engine/src/jobs.rs`: a long command runs on
//! worker threads and reports back through a channel; the session applies what arrives on its
//! own thread, so session state is only ever changed in one place.
//!
//! - [`Session::execute`] runs every command to completion: tests, the CLI and scripts never see
//!   a job.
//! - [`Session::start`] runs a job-capable command in the background and returns its [`JobId`];
//!   [`Session::poll_jobs`] (the UI calls it every frame) applies progress as it arrives and
//!   returns the jobs that ended.
//! - [`Session::wait_job`] blocks until a job ends; [`Session::cancel_job`] stops one.
//! - Workers never touch the session. A panic in a worker becomes an error for the item it was on.
//!
//! Jobs work through a list of apps on a small worker pool ([`spawn_pool`]), one message per
//! app, so rows update as results arrive: `updates.check` (`update_cmds`) and `icons.refresh`
//! (`icons`). Install jobs (M2) add their own kind.

use std::collections::{BTreeSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use serde_json::Value;

use crate::icons::{IconMsg, IconSummary};
use crate::install_cmds::{InstallMsg, InstallState};
use crate::update_cmds::{CheckMsg, CheckSummary};
use crate::{EngineError, Result, Session};

/// Identifies one background job for the session's lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct JobId(pub u64);

/// What [`Session::start`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum Started {
    /// The command ran inline; its result.
    Done(Value),
    /// The command runs in the background.
    Job(JobId),
}

/// A running job, for progress displays and `jobs` listings.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub id: JobId,
    pub command: String,
    pub label: String,
    pub done: usize,
    pub total: usize,
    /// Items being worked on right now (app ids).
    pub in_flight: Vec<String>,
    /// What the job is doing now, when it says (`Downloading`, `Verifying`, `Installing`).
    pub phase: Option<String>,
    /// How far along, 0..=1, when known.
    pub fraction: Option<f32>,
}

/// A job that ended, as [`Session::poll_jobs`] reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct JobEvent {
    pub id: JobId,
    pub command: String,
    pub result: std::result::Result<Value, String>,
}

/// What a worker reports.
pub(crate) enum JobMsg {
    /// Work on this app began.
    Started(String),
    Check(CheckMsg),
    Icon(IconMsg),
    Install(InstallMsg),
}

/// A job's own bookkeeping, by kind.
pub(crate) enum JobState {
    Check(CheckSummary),
    Icons(IconSummary),
    Install(InstallState),
}

impl JobState {
    fn done(&self) -> usize {
        match self {
            JobState::Check(s) => s.done(),
            JobState::Icons(s) => s.done(),
            JobState::Install(s) => usize::from(s.outcome.is_some()),
        }
    }
}

pub(crate) struct Running {
    pub(crate) id: JobId,
    pub(crate) command: &'static str,
    pub(crate) label: String,
    pub(crate) rx: Receiver<JobMsg>,
    pub(crate) cancel: Arc<AtomicBool>,
    /// Every item the job covers.
    pub(crate) items: Vec<String>,
    pub(crate) in_flight: BTreeSet<String>,
    pub(crate) state: JobState,
}

impl Running {
    fn info(&self) -> JobInfo {
        JobInfo {
            id: self.id,
            command: self.command.into(),
            label: self.label.clone(),
            done: self.state.done(),
            total: self.items.len(),
            in_flight: self.in_flight.iter().cloned().collect(),
            phase: match &self.state {
                JobState::Install(s) => Some(s.phase.clone()),
                _ => None,
            },
            fraction: match &self.state {
                JobState::Install(s) => s.fraction(),
                _ => None,
            },
        }
    }
}

#[derive(Default)]
pub(crate) struct Jobs {
    next: u64,
    running: Vec<Running>,
}

impl Jobs {
    pub(crate) fn next_id(&mut self) -> JobId {
        self.next = self.next.saturating_add(1);
        JobId(self.next)
    }

    pub(crate) fn push(&mut self, job: Running) {
        self.running.push(job);
    }

    /// Is a job running `command`?
    pub(crate) fn runs(&self, command: &str) -> bool {
        self.running.iter().any(|r| r.command == command)
    }

    /// Is a job of `command` working on `app` right now?
    pub(crate) fn working_on(&self, command: &str, app: &str) -> bool {
        self.running.iter().any(|r| r.command == command && r.in_flight.contains(app))
    }
}

/// One item of a pooled job.
pub(crate) trait PoolTask: Send + 'static {
    fn app(&self) -> &str;
    /// The message for this item when its work panicked.
    fn panicked(&self) -> JobMsg;
}

/// Work through `tasks` on up to `workers` threads: for each, send [`JobMsg::Started`], then
/// whatever `work` returns. A panic in `work` becomes the task's [`PoolTask::panicked`] message.
/// Workers stop taking tasks once `cancel` is set or the receiver is gone; when the last one
/// ends the channel disconnects, which is how the session learns the job is over.
pub(crate) fn spawn_pool<T: PoolTask>(
    name: &'static str,
    tasks: Vec<T>,
    workers: usize,
    cancel: Arc<AtomicBool>,
    tx: Sender<JobMsg>,
    work: impl Fn(&T) -> Vec<JobMsg> + Send + Sync + 'static,
) {
    let n = tasks.len().min(workers);
    let queue = Arc::new(Mutex::new(VecDeque::from(tasks)));
    let work = Arc::new(work);
    for i in 0..n {
        let (queue, cancel, tx, work) = (Arc::clone(&queue), Arc::clone(&cancel), tx.clone(), Arc::clone(&work));
        let spawned = std::thread::Builder::new().name(format!("{name}-{i}")).spawn(move || {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let next = queue.lock().unwrap_or_else(PoisonError::into_inner).pop_front();
                let Some(task) = next else { return };
                if tx.send(JobMsg::Started(task.app().to_string())).is_err() {
                    return;
                }
                let msgs = catch_unwind(AssertUnwindSafe(|| work(&task))).unwrap_or_else(|_| {
                    log::error!("{name}: {} panicked", task.app());
                    vec![task.panicked()]
                });
                for m in msgs {
                    if tx.send(m).is_err() {
                        return;
                    }
                }
            }
        });
        if let Err(e) = spawned {
            log::error!("couldn't start a {name} worker: {e}");
        }
    }
}

impl Session {
    /// Run a command, in the background when it supports it. Errors like [`Session::execute`]
    /// for unknown, disabled or failing commands.
    pub fn start(&mut self, id: &str, params: Value) -> Result<Started> {
        let (spec, params) = self.prepare(id, params)?;
        match spec.start {
            Some(start) => catch_unwind(AssertUnwindSafe(|| start(self, &params)))
                .unwrap_or_else(|_| {
                    log::error!("starting `{id}` panicked");
                    Err(EngineError::Internal(spec.id.into()))
                })
                .map(Started::Job),
            None => self.execute(id, params).map(Started::Done),
        }
    }

    /// Apply what running jobs reported and return the jobs that ended since the last call.
    /// Cheap when nothing runs; the UI calls it every frame.
    pub fn poll_jobs(&mut self) -> Vec<JobEvent> {
        let mut events = Vec::new();
        let mut keep = Vec::new();
        for mut job in std::mem::take(&mut self.jobs.running) {
            let ended = loop {
                match job.rx.try_recv() {
                    Ok(msg) => self.apply(&mut job, msg),
                    Err(TryRecvError::Empty) => break false,
                    // Every worker is gone: the job is over.
                    Err(TryRecvError::Disconnected) => break true,
                }
            };
            if ended {
                events.push(self.finish(job));
            } else {
                keep.push(job);
            }
        }
        keep.append(&mut self.jobs.running);
        self.jobs.running = keep;
        events
    }

    /// Block until job `id` ends, apply it, and return its result.
    pub fn wait_job(&mut self, id: JobId) -> Result<Value> {
        let Some(i) = self.jobs.running.iter().position(|r| r.id == id) else {
            return Err(EngineError::Job(format!("no running job {}", id.0)));
        };
        let mut job = self.jobs.running.remove(i);
        while let Ok(msg) = job.rx.recv() {
            self.apply(&mut job, msg);
        }
        self.finish(job).result.map_err(EngineError::Job)
    }

    /// Stop job `id`: its workers stop before their next item and what they still send is
    /// dropped. Results already applied stay. False if no such job runs.
    pub fn cancel_job(&mut self, id: JobId) -> bool {
        let Some(i) = self.jobs.running.iter().position(|r| r.id == id) else { return false };
        let job = self.jobs.running.remove(i);
        job.cancel.store(true, Ordering::Relaxed);
        true
    }

    /// Is any job running?
    pub fn has_jobs(&self) -> bool {
        !self.jobs.running.is_empty()
    }

    /// The running jobs.
    pub fn jobs(&self) -> Vec<JobInfo> {
        self.jobs.running.iter().map(Running::info).collect()
    }

    /// The running job of `command` that covers `app`, if any.
    pub fn job_for(&self, command: &str, app: &str) -> Option<JobInfo> {
        self.jobs.running.iter().find(|r| r.command == command && r.items.iter().any(|i| i == app)).map(Running::info)
    }

    fn apply(&mut self, job: &mut Running, msg: JobMsg) {
        match msg {
            JobMsg::Started(app) => {
                job.in_flight.insert(app);
            }
            JobMsg::Check(m) => crate::update_cmds::apply(self, job, m),
            JobMsg::Icon(m) => crate::icons::apply(self, job, m),
            JobMsg::Install(m) => crate::install_cmds::apply(self, job, m),
        }
    }

    fn finish(&mut self, job: Running) -> JobEvent {
        let (id, command) = (job.id, job.command.to_string());
        let result = match job.state {
            JobState::Check(_) => Ok(crate::update_cmds::finish(self, job)),
            JobState::Icons(_) => Ok(crate::icons::finish(job)),
            JobState::Install(_) => crate::install_cmds::finish(job),
        };
        JobEvent { id, command, result }
    }
}

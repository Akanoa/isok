pub use log::info;
pub use reqwest::Client as ReqwestClient;
pub use slab::Slab;
pub use std::collections::HashMap;
pub use std::future::Future;
pub use std::pin::Pin;
pub use std::sync::{Arc, Mutex};
pub use std::time::Duration;
pub use tokio::task::JoinHandle;
pub use tokio_util::task::LocalPoolHandle;
pub use uuid::Uuid;

pub use ping_data::check::{CheckKind, CheckOutput};
pub use ping_data::pulsar_commands::{Command, CommandKind};

pub use crate::http::{HttpClient, HttpContext};
pub use crate::magic_pool::MagicPool;

pub struct JobRessources {
    pub http_pool: MagicPool<HttpClient>,
}

impl Default for JobRessources {
    fn default() -> Self {
        JobRessources {
            http_pool: MagicPool::new(10),
        }
    }
}

#[derive(Debug, Clone)]
pub enum JobKind {
    Http(HttpContext),
    Dummy,
}

#[derive(Debug, Clone)]
pub struct Job {
    id: Uuid,
    kind: JobKind,
}

impl Job {
    fn execute_dummy(id: &Uuid, task_pool: &LocalPoolHandle) {
        let borowed_id = id.clone();
        let process = async move {
            info!("Check {borowed_id} has been trigerred !");
        };

        info!("Triggering check {id}...");
        task_pool.spawn_pinned(|| process);
    }

    fn execute_http(
        id: &Uuid,
        ctx: HttpContext,
        task_pool: &LocalPoolHandle,
        mut resources: &mut JobRessources,
    ) {
        let borowed_id = id.clone();
        let borowed_req = ctx.clone().into();
        let checkout = resources.http_pool.get();
        let process = async move {
            let http_result = checkout.send(borowed_req).await;

            match http_result {
                Some(res) => {
                    ();
                    info!(
                        "Check http {borowed_id} has been trigerred with status {} and response time {} !",
                        res.status
                        , res.request_time.as_millis()
                    );
                }
                None => todo!(),
            };
        };

        info!("Triggering check http {id} at {} ...", ctx.url());
        task_pool.spawn_pinned(|| process);
    }

    pub fn execute(&self, task_pool: &LocalPoolHandle, mut resources: &mut JobRessources) {
        match &self.kind {
            JobKind::Dummy => Self::execute_dummy(&self.id, task_pool),
            JobKind::Http(ctx) => Self::execute_http(&self.id, ctx.clone(), task_pool, resources),
        }
    }
}

impl From<CheckOutput> for Job {
    fn from(value: CheckOutput) -> Self {
        let kind = match value.kind {
            CheckKind::Http(http) => JobKind::Http(HttpContext::from(http)),
            _ => JobKind::Dummy,
        };

        Self { id: value.id, kind }
    }
}

pub struct JobLocation {
    frequency: Duration,
    offset: usize,
    position: usize,
}

pub struct JobScheduler {
    fill_cursor: usize,
    empty_slot: Vec<usize>,
    jobs: Arc<Mutex<Vec<Slab<Job>>>>,
    process: JoinHandle<()>,
}

impl JobScheduler {
    pub fn new(
        range: usize,
        wait: Duration,
        resources: Arc<Mutex<JobRessources>>,
        task_pool_size: usize,
    ) -> Self {
        let jobs = {
            let mut res: Vec<Slab<Job>> = Vec::with_capacity(range);

            for _ in 0..range {
                res.push(Slab::new())
            }
            res
        };

        let jobs = Arc::new(Mutex::new(jobs));
        let job_list = Arc::clone(&jobs);

        let process = async move {
            let task_pool = LocalPoolHandle::new(task_pool_size);
            let mut time_cursor = 0;

            loop {
                if let Ok(mut jl) = job_list.lock() {
                    for (_, j) in &mut jl[time_cursor] {
                        if let Ok(mut resources) = resources.lock() {
                            j.execute(&task_pool, &mut resources)
                        }
                    }
                }

                std::thread::sleep(wait);
                if time_cursor + 1 == range {
                    time_cursor = 0;
                } else {
                    time_cursor += 1;
                }
            }
        };

        Self {
            fill_cursor: 0,
            empty_slot: Vec::new(),
            jobs,
            process: tokio::task::spawn(process),
        }
    }

    pub fn add_job(&mut self, job: Job) -> (usize, usize) {
        let mut cursor = self.fill_cursor;
        let mut position = 0;
        if let Ok(mut jobs) = self.jobs.lock() {
            if self.empty_slot.is_empty() {
                position = jobs[cursor].insert(job);
                if self.fill_cursor + 1 == jobs.len() {
                    self.fill_cursor = 0;
                } else {
                    self.fill_cursor += 1;
                }
            } else {
                cursor = self.empty_slot.pop().unwrap();
                position = jobs[cursor].insert(job);
            }
        }

        (cursor, position)
    }
}

pub struct JobsHandler {
    resources: Arc<Mutex<JobRessources>>,
    checks: HashMap<Uuid, JobLocation>,
    jobs: HashMap<Duration, JobScheduler>,
    scheduler_task_pool_size: usize,
}

impl JobsHandler {
    pub fn new(resources: JobRessources, scheduler_task_pool_size: usize) -> Self {
        Self {
            resources: Arc::new(Mutex::new(resources)),
            checks: HashMap::new(),
            jobs: HashMap::new(),
            scheduler_task_pool_size,
        }
    }

    pub fn handle_command(&mut self, cmd: Command) {
        match cmd.kind() {
            CommandKind::Add(a) => self.add_check(&a.check),
            CommandKind::Remove(r) => todo!(),
        }
    }

    pub fn add_check(&mut self, c: &CheckOutput) {
        let frequency = c.interval;

        if !self.jobs.contains_key(&frequency) {
            self.jobs.insert(
                frequency,
                JobScheduler::new(
                    frequency.as_secs() as usize,
                    Duration::from_secs(1),
                    Arc::clone(&self.resources),
                    self.scheduler_task_pool_size,
                ),
            );
        }

        let (offset, position) = self
            .jobs
            .get_mut(&frequency)
            .unwrap()
            .add_job(Job::from(c.clone()));

        self.checks.insert(
            c.id,
            JobLocation {
                frequency,
                offset,
                position,
            },
        );
    }
}
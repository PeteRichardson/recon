//! Where jobs run (#455, ADR 0007): a pool of worker threads in the app,
//! and the caller's own thread in tests.

use super::cache::Cache;
use super::{Done, Job, run};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

/// The job for the file on screen, numbered so that the answer for a file
/// the user has already left can be told apart and dropped.
#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub(crate) generation: u64,
    pub(crate) job: Job,
}

/// A request's result.
#[derive(Debug)]
pub(crate) struct Answer {
    pub(crate) generation: u64,
    pub(crate) done: Done,
}

/// Runs jobs. `App` holds one: a `Pool` in the app, `Immediate` in tests.
pub(crate) trait Executor {
    /// Run the job for the file on screen, before every prefetch job. A
    /// request not yet started is replaced: its file is no longer on screen.
    fn submit(&mut self, request: Request);
    /// Replace the queued prefetch jobs with `jobs`. Their results go to the
    /// cache only. A queued job not in `jobs` is no longer near the
    /// selection, and is dropped.
    fn prefetch(&mut self, jobs: Vec<Job>);
    /// The next answer, waiting no more than `timeout`.
    fn recv_timeout(&mut self, timeout: Duration) -> Option<Answer>;
}

/// Runs each request in the caller, at once, so that a test sees the
/// result in the same step as the request, as it did before the pipeline.
/// Prefetches nothing and caches nothing.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct Immediate {
    answers: VecDeque<Answer>,
}

#[cfg(test)]
impl Executor for Immediate {
    fn submit(&mut self, request: Request) {
        let done = run(&request.job, None);
        self.answers.push_back(Answer {
            generation: request.generation,
            done,
        });
    }

    fn prefetch(&mut self, _: Vec<Job>) {}

    fn recv_timeout(&mut self, _: Duration) -> Option<Answer> {
        self.answers.pop_front()
    }
}

/// Runs nothing, and never answers. So `App` can hold a `Box<dyn Executor>`
/// in a `#[derive(Default)]` struct, as it does for its scanner, without a
/// default that reads files on the main thread. `App::new` replaces it.
struct NoExecutor;

impl Executor for NoExecutor {
    fn submit(&mut self, _: Request) {}

    fn prefetch(&mut self, _: Vec<Job>) {}

    fn recv_timeout(&mut self, _: Duration) -> Option<Answer> {
        None
    }
}

impl Default for Box<dyn Executor> {
    fn default() -> Self {
        Box::new(NoExecutor)
    }
}

/// What a worker does with a job. Only tests replace it.
type Runner = Arc<dyn Fn(&Job, &Cache) -> Done + Send + Sync>;

#[derive(Default)]
struct Queue {
    current: Option<Request>,
    prefetch: VecDeque<Job>,
    /// Prefetch jobs running now.
    prefetching: usize,
    /// Workers started.
    workers: usize,
    stopped: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    cache: Cache,
    runner: Runner,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// `cores − 2` worker threads, at least one, with one shared cache. The
/// two cores left over are the main thread's and the explorer scan's.
pub(crate) struct Pool {
    shared: Arc<Shared>,
    answers: Receiver<Answer>,
    sender: Sender<Answer>,
}

impl Pool {
    pub(crate) fn new() -> Self {
        let workers = std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .saturating_sub(2)
            .max(1);
        let mut pool = Self::paused(Arc::new(|job, cache| run(job, Some(cache))));
        pool.start(workers);
        pool
    }

    /// A pool with no workers yet: jobs queue until `start`.
    fn paused(runner: Runner) -> Self {
        let (sender, answers) = channel();
        Self {
            shared: Arc::new(Shared {
                queue: Mutex::default(),
                ready: Condvar::new(),
                cache: Cache::default(),
                runner,
            }),
            answers,
            sender,
        }
    }

    fn start(&mut self, workers: usize) {
        self.shared.lock().workers += workers;
        for index in 0..workers {
            let shared = Arc::clone(&self.shared);
            let sender = self.sender.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("recon-view-{index}"))
                .spawn(move || work(&shared, &sender));
            if let Err(err) = spawned {
                log::warn!("cannot start a view worker: {err}");
            }
        }
    }
}

/// One worker: the request first, then prefetch, until the pool goes.
fn work(shared: &Shared, answers: &Sender<Answer>) {
    loop {
        let (job, generation) = {
            let mut queue = shared.lock();
            loop {
                if queue.stopped {
                    return;
                }
                if let Some(request) = queue.current.take() {
                    break (request.job, Some(request.generation));
                }
                // One worker is kept for the file on screen, so that it never
                // waits behind a prefetch read. With one worker there is
                // none to spare, and prefetch shares it.
                if queue.prefetching < queue.workers.saturating_sub(1).max(1)
                    && let Some(job) = queue.prefetch.pop_front()
                {
                    queue.prefetching += 1;
                    break (job, None);
                }
                queue = shared
                    .ready
                    .wait(queue)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let done = (shared.runner)(&job, &shared.cache);
        if let Some(generation) = generation {
            // The app has gone when this fails, and so has the reason to send.
            let _ = answers.send(Answer { generation, done });
        } else {
            shared.lock().prefetching -= 1;
            shared.ready.notify_one();
        }
    }
}

impl Executor for Pool {
    fn submit(&mut self, request: Request) {
        self.shared.lock().current = Some(request);
        self.shared.ready.notify_one();
    }

    fn prefetch(&mut self, jobs: Vec<Job>) {
        self.shared.lock().prefetch = jobs.into();
        self.shared.ready.notify_all();
    }

    fn recv_timeout(&mut self, timeout: Duration) -> Option<Answer> {
        self.answers.recv_timeout(timeout).ok()
    }
}

impl Drop for Pool {
    /// Workers stop at their next job. One busy with a read finishes it
    /// first; nothing waits for it.
    fn drop(&mut self) {
        self.shared.lock().stopped = true;
        self.shared.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::fixture_file;
    use crate::views::cycle::{Steps, Table};
    use crate::views::{Extent, read::Contents};
    use std::path::PathBuf;

    fn job(path: &str) -> Job {
        let table = Table::default();
        Job {
            path: PathBuf::from(path),
            extent: Extent::Preview,
            steps: Steps::new(&table),
            table: Arc::new(table),
            px: 0,
            prefetch: false,
        }
    }

    /// A pool whose workers record each job's path and run nothing.
    fn recording() -> (Pool, Arc<Mutex<Vec<PathBuf>>>) {
        let ran = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&ran);
        let pool = Pool::paused(Arc::new(move |job: &Job, _: &Cache| {
            log.lock().unwrap().push(job.path.clone());
            Done {
                path: job.path.clone(),
                contents: Contents::default(),
                shown: None,
                facts: None,
            }
        }));
        (pool, ran)
    }

    /// Wait until the one worker has run `count` jobs.
    fn ran(log: &Mutex<Vec<PathBuf>>, count: usize) -> Vec<PathBuf> {
        for _ in 0..500 {
            let ran = log.lock().unwrap().clone();
            if ran.len() >= count {
                return ran;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        log.lock().unwrap().clone()
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn the_file_on_screen_goes_before_prefetch() {
        let (mut pool, log) = recording();
        pool.prefetch(vec![job("a"), job("b")]);
        pool.submit(Request {
            generation: 1,
            job: job("on-screen"),
        });
        pool.start(1);

        assert_eq!(ran(&log, 3), paths(&["on-screen", "a", "b"]));
        let answer = pool
            .recv_timeout(Duration::from_secs(1))
            .expect("an answer");
        assert_eq!(answer.generation, 1);
        assert!(
            pool.recv_timeout(Duration::from_millis(20)).is_none(),
            "a prefetch job answered the app"
        );
    }

    #[test]
    fn prefetch_far_from_the_selection_is_dropped() {
        let (mut pool, log) = recording();
        pool.prefetch(vec![job("far-1"), job("far-2")]);
        pool.prefetch(vec![job("near")]);
        pool.start(1);

        assert_eq!(ran(&log, 1), paths(&["near"]));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(log.lock().unwrap().len(), 1, "a dropped job ran");
    }

    #[test]
    fn a_new_request_replaces_one_not_yet_started() {
        let (mut pool, log) = recording();
        pool.submit(Request {
            generation: 1,
            job: job("left"),
        });
        pool.submit(Request {
            generation: 2,
            job: job("on-screen"),
        });
        pool.start(1);

        assert_eq!(ran(&log, 1), paths(&["on-screen"]));
        let answer = pool
            .recv_timeout(Duration::from_secs(1))
            .expect("an answer");
        assert_eq!(answer.generation, 2);
    }

    #[test]
    fn a_real_pool_reads_the_file() {
        let path = fixture_file("pool_real.txt", b"one\ntwo\n");
        let mut pool = Pool::new();
        let mut request = job("");
        request.path = path;
        pool.submit(Request {
            generation: 7,
            job: request,
        });

        let answer = pool
            .recv_timeout(Duration::from_secs(5))
            .expect("an answer");
        assert_eq!(answer.generation, 7);
        assert_eq!(*answer.done.contents.lines, ["one", "two"]);
    }
}

/// Test doubles. `pub(crate)` so `app`'s tests can install one.
#[cfg(test)]
pub(crate) mod double {
    use super::{Answer, Executor, Immediate, Request};
    use crate::views::Job;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Default)]
    struct State {
        held: Vec<Request>,
        ran: Immediate,
        prefetched: Vec<PathBuf>,
    }

    /// Holds every request until `release`, so that a test sees what the
    /// app does while a view is late. Records the prefetch it was given.
    #[derive(Clone, Default)]
    pub(crate) struct Held(Arc<Mutex<State>>);

    impl Held {
        /// Run every held request, oldest first; their answers arrive at
        /// the app's next wake.
        pub(crate) fn release(&self) {
            let mut state = self.0.lock().unwrap();
            for request in std::mem::take(&mut state.held) {
                state.ran.submit(request);
            }
        }

        /// The files of the last prefetch, in queue order.
        pub(crate) fn prefetched(&self) -> Vec<PathBuf> {
            self.0.lock().unwrap().prefetched.clone()
        }
    }

    impl Executor for Held {
        fn submit(&mut self, request: Request) {
            self.0.lock().unwrap().held.push(request);
        }

        fn prefetch(&mut self, jobs: Vec<Job>) {
            self.0.lock().unwrap().prefetched = jobs.into_iter().map(|job| job.path).collect();
        }

        fn recv_timeout(&mut self, timeout: Duration) -> Option<Answer> {
            self.0.lock().unwrap().ran.recv_timeout(timeout)
        }
    }
}

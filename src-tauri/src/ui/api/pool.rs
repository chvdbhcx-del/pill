// The worker pool behind `ui::api::call_async`: plain std threads, never the native UI thread, never a tokio worker (the `async fn` commands are
// driven with `block_on` inside a job, which needs a thread that is not already inside the runtime).
// It grows on demand: a job that finds no idle worker gets a new thread (up to MAX), so a quick command (`ai_transform_cancel`, `chat_stop`,
// `capture_cancel`) is never queued behind long ones (`ai_transform`, `voice_test_mic`, `aisetup_*`). Idle workers above MIN exit after IDLE.
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const MIN: usize = 2;
const MAX: usize = 16;
const IDLE: Duration = Duration::from_secs(30);

type Job = Box<dyn FnOnce() + Send + 'static>;

struct Pool {
    tx: Sender<Job>,
    rx: Arc<Mutex<Receiver<Job>>>,
    idle: Arc<AtomicUsize>,   // workers waiting for a job
    queued: AtomicUsize,      // jobs sent and not yet picked up
    total: Arc<AtomicUsize>,  // live workers
}

static POOL: OnceLock<Pool> = OnceLock::new();

fn pool() -> &'static Pool {
    POOL.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        Pool { tx, rx: Arc::new(Mutex::new(rx)), idle: Arc::default(), queued: AtomicUsize::new(0), total: Arc::default() }
    })
}

fn worker(p: &'static Pool) {
    loop {
        p.idle.fetch_add(1, Ordering::AcqRel);
        let got = p.rx.lock().unwrap_or_else(|e| e.into_inner()).recv_timeout(IDLE);
        p.idle.fetch_sub(1, Ordering::AcqRel);
        match got {
            Ok(job) => {
                p.queued.fetch_sub(1, Ordering::AcqRel);
                // a panicking job (even in its own completion callback) must not take the worker down
                let _ = catch_unwind(AssertUnwindSafe(job));
            }
            Err(RecvTimeoutError::Timeout) => {
                // leave only if the pool stays at MIN or more, and nothing was queued meanwhile
                if p.queued.load(Ordering::Acquire) == 0 && p.total.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| (n > MIN).then(|| n - 1)).is_ok() {
                    return;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                p.total.fetch_sub(1, Ordering::AcqRel);
                return;
            }
        }
    }
}

fn add_worker(p: &'static Pool) -> bool {
    if p.total.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| (n < MAX).then(|| n + 1)).is_err() {
        return false;
    }
    if std::thread::Builder::new().name("pill-api".into()).spawn(move || worker(p)).is_err() {
        p.total.fetch_sub(1, Ordering::AcqRel);
        return false;
    }
    true
}

/// Run `job` on a worker thread.
pub fn submit(job: impl FnOnce() + Send + 'static) {
    let p = pool();
    let waiting = p.queued.fetch_add(1, Ordering::AcqRel) + 1;
    if waiting > p.idle.load(Ordering::Acquire) {
        add_worker(p); // at MAX the job simply waits for the next free worker
    }
    if p.tx.send(Box::new(job)).is_err() {
        p.queued.fetch_sub(1, Ordering::AcqRel);
    }
    if p.total.load(Ordering::Acquire) == 0 {
        add_worker(p); // every worker had timed out in the meantime
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;

    #[test]
    fn jobs_run_off_the_calling_thread() {
        let me = std::thread::current().id();
        let (tx, rx) = mpsc::channel();
        submit(move || tx.send(std::thread::current().id()).unwrap());
        assert_ne!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), me);
    }

    #[test]
    fn a_blocked_job_does_not_hold_up_the_next_one() {
        // the long job waits for the quick one: with a fixed single worker this would deadlock
        let gate = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let g = gate.clone();
        submit(move || {
            while !g.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(2));
            }
            tx.send("long").unwrap();
        });
        let (tx2, rx2) = mpsc::channel();
        submit(move || tx2.send("quick").unwrap());
        assert_eq!(rx2.recv_timeout(Duration::from_secs(5)).unwrap(), "quick");
        gate.store(true, Ordering::Release);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "long");
    }

    #[test]
    fn a_panicking_job_leaves_the_pool_working() {
        submit(|| panic!("boom"));
        let (tx, rx) = mpsc::channel();
        submit(move || tx.send(1).unwrap());
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
    }
}

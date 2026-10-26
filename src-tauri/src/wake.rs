// Interruptible nap for the generation-restarted workers (steam / notif / obs): one condvar wait instead of 250 ms sleep slices.
// A worker naps with `nap(&GEN, its_gen, ms)`; whoever restarts it calls `bump(&GEN)` and the nap ends at once.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

static M: Mutex<()> = Mutex::new(());
static CV: Condvar = Condvar::new();

/// New generation (old workers see it and end); wakes every napping worker.
pub fn bump(gen: &AtomicU64) -> u64 {
    let _g = M.lock().unwrap_or_else(|e| e.into_inner()); // the check in `nap` runs under this lock: no lost wake-up
    let n = gen.fetch_add(1, Ordering::SeqCst) + 1;
    CV.notify_all();
    n
}

/// Sleeps up to `ms`; false as soon as the generation is no longer `mine` (also on entry), true after the full time.
pub fn nap(gen: &AtomicU64, mine: u64, ms: u64) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    let mut g = M.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if gen.load(Ordering::SeqCst) != mine {
            return false;
        }
        let Some(left) = end.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) else { return true };
        g = CV.wait_timeout(g, left).unwrap_or_else(|e| e.into_inner()).0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nap_full_and_interrupted() {
        static G: AtomicU64 = AtomicU64::new(0);
        assert!(nap(&G, 0, 30)); // nothing changes: sleeps the whole time
        assert!(!nap(&G, 7, 5_000)); // already another generation: returns at once
        let t = Instant::now();
        let h = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            bump(&G)
        });
        assert!(!nap(&G, 0, 5_000));
        assert_eq!(h.join().unwrap(), 1);
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}

//! Deterministic replacement for the `setTimeout` calls of pill.js: named one-shot timers on the monotonic clock, fired from `tick`.
//! `set` replaces a pending timer of the same key (clear + set), `take_due` returns the earliest due key first (ties in arming order).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimerKey {
    /// hover intent (250 ms)
    Open,
    /// pointer left the expanded pill (380 ms)
    Leave,
    /// auto-hide idle delay (1500 ms)
    Away,
    /// auto-hide slide finished -> `pill_auto_hide`
    AwaySlide,
    /// next live-row render (minute / second tick, slot ttl)
    Live,
    /// battery poll (60 s)
    Battery,
    /// weather refresh (15 min)
    Weather,
    /// focus left a text field (300 ms)
    FocusOut,
    /// the OS frame did not reach the expanded size: start the spring anyway (55 ms)
    ExpFrame,
    /// quiet-hours sweep interval (20 s)
    Quiet,
}

#[derive(Debug, Clone, Default)]
pub struct Timers {
    items: Vec<(TimerKey, f64, u64)>,
    seq: u64,
}

impl Timers {
    pub fn new() -> Timers {
        Timers::default()
    }
    pub fn set(&mut self, key: TimerKey, at: f64) {
        self.clear(key);
        self.seq += 1;
        self.items.push((key, at, self.seq));
    }
    pub fn clear(&mut self, key: TimerKey) {
        self.items.retain(|(k, _, _)| *k != key);
    }
    pub fn has(&self, key: TimerKey) -> bool {
        self.items.iter().any(|(k, _, _)| *k == key)
    }
    pub fn deadline(&self, key: TimerKey) -> Option<f64> {
        self.items.iter().find(|(k, _, _)| *k == key).map(|(_, d, _)| *d)
    }
    pub fn next_deadline(&self) -> Option<f64> {
        self.items.iter().map(|(_, d, _)| *d).fold(None, |a: Option<f64>, d| Some(a.map_or(d, |x| x.min(d))))
    }
    pub fn clear_all(&mut self) {
        self.items.clear();
    }
    /// Remove and return the earliest timer due at `now`.
    pub fn take_due(&mut self, now: f64) -> Option<TimerKey> {
        let i = self.items.iter().enumerate().filter(|(_, (_, d, _))| *d <= now).min_by(|(_, a), (_, b)| a.1.partial_cmp(&b.1).unwrap().then(a.2.cmp(&b.2))).map(|(i, _)| i)?;
        Some(self.items.remove(i).0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_in_deadline_order_and_only_when_due() {
        let mut t = Timers::new();
        t.set(TimerKey::Leave, 380.0);
        t.set(TimerKey::Open, 250.0);
        assert_eq!(t.take_due(100.0), None);
        assert_eq!(t.take_due(400.0), Some(TimerKey::Open));
        assert_eq!(t.take_due(400.0), Some(TimerKey::Leave));
        assert_eq!(t.take_due(400.0), None);
    }

    #[test]
    fn set_replaces_and_clear_removes() {
        let mut t = Timers::new();
        t.set(TimerKey::Away, 100.0);
        t.set(TimerKey::Away, 900.0);
        assert_eq!(t.take_due(500.0), None);
        assert_eq!(t.deadline(TimerKey::Away), Some(900.0));
        t.clear(TimerKey::Away);
        assert!(!t.has(TimerKey::Away));
        assert_eq!(t.next_deadline(), None);
    }

    #[test]
    fn ties_keep_arming_order() {
        let mut t = Timers::new();
        t.set(TimerKey::Battery, 10.0);
        t.set(TimerKey::Weather, 10.0);
        assert_eq!(t.take_due(10.0), Some(TimerKey::Battery));
        assert_eq!(t.take_due(10.0), Some(TimerKey::Weather));
    }

    #[test]
    fn next_deadline_is_the_minimum() {
        let mut t = Timers::new();
        t.set(TimerKey::Live, 70.0);
        t.set(TimerKey::Quiet, 20.0);
        assert_eq!(t.next_deadline(), Some(20.0));
        t.clear_all();
        assert_eq!(t.next_deadline(), None);
    }
}

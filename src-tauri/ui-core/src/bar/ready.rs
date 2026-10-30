//! Handshake for the lazily created command bar window (bar-ready.js, ui/lazy.rs, command.rs): the window tells Rust (`cmd_ready`) once it has run
//! all its start-up code AND every event subscription (`bar:shown`, `bar:hidden`, `uninstall:state`, ...) is registered, because Rust shows the bar
//! and sends `bar:shown` only after that. The JS wraps `listen` to collect the registration promises; here the same barrier is a small machine:
//!
//!   1. every subscription is announced (`expect`) when the window asks for it and confirmed (`settled`) when the bus accepted it (a failed
//!      registration also counts as settled, like `Promise.allSettled`);
//!   2. when the window's start-up is done (`loaded`) and nothing is outstanding, one more round trip on the bus runs (`Step::Barrier`: subscribe to
//!      `bar:ready-barrier` and drop it again) because calls are handled in order, so everything before it has landed;
//!   3. then, exactly once, `cmd_ready` is sent (`Step::Report`).
//!
//! A `cmd_ready` that arrives after the bar is already on screen makes Rust replay `bar:shown`; the bar model treats a repeated `Shown` as a reset.

use super::protocol::BackendCall;
use std::collections::HashSet;

/// Event names the bar window subscribes to before it may report ready.
pub const BAR_EVENTS: [&str; 6] = ["bar:shown", "bar:hidden", "voice:state", "uninstall:state", "uninstall:item", "uninstall:focus"];

/// The name of the throw-away round-trip subscription.
pub const BARRIER_EVENT: &str = "bar:ready-barrier";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// subscribe to `BARRIER_EVENT` and unsubscribe at once (the window reports back with `barrier_done`)
    Barrier,
    /// send `cmd_ready`
    Report(BackendCall),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Collecting,
    BarrierSent,
    Reported,
}

#[derive(Clone, Debug)]
pub struct ReadyBarrier {
    pending: HashSet<String>,
    loaded: bool,
    phase: Phase,
}

impl Default for ReadyBarrier {
    fn default() -> Self {
        ReadyBarrier::new()
    }
}

impl ReadyBarrier {
    pub fn new() -> ReadyBarrier {
        ReadyBarrier { pending: HashSet::new(), loaded: false, phase: Phase::Collecting }
    }

    /// A subscription was requested.
    pub fn expect(&mut self, event: &str) {
        if self.phase == Phase::Collecting {
            self.pending.insert(event.to_string());
        }
    }
    /// The bus answered the subscription (accepted or not).
    pub fn settled(&mut self, event: &str) -> Option<Step> {
        self.pending.remove(event);
        self.advance()
    }
    /// The window's start-up code has run (`DOMContentLoaded`).
    pub fn loaded(&mut self) -> Option<Step> {
        self.loaded = true;
        self.advance()
    }
    /// The barrier round trip finished: report.
    pub fn barrier_done(&mut self) -> Option<Step> {
        if self.phase != Phase::BarrierSent {
            return None;
        }
        self.phase = Phase::Reported;
        Some(Step::Report(BackendCall::ready()))
    }
    pub fn reported(&self) -> bool {
        self.phase == Phase::Reported
    }
    pub fn outstanding(&self) -> usize {
        self.pending.len()
    }

    fn advance(&mut self) -> Option<Step> {
        if self.phase == Phase::Collecting && self.loaded && self.pending.is_empty() {
            self.phase = Phase::BarrierSent;
            return Some(Step::Barrier);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_only_after_every_subscription_settled_and_the_barrier_ran() {
        let mut b = ReadyBarrier::new();
        for e in BAR_EVENTS {
            b.expect(e);
        }
        assert_eq!(b.outstanding(), 6);
        assert_eq!(b.loaded(), None, "start-up is done but subscriptions are outstanding");
        for e in &BAR_EVENTS[..5] {
            assert_eq!(b.settled(e), None);
        }
        assert_eq!(b.settled(BAR_EVENTS[5]), Some(Step::Barrier));
        assert!(!b.reported());
        assert_eq!(b.barrier_done(), Some(Step::Report(BackendCall::ready())));
        assert!(b.reported());
        assert_eq!(b.barrier_done(), None, "exactly once");
    }

    #[test]
    fn start_up_finishing_last_also_works() {
        let mut b = ReadyBarrier::new();
        b.expect("bar:shown");
        assert_eq!(b.settled("bar:shown"), None, "the page has not finished loading");
        assert_eq!(b.loaded(), Some(Step::Barrier));
    }

    #[test]
    fn a_failed_subscription_still_counts_as_settled() {
        let mut b = ReadyBarrier::new();
        b.expect("voice:state");
        b.loaded();
        assert_eq!(b.settled("voice:state"), Some(Step::Barrier), "allSettled: the bar must not hang on a failed listener");
    }

    #[test]
    fn late_subscriptions_after_the_barrier_do_not_restart_it() {
        // nothing was announced: the barrier runs straight away
        let mut c = ReadyBarrier::new();
        assert_eq!(c.loaded(), Some(Step::Barrier));
        c.expect("late");
        assert_eq!(c.outstanding(), 0, "announcements after the barrier started are ignored");
        assert_eq!(c.barrier_done().map(|s| matches!(s, Step::Report(_))), Some(true));
    }

    #[test]
    fn the_report_is_the_cmd_ready_command() {
        let BackendCall { cmd, .. } = BackendCall::ready();
        assert_eq!(cmd, "cmd_ready");
        assert_eq!(BARRIER_EVENT, "bar:ready-barrier");
    }
}

//! Event cards (`Pill.toast`): queue, one card at a time INSIDE the island, ttl, replace-by-id, dismiss/out phase, the OS card frame.
//!
//! A card owns the live row until it is dismissed; the island grows to the card (the shell measures it) and the liquid edge stays the
//! pill's border. `toast` of an id that is queued/showing replaces it. `ttl_ms` defaults to 4000. Dismissing starts the out animation
//! (`CARD_OUT`, 180 ms; 0 under reduced motion), then the next card starts, or the live row returns and 600 ms later the taller OS frame
//! is given back (`pill_cards(false)`). The shell does the gating (hidden, quiet hours, away, expanded) and the side effects; this
//! module is the queue + timers (monotonic ms deadlines driven by `on_time`).

use std::collections::VecDeque;

/// Default card lifetime when `ttlMs` is missing.
pub const DEFAULT_TTL_MS: f64 = 4000.0;
/// Out animation duration (`dismiss`: `setTimeout(.., reduce() ? 0 : 180)`).
pub const OUT_MS: f64 = 180.0;
/// After the last card folded away the OS frame shrinks this much later.
pub const FRAME_OFF_MS: f64 = 600.0;
/// Hover must not reopen the pill for this long after a card closed (the cursor still rests where its button was).
pub const NO_OPEN_AFTER_DISMISS_MS: f64 = 900.0;
/// `o.fit(el, room)`: css px the OS card frame leaves under the island, minus 4 px spare: `floor(200 - 8 - 28) - 4`.
pub const FIT_ROOM_PX: f64 = 160.0;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CardAction {
    /// text button
    pub label: String,
    /// icon name (icon-only button when `label` is empty)
    pub icon: String,
    /// extra style class: `white`, ...
    pub kind: String,
    /// accessible name of an icon-only button
    pub title: String,
    /// returned in `Notice::CardAction` when pressed (the JS `run` callback)
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CardSpec {
    pub id: Option<String>,
    pub icon: String,
    /// rounded-square icon tile instead of a circle
    pub square: bool,
    pub title: String,
    pub sub: String,
    /// free message line (used when there is no title)
    pub html: String,
    /// `amber | green | red | accent`: the icon colour and the wavy-outline accent
    pub tone: String,
    pub cls: String,
    pub actions: Vec<CardAction>,
    pub ttl_ms: f64,
    pub urgent: bool,
    /// the card trims its line clamp to `FIT_ROOM_PX` (renderer's job)
    pub fit: bool,
}

/// Animation phase of the card on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CardPhase {
    /// entering since `since_ms` (play `motion::CARD_IN`)
    In { since_ms: f64 },
    /// folding away since `since_ms` (play `motion::CARD_OUT`)
    Out { since_ms: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CardView {
    pub spec: CardSpec,
    pub phase: CardPhase,
}

#[derive(Debug, Clone)]
struct Cur {
    spec: CardSpec,
    started: f64,
    ttl_deadline: Option<f64>,
    out_at: Option<f64>,
    out_deadline: Option<f64>,
}

/// What happened while advancing the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardEvent {
    /// the ttl ran out: the card started folding (`noOpenUntil` should be set)
    Expired,
    /// the out animation finished and the card was removed: the shell starts the next card / restores the live row
    Ended,
    /// the OS card frame can shrink (`pill_cards(false)`)
    FrameOff,
}

#[derive(Debug, Clone, Default)]
pub struct Cards {
    queue: VecDeque<CardSpec>,
    cur: Option<Cur>,
    frame_off_deadline: Option<f64>,
}

impl Cards {
    pub fn new() -> Cards {
        Cards::default()
    }
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }
    pub fn has_current(&self) -> bool {
        self.cur.is_some()
    }
    pub fn current_id(&self) -> Option<&str> {
        self.cur.as_ref().and_then(|c| c.spec.id.as_deref())
    }
    pub fn current_urgent(&self) -> bool {
        self.cur.as_ref().is_some_and(|c| c.spec.urgent)
    }
    pub fn current_spec(&self) -> Option<&CardSpec> {
        self.cur.as_ref().map(|c| &c.spec)
    }
    pub fn queued_ids(&self) -> Vec<Option<&str>> {
        self.queue.iter().map(|c| c.id.as_deref()).collect()
    }
    /// `curCard.out`
    pub fn is_out(&self) -> bool {
        self.cur.as_ref().is_some_and(|c| c.out_at.is_some())
    }
    pub fn view(&self) -> Option<CardView> {
        self.cur.as_ref().map(|c| CardView {
            spec: c.spec.clone(),
            phase: match c.out_at {
                Some(t) => CardPhase::Out { since_ms: t },
                None => CardPhase::In { since_ms: c.started },
            },
        })
    }
    pub fn next_deadline(&self) -> Option<f64> {
        let c = self.cur.as_ref();
        [c.and_then(|c| c.ttl_deadline), c.and_then(|c| c.out_deadline), self.frame_off_deadline].into_iter().flatten().fold(None, |a: Option<f64>, d| Some(a.map_or(d, |x| x.min(d))))
    }

    /// The queueing half of `toast`: replace an equal id (queued and showing), then enqueue. Returns true when the SHOWING card was
    /// replaced (it is removed at once; the shell then calls `next`).
    pub fn enqueue(&mut self, spec: CardSpec) -> bool {
        let mut replaced = false;
        if let Some(id) = spec.id.as_deref() {
            self.queue.retain(|c| c.id.as_deref() != Some(id));
            if self.current_id() == Some(id) {
                self.cur = None;
                replaced = true;
            }
        }
        self.queue.push_back(spec);
        replaced
    }

    /// `next()`: start the next queued card when allowed (`can_show` = not away, not hidden, not expanded) and none is up.
    pub fn next(&mut self, can_show: bool, now: f64) -> bool {
        if self.cur.is_some() || !can_show {
            return false;
        }
        let Some(spec) = self.queue.pop_front() else { return false };
        self.frame_off_deadline = None;
        let ttl = if spec.ttl_ms != 0.0 { spec.ttl_ms } else { DEFAULT_TTL_MS };
        self.cur = Some(Cur { spec, started: now, ttl_deadline: Some(now + ttl), out_at: None, out_deadline: None });
        true
    }

    /// `dismiss()`: start the out phase. False when there is nothing to dismiss or it is already folding.
    pub fn dismiss(&mut self, now: f64, reduce: bool) -> bool {
        let Some(c) = self.cur.as_mut() else { return false };
        if c.out_at.is_some() {
            return false;
        }
        c.ttl_deadline = None;
        c.out_at = Some(now);
        c.out_deadline = Some(now + if reduce { 0.0 } else { OUT_MS });
        true
    }

    /// `endCard(curCard)`: remove the showing card immediately (hide, quiet sweep, replace).
    pub fn end_current(&mut self) -> bool {
        self.cur.take().is_some()
    }

    /// `dropCards()` queue part.
    pub fn clear_queue(&mut self) {
        self.queue.clear();
    }

    /// Quiet hours: drop queued cards that are not exempt; returns how many were dropped.
    pub fn drop_queued_where(&mut self, f: impl Fn(&CardSpec) -> bool) -> usize {
        let n = self.queue.len();
        self.queue.retain(|c| !f(c));
        n - self.queue.len()
    }

    /// Arm the OS-frame shrink (600 ms after the island folded back).
    pub fn arm_frame_off(&mut self, now: f64) {
        self.frame_off_deadline = Some(now + FRAME_OFF_MS);
    }
    pub fn cancel_frame_off(&mut self) {
        self.frame_off_deadline = None;
    }

    /// Fire everything due at `now`. `reduce`: motion is reduced (out phase of 0 ms).
    pub fn on_time(&mut self, now: f64, reduce: bool) -> Vec<CardEvent> {
        let mut out = Vec::new();
        loop {
            let ttl_due = self.cur.as_ref().and_then(|c| c.ttl_deadline).is_some_and(|d| d <= now);
            if ttl_due {
                if self.dismiss(now, reduce) {
                    out.push(CardEvent::Expired);
                }
                continue;
            }
            let out_due = self.cur.as_ref().and_then(|c| c.out_deadline).is_some_and(|d| d <= now);
            if out_due {
                self.cur = None;
                out.push(CardEvent::Ended);
                // the shell reacts (next / live row) and may start a new card; stop here so it can
                break;
            }
            break;
        }
        if self.frame_off_deadline.is_some_and(|d| d <= now) {
            self.frame_off_deadline = None;
            if self.cur.is_none() {
                out.push(CardEvent::FrameOff);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str) -> CardSpec {
        CardSpec { id: Some(id.into()), title: format!("t-{id}"), ..Default::default() }
    }

    #[test]
    fn one_at_a_time_in_order() {
        let mut c = Cards::new();
        c.enqueue(card("a"));
        c.enqueue(card("b"));
        assert!(c.next(true, 0.0));
        assert_eq!(c.current_id(), Some("a"));
        assert!(!c.next(true, 1.0)); // one is up
        assert_eq!(c.queue_len(), 1);
        assert!(c.dismiss(10.0, false));
        assert_eq!(c.on_time(190.0, false), vec![CardEvent::Ended]);
        assert!(c.next(true, 190.0));
        assert_eq!(c.current_id(), Some("b"));
    }

    #[test]
    fn next_respects_gating() {
        let mut c = Cards::new();
        c.enqueue(card("a"));
        assert!(!c.next(false, 0.0)); // away / hidden / expanded
        assert!(c.next(true, 0.0));
        assert!(!c.next(true, 0.0)); // empty queue + one showing
    }

    #[test]
    fn default_ttl_is_four_seconds_and_custom_ttl_is_used() {
        let mut c = Cards::new();
        c.enqueue(card("a"));
        c.next(true, 100.0);
        assert_eq!(c.next_deadline(), Some(4100.0));
        let mut c = Cards::new();
        c.enqueue(CardSpec { ttl_ms: 1500.0, ..card("a") });
        c.next(true, 100.0);
        assert_eq!(c.next_deadline(), Some(1600.0));
    }

    #[test]
    fn ttl_expiry_dismisses_then_removes_after_the_out_animation() {
        let mut c = Cards::new();
        c.enqueue(card("a"));
        c.next(true, 0.0);
        assert_eq!(c.on_time(3999.0, false), vec![]);
        assert_eq!(c.on_time(4000.0, false), vec![CardEvent::Expired]);
        assert!(c.is_out());
        assert!(matches!(c.view().unwrap().phase, CardPhase::Out { since_ms } if since_ms == 4000.0));
        assert_eq!(c.on_time(4179.0, false), vec![]);
        assert_eq!(c.on_time(4180.0, false), vec![CardEvent::Ended]);
        assert!(!c.has_current());
    }

    #[test]
    fn reduced_motion_removes_in_the_same_pass() {
        let mut c = Cards::new();
        c.enqueue(card("a"));
        c.next(true, 0.0);
        assert!(c.dismiss(50.0, true));
        assert_eq!(c.on_time(50.0, true), vec![CardEvent::Ended]);
        // ttl expiry under reduced motion: both in one call
        let mut c = Cards::new();
        c.enqueue(card("a"));
        c.next(true, 0.0);
        assert_eq!(c.on_time(4000.0, true), vec![CardEvent::Expired, CardEvent::Ended]);
    }

    #[test]
    fn dismiss_is_idempotent() {
        let mut c = Cards::new();
        assert!(!c.dismiss(0.0, false)); // nothing showing
        c.enqueue(card("a"));
        c.next(true, 0.0);
        assert!(c.dismiss(100.0, false));
        assert!(!c.dismiss(120.0, false)); // already out: the first out deadline stands
        assert_eq!(c.next_deadline(), Some(280.0));
    }

    #[test]
    fn same_id_replaces_queued_and_showing() {
        let mut c = Cards::new();
        c.enqueue(card("a"));
        c.enqueue(card("b"));
        c.enqueue(CardSpec { title: "newer".into(), ..card("b") });
        assert_eq!(c.queue_len(), 2);
        assert_eq!(c.queued_ids(), vec![Some("a"), Some("b")]);
        c.next(true, 0.0); // a shows
        let replaced = c.enqueue(CardSpec { title: "a2".into(), ..card("a") });
        assert!(replaced);
        assert!(!c.has_current());
        assert!(c.next(true, 5.0));
        assert_eq!(c.current_id(), Some("b")); // queue order: b (older slot dropped, newest b stays before a2)
        assert_eq!(c.current_spec().unwrap().title, "newer");
    }

    #[test]
    fn id_less_cards_never_replace() {
        let mut c = Cards::new();
        c.enqueue(CardSpec { title: "x".into(), ..Default::default() });
        c.enqueue(CardSpec { title: "y".into(), ..Default::default() });
        assert_eq!(c.queue_len(), 2);
    }

    #[test]
    fn urgent_flag_and_frame_off() {
        let mut c = Cards::new();
        c.enqueue(CardSpec { urgent: true, ..card("u") });
        c.next(true, 0.0);
        assert!(c.current_urgent());
        c.dismiss(0.0, false);
        assert_eq!(c.on_time(180.0, false), vec![CardEvent::Ended]);
        c.arm_frame_off(180.0);
        assert_eq!(c.on_time(779.0, false), vec![]);
        assert_eq!(c.on_time(780.0, false), vec![CardEvent::FrameOff]);
    }

    #[test]
    fn frame_off_is_skipped_when_a_new_card_is_up() {
        let mut c = Cards::new();
        c.arm_frame_off(0.0);
        c.enqueue(card("n"));
        assert!(c.next(true, 100.0)); // a new card cancels the pending shrink
        assert_eq!(c.on_time(700.0, false), vec![]);
        let mut c = Cards::new();
        c.arm_frame_off(0.0);
        c.cur = Some(Cur { spec: card("z"), started: 0.0, ttl_deadline: None, out_at: None, out_deadline: None });
        assert_eq!(c.on_time(700.0, false), vec![]); // `if (!curCard)` guard
    }

    #[test]
    fn quiet_sweep_drops_non_exempt_queued_cards() {
        let mut c = Cards::new();
        c.enqueue(card("copy"));
        c.enqueue(CardSpec { urgent: true, ..card("x") });
        c.enqueue(card("timer-1"));
        let n = c.drop_queued_where(|s| !crate::shell::quiet::exempt_toast(s.id.as_deref(), s.urgent));
        assert_eq!(n, 1);
        assert_eq!(c.queued_ids(), vec![Some("x"), Some("timer-1")]);
    }

    #[test]
    fn end_current_and_clear_queue() {
        let mut c = Cards::new();
        c.enqueue(card("a"));
        c.enqueue(card("b"));
        c.next(true, 0.0);
        assert!(c.end_current());
        assert!(!c.end_current());
        c.clear_queue();
        assert_eq!(c.queue_len(), 0);
        assert!(c.view().is_none());
    }

    #[test]
    fn view_phase_in_then_out() {
        let mut c = Cards::new();
        c.enqueue(card("a"));
        c.next(true, 42.0);
        assert!(matches!(c.view().unwrap().phase, CardPhase::In { since_ms } if since_ms == 42.0));
    }
}

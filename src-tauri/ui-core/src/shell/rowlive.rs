//! The collapsed live row of the shell: slot selection (`renderLive`), the clock row (`renderClock`), event cards (`toast`/`next`/`dismiss`),
//! weather and battery polling. Extends `Shell` from capsule.rs.
//!
//! Row precedence: an event card owns the row until dismissed; else the highest-priority `Pill.setLive` slot; else the clock
//! `[weather icon] time date [battery] temp`. A different owner crossfades (the old content ghosts out ~90 ms while the new blurs in
//! over 130 ms, see `motion::LIVE_SWAP` / `LIVE_GHOST_OUT`); under reduced motion nothing animates.

use serde_json::{json, Value};

use crate::shell::cards::{CardSpec, CardView};
use crate::shell::clock::{build_row, next_clock_render_ms, time_text, Battery, ClockInputs, ClockSlots, ClockUpdate, Weather};
use crate::shell::io::{Backend, Clock};
use crate::shell::lives::{LiveInput, LiveSlot};
use crate::shell::quiet::{exempt_toast, is_quiet_source};
use crate::shell::spring::Mode;
use crate::shell::timers::TimerKey;
use crate::shell::types::LiveContent;

use super::capsule::{PillState, Shell, BATTERY_POLL_MS, WEATHER_MS};

impl<B: Backend, C: Clock> Shell<B, C> {
    // ---------- ownership of the row ----------
    /// `invalidateLive()`: the next render rebuilds the row.
    pub(super) fn invalidate_live(&mut self) {
        self.live_key.clear();
        self.clock_state.invalidate();
    }

    pub(super) fn current_content(&self) -> LiveContent {
        if self.live_key == "card" {
            return self.cards.view().map(LiveContent::Card).unwrap_or(LiveContent::None);
        }
        self.content.clone()
    }

    /// `liveGhost()`: keep the outgoing content for ~90 ms.
    fn live_ghost(&mut self) {
        if self.reduce() || self.hidden || self.env.occluded {
            return;
        }
        let c = self.current_content();
        if c != LiveContent::None {
            self.ghost = Some((c, self.now));
        }
    }
    /// `swapFx()`: play the incoming animation (not under reduced motion).
    fn swap_fx(&mut self) {
        if !self.reduce() {
            self.swap_epoch += 1;
            self.swap_at = self.now;
        }
    }

    /// `renderLive()`
    pub(super) fn render_live(&mut self) {
        self.timers.clear(TimerKey::Live);
        if self.cards.has_current() {
            // an event card owns the live row until it is dismissed
            if self.live_key != "card" || self.shown_card != self.card_gen {
                self.live_key = "card".into();
                self.clock_state.reset();
                self.live_ghost();
                self.shown_card = self.card_gen;
                self.root_tall = true;
                self.swap_fx();
            }
            let tone = self.cards.current_spec().map(|c| c.tone.clone()).unwrap_or_default();
            self.wave_c = if self.palette().tone(&tone).is_some() { format!("var(--{tone})") } else { String::new() };
            self.liq_colors();
            return;
        }
        if self.live_key == "card" {
            self.live_key.clear();
            self.root_tall = false;
            self.content = LiveContent::None;
        }
        let sel = self.lives.select(self.wall);
        let want_tall = sel.best.as_ref().is_some_and(|b| b.tall);
        self.root_tall = want_tall; // 'tall' = natural size, measured by colTarget
        if want_tall != self.live_tall {
            self.live_tall = want_tall;
        }
        match &sel.best {
            Some(b) => {
                let key = b.render_key();
                if key != self.live_key {
                    self.live_key = key;
                    self.clock_state.reset();
                    self.live_ghost();
                    self.content = LiveContent::Slot { id: b.id.clone(), key: b.key.clone(), spec: b.spec.clone() };
                    self.swap_fx();
                }
            }
            None => self.render_clock(),
        }
        let accent = sel.best.as_ref().map(|b| b.accent.clone()).unwrap_or_default();
        self.wave_c = accent.clone();
        self.liq_colors();
        self.liq_playing(!accent.is_empty());
        if self.hidden {
            return;
        }
        // clock: once a minute (once a second with seconds); slots with a ttl re-render when they expire
        let mut next = if sel.best.is_some() {
            f64::INFINITY
        } else {
            next_clock_render_ms(self.wall, self.s.show_seconds) as f64
        };
        if let Some(u) = sel.next_until {
            next = next.min((u - self.wall) as f64 + 5.0);
        }
        if next.is_finite() {
            self.timers.set(TimerKey::Live, self.now + next.max(50.0));
        }
    }

    /// `renderClock()`
    pub(super) fn render_clock(&mut self) {
        let local = self.local();
        let tt = time_text(&local, self.s.clock24, self.s.show_seconds);
        let row = build_row(&ClockInputs {
            settings: &self.s,
            lang: self.lang,
            local,
            wx: self.wx.as_ref(),
            battery: self.battery.as_ref(),
            slots: &self.slots,
            char_side: self.char_side,
            char_legs: self.char_legs,
        });
        let live_is_clock = self.live_key == "clock";
        let animate = !self.s.show_seconds && !self.reduce() && !self.hidden && !self.env.occluded;
        match self.clock_state.render(live_is_clock, row.clone(), &tt, animate) {
            ClockUpdate::Full { swap, reshaped, entering } => {
                self.live_key = "clock".into();
                if swap {
                    self.live_ghost();
                }
                self.content = LiveContent::Clock { time: if row.show_time { Some(tt) } else { None }, row };
                if swap {
                    self.swap_fx();
                }
                self.rolls.clear();
                self.entering.clear();
                if reshaped && !self.reduce() {
                    let now = self.now;
                    self.entering = entering.into_iter().map(|(r, i)| (r, i, now)).collect();
                }
                self.notices.push(crate::shell::types::Notice::LiveChanged); // characters.js re-measures where the eyes stand
            }
            ClockUpdate::Time { rolls } => {
                if let LiveContent::Clock { time, .. } = &mut self.content {
                    *time = Some(tt);
                }
                let now = self.now;
                self.rolls = rolls.into_iter().map(|r| (r, now)).collect();
            }
            ClockUpdate::Unchanged => {}
        }
    }

    // ---------- public live API ----------
    /// `Pill.setLive(id, w)`; `None` removes the slot.
    pub fn set_live(&mut self, id: &str, w: Option<LiveInput>) {
        let w = match w {
            Some(x) if is_quiet_source(id) && !x.urgent && self.quiet() => None, // quiet hours: event pulses and chips are not shown
            other => other,
        };
        match w {
            None => {
                self.lives.delete(id);
            }
            Some(x) => {
                self.lives.set(LiveSlot::from_input(id, &x, self.wall));
                self.reveal();
            }
        }
        if !self.hidden {
            self.render_live();
        }
    }
    /// `Pill.setClockSlots({ left, right })`
    pub fn set_clock_slots(&mut self, slots: ClockSlots) {
        self.slots = slots;
        self.invalidate_live();
        self.render_live();
    }
    /// `Pill.relayoutLive()`: the row's CSS changed (the legs' eyes keep a slot): rebuild and resize.
    pub fn relayout_live(&mut self) {
        if self.hidden || self.cards.has_current() {
            return;
        }
        if self.live_key == "clock" {
            self.clock_state.invalidate();
            self.render_clock();
        }
        if self.state == PillState::Collapsed {
            self.retarget(false, None, Mode::Resize);
        }
    }

    // ---------- cards ----------
    /// `Pill.toast(o)`
    pub fn toast(&mut self, o: CardSpec) {
        if self.hidden {
            return;
        }
        if self.quiet() && !exempt_toast(o.id.as_deref(), o.urgent) {
            return; // quiet hours: dropped, never shown later
        }
        self.cancel_away();
        self.cards.enqueue(o);
        self.reveal();
        self.next();
    }
    /// `next()`: the island itself grows to the card. The OS frame is taller while a card is up.
    pub(super) fn next(&mut self) {
        let can_show = !self.away && !self.hidden && self.state != PillState::Expanded;
        if self.cards.next(can_show, self.now) {
            self.invoke("pill_cards", json!({"on": true})); // pill.rs CARD_FRAME
            self.carding = true;
            self.card_gen += 1;
            self.render_live();
        }
    }
    pub(super) fn dismiss(&mut self) {
        if self.cards.dismiss(self.now, self.reduce()) {
            self.no_open_until = self.now + crate::shell::cards::NO_OPEN_AFTER_DISMISS_MS;
        }
    }
    /// `dropCards()` (pill:hide)
    pub(super) fn drop_cards(&mut self) {
        self.cards.clear_queue();
        self.carding = false;
        self.invoke("pill_cards", json!({"on": false}));
        self.cards.cancel_frame_off();
        if self.cards.has_current() {
            self.cards.end_current();
            self.render_live();
        }
    }
    /// The card on screen, for the renderer.
    pub fn card_view(&self) -> Option<CardView> {
        self.cards.view()
    }

    // ---------- battery ----------
    pub(super) fn poll_battery(&mut self) {
        self.timers.clear(TimerKey::Battery);
        if self.hidden {
            return;
        }
        if self.s.show_battery {
            self.invoke("battery_state", json!({}));
        }
        self.timers.set(TimerKey::Battery, self.now + BATTERY_POLL_MS);
    }
    pub(super) fn on_battery(&mut self, v: &Value) {
        let b = Battery::from_json(v);
        if b.is_some() && b != self.battery {
            self.battery = b;
            self.invalidate_live();
            self.render_live();
        }
    }

    // ---------- weather ----------
    /// event `weather:current` { temp, unit, code, icon, text } or `weather_get().now`; nothing is shown when weather is off / unknown
    pub(super) fn set_wx(&mut self, w: Option<Weather>) {
        self.wx = w;
        self.invalidate_live();
        self.render_live();
    }
    pub(super) fn refresh_weather(&mut self) {
        self.timers.clear(TimerKey::Weather);
        let wanted = self.s.wx_wanted();
        self.wx_net = Some(wanted);
        if !wanted {
            if self.wx.is_some() {
                self.set_wx(None);
            }
            return;
        }
        if self.hidden {
            return; // nobody sees the capsule: no fetch while the island is hidden, pill:show comes back through wx_on_show()
        }
        let tz = self.clock.utc_offset_min();
        self.invoke("weather_get", json!({"force": false, "tz": tz}));
    }
    /// answer of `weather_get`: `{ status: 'ok', now: {..} }` | `off` | `nocity`
    pub(super) fn on_weather_get(&mut self, r: &Value) {
        match r.get("status").and_then(|s| s.as_str()) {
            Some("ok") if r.get("now").is_some_and(|n| !n.is_null()) => {
                self.set_wx(Weather::from_json(&r["now"]));
                self.wx_at = Some(self.wall);
            }
            Some("off") | Some("nocity") => self.set_wx(None),
            _ => {}
        }
        if !self.hidden {
            self.timers.set(TimerKey::Weather, self.now + WEATHER_MS);
        }
    }
    /// pill:show: one refresh if the forecast is older than 15 min, else just re-arm the timer for the rest of it
    pub(super) fn wx_on_show(&mut self) {
        self.timers.clear(TimerKey::Weather);
        match self.wx_at {
            Some(at) if self.s.wx_wanted() && self.wall - at < WEATHER_MS as i64 => {
                let rest = (WEATHER_MS - (self.wall - at) as f64).max(1000.0);
                self.timers.set(TimerKey::Weather, self.now + rest);
            }
            _ => self.refresh_weather(),
        }
    }
}

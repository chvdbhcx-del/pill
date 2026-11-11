//! The motion of the paired Phone view: every number the phone reports as a [`Num`] spring (percent, bars, GB, counters, signal), the chips and
//! optional parts as 0..1 [`Num`]s. Targets are set in `Card::sync` (called from `step`), `draw` (`phone_dash.rs`) only reads. Unknown values are
//! simply not drawn; offline, the last known values stay. Nothing polls: the "connected for" text is recomputed when the view is painted for
//! another reason, like the notification times.
use super::phone::PhoneView;
use crate::ui::liquid::{Feel, Flow};
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::panel::b::Lang;
use pill_ui_core::panel::c::phone::Device;
use pill_ui_core::panel::c::phone_info::*;

const GAP: f32 = 6.0;
const GIB: f32 = 1_073_741_824.0;
const MIB: f32 = 1_048_576.0;

/// One animated number. The first value lands at once (an opening tab is not news); reduced motion lands every value at once.
pub(super) struct Num {
    f: Flow,
    seen: bool,
}

impl Num {
    fn new(px: bool) -> Num {
        Num { f: if px { Flow::px(0.0, Feel::DRAG) } else { Flow::new(0.0, Feel::DRAG) }, seen: false }
    }
    /// A 0..1 switch (a chip, a tile that appears) with its own feel.
    fn lit(feel: Feel) -> Num {
        Num { f: Flow::new(0.0, feel), seen: false }
    }
    fn on(&mut self, on: bool, reduced: bool) {
        self.set(on as u8 as f32, reduced);
    }
    pub(super) fn at01(&self) -> f32 {
        self.f.at01()
    }
    fn set(&mut self, v: f32, reduced: bool) {
        if !self.seen || reduced {
            self.seen = true;
            self.f.jump(v);
        }
        self.f.to(v);
    }
    pub(super) fn at(&self) -> f32 {
        self.f.at()
    }
    pub(super) fn target(&self) -> f32 {
        self.f.target()
    }
    fn step(&mut self, dt_ms: f64) -> bool {
        self.f.step(dt_ms);
        self.f.moving()
    }
}

/// All the motion of the card.
pub(super) struct Card {
    /// battery temperature, deg C
    pub temp: Num,
    /// used fractions of storage and RAM, free amounts in GiB
    pub st_used: Num,
    pub st_free: Num,
    pub ram_used: Num,
    pub ram_free: Num,
    /// signal level 0..4, link speed Mbps
    pub sig: Num,
    pub mbps: Num,
    /// counters; bytes in MiB
    pub n_notifs: Num,
    pub n_replies: Num,
    pub n_in: Num,
    pub n_out: Num,
    pub b_in: Num,
    pub b_out: Num,
    /// mode chips
    pub dnd: Num,
    pub air: Num,
    pub ringer: Num,
    pub screen: Num,
    pub screen_on: Num,
    /// parts that exist only when the phone told: the storage row, the temperature line, the signal bars; and the bolt
    pub store_k: Num,
    pub detail_k: Num,
    pub bars_k: Num,
    pub charge_k: Num,
}

impl Card {
    pub(super) fn new() -> Card {
        let n = || Num::new(true);
        Card {
            temp: n(),
            st_used: Num::new(false),
            st_free: n(),
            ram_used: Num::new(false),
            ram_free: n(),
            sig: Num::new(false),
            mbps: n(),
            n_notifs: n(),
            n_replies: n(),
            n_in: n(),
            n_out: n(),
            b_in: n(),
            b_out: n(),
            dnd: Num::lit(Feel::POP),
            air: Num::lit(Feel::POP),
            ringer: Num::lit(Feel::POP),
            screen: Num::lit(Feel::POP),
            screen_on: Num::lit(Feel::FADE),
            store_k: Num::lit(Feel::FLOW),
            detail_k: Num::lit(Feel::FLOW),
            bars_k: Num::lit(Feel::FADE),
            charge_k: Num::lit(Feel::FADE),
        }
    }

    /// Targets from the device. Called from `step` only. A value the phone did not send keeps its last target.
    pub(super) fn sync(&mut self, d: &Device, reduced: bool) {
        let i = &d.info;
        if let Some(t) = i.temp_c {
            self.temp.set(t, reduced);
        }
        if let (Some(u), Some((f, _))) = (i.storage_used(), i.storage) {
            self.st_used.set(u, reduced);
            self.st_free.set(f as f32 / GIB, reduced);
        }
        if let (Some(u), Some((f, _))) = (i.ram_used(), i.ram) {
            self.ram_used.set(u, reduced);
            self.ram_free.set(f as f32 * MIB / GIB, reduced);
        }
        let net = net_view(&d.network, d.ssid.as_deref(), i, Lang::En);
        if let Some(b) = net.bars {
            self.sig.set(b as f32, reduced);
        }
        if let Some(m) = i.mbps {
            self.mbps.set(m as f32, reduced);
        }
        let s = &d.stats;
        for (n, v) in [(&mut self.n_notifs, s.notifs), (&mut self.n_replies, s.replies), (&mut self.n_in, s.files_in), (&mut self.n_out, s.files_out)] {
            n.set(v as f32, reduced);
        }
        self.b_in.set(s.bytes_in as f32 / MIB, reduced);
        self.b_out.set(s.bytes_out as f32 / MIB, reduced);
        self.dnd.on(d.dnd, reduced);
        self.air.on(i.airplane == Some(true), reduced);
        self.ringer.on(matches!(i.ringer, Some(Ringer::Vibrate | Ringer::Silent)), reduced);
        self.screen.on(i.screen.is_some(), reduced);
        self.screen_on.on(i.screen == Some(true), reduced);
        self.store_k.on(i.storage.is_some() || i.ram.is_some(), reduced);
        self.detail_k.on(battery_detail(i, Lang::En).is_some(), reduced);
        self.bars_k.on(net.bars.is_some(), reduced);
        self.charge_k.on(
            match i.charge {
                Some(c) => c == Charge::Charging,
                None => d.charging,
            },
            reduced,
        );
    }

    /// Advances every spring; true while one is still moving.
    pub(super) fn step(&mut self, dt_ms: f64) -> bool {
        let mut busy = false;
        for n in [
            &mut self.temp,
            &mut self.st_used,
            &mut self.st_free,
            &mut self.ram_used,
            &mut self.ram_free,
            &mut self.sig,
            &mut self.mbps,
            &mut self.n_notifs,
            &mut self.n_replies,
            &mut self.n_in,
            &mut self.n_out,
            &mut self.b_in,
            &mut self.b_out,
            &mut self.dnd,
            &mut self.air,
            &mut self.ringer,
            &mut self.screen,
            &mut self.screen_on,
            &mut self.store_k,
            &mut self.detail_k,
            &mut self.bars_k,
            &mut self.charge_k,
        ] {
            busy |= n.step(dt_ms);
        }
        busy
    }
}

pub(super) fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A usage bar colour that slides from the accent to amber to red as the bar fills.
pub(super) fn usage_color(th: &Theme, v: f32) -> Color {
    th.accent.mix(th.amber, smooth(0.78, 0.9, v)).mix(th.red, smooth(0.93, 0.98, v))
}

/// Flow-wraps chips of the given (width, appearance) into rows of `avail` px; returns each chip's (x, y) and the total height.
/// A chip of width 0 takes no room.
pub(super) fn wrap(items: &[(f32, f32)], avail: f32, row_h: f32) -> (Vec<(f32, f32)>, f32) {
    let (mut x, mut y, mut any) = (0.0f32, 0.0f32, false);
    let mut out = Vec::new();
    for &(w, k) in items {
        if w < 0.5 {
            out.push((x, y));
            continue;
        }
        if x > 0.0 && x + w > avail {
            x = 0.0;
            y += row_h + 5.0;
        }
        any = true;
        out.push((x, y));
        x += w + GAP * k;
    }
    (out, if any { y + row_h } else { 0.0 })
}

pub(super) fn city(tz: &str) -> String {
    tz.rsplit('/').next().unwrap_or(tz).replace('_', " ")
}

#[cfg(windows)]
fn local_tz_min() -> i64 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    let dst = if id == 2 { tz.DaylightBias } else { tz.StandardBias };
    -(tz.Bias + dst) as i64
}
#[cfg(not(windows))]
fn local_tz_min() -> i64 {
    0
}

impl PhoneView {
    /// The tz offset used for dates: the PC's, 0 when the wall clock is pinned (tests).
    pub(super) fn tz_min(&self) -> i64 {
        if self.wall_pinned() {
            0
        } else {
            local_tz_min()
        }
    }
}

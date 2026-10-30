//! System monitor in the pill (port of sysmon-live.js; Rust side sysmon.rs): CPU / RAM / GPU meters on top of
//! the Controls tab plus a warning chip in the collapsed island.
//!
//! * `sysmon:tick {cpu, ram:{usedMb,totalMb}, gpus:[{index,name,util,tempC,memUsedMb,memTotalMb,powerW}]}` every 2 s while the
//!   switch is on and the pill is visible.
//! * meters: [`MetersView`] (the Controls tab draws it; `LiveCmd::SysMeters` carries it on every sample and settings change).
//! * chip: GPU >= 85 C or CPU >= 95 % held for 10 s -> row `sysmon` (priority 90) for 8 s, then a 5 minute cool-down per source;
//!   never fires while the island is expanded.
//!
//! Switch: Settings > Live events > System monitor (`live.sysmon`). The JS read the setting itself: only an explicit `false`
//! disables it (a missing key counts as on), and so does this port. PRIVACY: only load counters, nothing is stored or sent.

use std::any::Any;

use serde_json::Value;

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

pub const GPU_C: f64 = 85.0;
pub const CPU_PCT: f64 = 95.0;
pub const RAM_PCT: f64 = 90.0;
pub const HOLD_MS: i64 = 10_000;
pub const COOL_MS: i64 = 5 * 60_000;
pub const CHIP_MS: u32 = 8000;
pub const MAX_GPU: usize = 2;
/// The sample period of the Rust side; a gap of more than three of them breaks a hot streak.
pub const SAMPLE_MS: i64 = 2000;
pub const CHIP_PRIORITY: i32 = 90;

/// One meter of the Controls tab.
#[derive(Clone, Debug, PartialEq)]
pub struct MeterRow {
    /// `cpu`, `ram`, `g0`, `g1`
    pub key: String,
    pub label: String,
    pub value: String,
    /// Bar fill 0..100.
    pub pct: f32,
    /// Draw in the warning colour.
    pub hot: bool,
    pub tip: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetersView {
    pub visible: bool,
    pub rows: Vec<MeterRow>,
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|x| x.is_finite())
}

fn gb(mb: f64) -> String {
    to_fixed(mb / 1024.0, 1)
}

fn rnd(x: f64) -> String {
    fmt_num(js_round(x))
}

pub fn enabled(env: &dyn Env) -> bool {
    env.setting("live.sysmon") != Some(Value::Bool(false))
}

#[derive(Debug, Default)]
pub struct SysmonLive {
    d: Option<Value>,
    hot_since: Vec<(String, i64)>,
    shown_at: Vec<(String, i64)>,
    last_tick: i64,
}

impl SysmonLive {
    /// The meters for the last sample (`draw`).
    pub fn view(&self, env: &dyn Env) -> MetersView {
        if !enabled(env) {
            return MetersView { visible: false, rows: Vec::new() };
        }
        let d = self.d.as_ref();
        let mut rows = Vec::new();
        let cpu = d.and_then(|d| num(d.get("cpu")));
        rows.push(MeterRow {
            key: "cpu".into(),
            label: "CPU".into(),
            value: cpu.map_or("–".to_string(), |c| format!("{}%", rnd(c))),
            pct: cpu.unwrap_or(0.0).clamp(0.0, 100.0) as f32,
            hot: cpu.is_some_and(|c| c >= CPU_PCT),
            tip: env.tr("Процессор", "Processor"),
        });
        let ram = d.and_then(|d| d.get("ram"));
        let (used, total) = (ram.map_or(f64::NAN, |r| to_number(r.get("usedMb"))), ram.map_or(f64::NAN, |r| to_number(r.get("totalMb"))));
        let rp = if total > 0.0 { Some(used / total * 100.0) } else { None };
        rows.push(MeterRow {
            key: "ram".into(),
            label: "RAM".into(),
            value: rp.map_or("–".to_string(), |_| format!("{} GB", gb(used))),
            pct: rp.unwrap_or(0.0).clamp(0.0, 100.0) as f32,
            hot: rp.is_some_and(|p| p >= RAM_PCT),
            tip: rp.map_or(String::new(), |p| format!("{} / {} GB ({}%)", gb(used), gb(total), rnd(p))),
        });
        let gpus: Vec<&Value> = d.and_then(|d| d.get("gpus")).and_then(Value::as_array).map(|a| a.iter().take(MAX_GPU).collect()).unwrap_or_default();
        for (i, g) in gpus.iter().enumerate() {
            let (u, c) = (num(g.get("util")), num(g.get("tempC")));
            let val = format!("{}{}", u.map_or("–".to_string(), |u| format!("{}%", rnd(u))), c.map_or(String::new(), |c| format!(" · {}°", rnd(c))));
            let mem = if truthy(g.get("memTotalMb")) { format!("{} / {} GB", gb(num_or0(g.get("memUsedMb"))), gb(to_number(g.get("memTotalMb")))) } else { String::new() };
            let power = num(g.get("powerW")).map_or(String::new(), |w| format!("{} W", rnd(w)));
            rows.push(MeterRow {
                key: format!("g{i}"),
                label: if gpus.len() > 1 { format!("GPU {}", display(g.get("index"))) } else { "GPU".to_string() },
                value: val,
                pct: u.unwrap_or(0.0).clamp(0.0, 100.0) as f32,
                hot: c.is_some_and(|c| c >= GPU_C),
                tip: join_nonempty(&[text_or_empty(g.get("name")), mem, power], " · "),
            });
        }
        MetersView { visible: true, rows }
    }

    fn consider(&mut self, now: i64, key: &str, hot: bool, text: String, cand: &mut Vec<(String, String)>) {
        if !hot {
            self.hot_since.retain(|(k, _)| k != key);
            return;
        }
        let since = match self.hot_since.iter().find(|(k, _)| k == key) {
            Some((_, t)) => *t,
            None => {
                self.hot_since.push((key.to_string(), now));
                now
            }
        };
        let shown = self.shown_at.iter().find(|(k, _)| k == key).map_or(0, |(_, t)| *t);
        if now - since >= HOLD_MS && now - shown >= COOL_MS {
            cand.push((key.to_string(), text));
        }
    }

    fn watch(&mut self, env: &dyn Env, p: &Value, now: i64) -> Vec<LiveCmd> {
        if now - self.last_tick > 3 * SAMPLE_MS + 1000 {
            self.hot_since.clear(); // a gap in the ticks (pill hidden): the hot streak must be continuous
        }
        self.last_tick = now;
        let mut cand = Vec::new();
        if let Some(gpus) = p.get("gpus").and_then(Value::as_array) {
            for g in gpus {
                let c = num(g.get("tempC"));
                let key = format!("gpu{}", display(g.get("index")));
                self.consider(now, &key, c.is_some_and(|c| c >= GPU_C), format!("GPU {} · {}°C", int32(g.get("index")), rnd(c.unwrap_or(0.0))), &mut cand);
            }
        }
        let cpu = num(p.get("cpu"));
        self.consider(now, "cpu", cpu.is_some_and(|c| c >= CPU_PCT), format!("CPU {}%", rnd(cpu.unwrap_or(0.0))), &mut cand);
        // the meters are in front of the user; the chip comes after the island collapses
        if cand.is_empty() || env.expanded() {
            return Vec::new();
        }
        let (key, txt) = cand.remove(0); // GPUs come first; the others keep waiting (their cool-down has not started)
        match self.shown_at.iter_mut().find(|(k, _)| *k == key) {
            Some(e) => e.1 = now,
            None => self.shown_at.push((key, now)),
        }
        vec![LiveCmd::set(
            RowSpec::new("sysmon", CHIP_PRIORITY)
                .ttl(CHIP_MS)
                .anim(Anim::Pop, Anim::None, Anim::Fade)
                .with(Elem::icon_c(IconName::Warn, 17.0, ColorRole::Dim))
                .with(Elem::text(txt, TextStyle::Code)),
        )]
    }

    fn sync(&mut self, env: &dyn Env) -> Vec<LiveCmd> {
        let mut out = vec![LiveCmd::SysMeters(self.view(env))];
        if !enabled(env) {
            self.d = None;
            self.hot_since.clear();
            out.push(LiveCmd::clear("sysmon"));
        } else if self.d.is_none() {
            // first picture at once, not after 2 s
            out.push(LiveCmd::Query(Query::new("sysmon_get", "sysmon:get")));
        }
        out
    }
}

impl Provider for SysmonLive {
    fn name(&self) -> &'static str {
        "sysmon"
    }
    fn events(&self) -> &'static [&'static str] {
        &["sysmon:tick", "sysmon:get", "settings:changed"]
    }
    fn boot(&self) -> Vec<Query> {
        vec![Query::new("sysmon_get", "sysmon:get")]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, now: i64) -> Vec<LiveCmd> {
        match name {
            "sysmon:tick" => {
                if p.is_null() || !enabled(env) {
                    return Vec::new();
                }
                self.d = Some(p.clone());
                let mut out = vec![LiveCmd::SysMeters(self.view(env))];
                out.extend(self.watch(env, p, now));
                out
            }
            "sysmon:get" => {
                if p.is_null() || !enabled(env) {
                    return Vec::new();
                }
                self.d = Some(p.clone());
                vec![LiveCmd::SysMeters(self.view(env))]
            }
            "settings:changed" => self.sync(env),
            _ => Vec::new(),
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::{MemEnv, PillState};
    use serde_json::json;

    const T0: i64 = 1_700_000_000_000;

    fn gpu(index: u32, util: Value, temp: Value) -> Value {
        json!({"index": index, "name": "NVIDIA GeForce RTX 4090", "util": util, "tempC": temp, "memUsedMb": 5006, "memTotalMb": 24564, "powerW": 83.88})
    }
    fn sample(cpu: f64, used: u64, total: u64, gpus: Vec<Value>) -> Value {
        json!({"cpu": cpu, "ram": {"usedMb": used, "totalMb": total}, "gpus": gpus})
    }
    fn meters(cmds: &[LiveCmd]) -> &MetersView {
        cmds.iter().find_map(|c| if let LiveCmd::SysMeters(v) = c { Some(v) } else { None }).expect("meters")
    }
    fn chip(cmds: &[LiveCmd]) -> Option<&RowSpec> {
        cmds.iter().find_map(|c| if let LiveCmd::SetRow(r) = c { Some(r) } else { None })
    }
    fn chip_text(r: &RowSpec) -> String {
        match &r.elements[1] {
            Elem::Text { text, .. } => text.clone(),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn meters_one_gpu() {
        let mut s = SysmonLive::default();
        let env = MemEnv::default();
        let cmds = s.on_event(&env, "sysmon:tick", &sample(12.34, 16384, 32768, vec![gpu(0, json!(5), json!(33))]), T0);
        let v = meters(&cmds);
        assert!(v.visible);
        assert_eq!(v.rows.len(), 3);
        assert_eq!(v.rows[0], MeterRow { key: "cpu".into(), label: "CPU".into(), value: "12%".into(), pct: 12.34, hot: false, tip: "Процессор".into() });
        assert_eq!(v.rows[1], MeterRow { key: "ram".into(), label: "RAM".into(), value: "16.0 GB".into(), pct: 50.0, hot: false, tip: "16.0 / 32.0 GB (50%)".into() });
        assert_eq!(
            v.rows[2],
            MeterRow { key: "g0".into(), label: "GPU".into(), value: "5% · 33°".into(), pct: 5.0, hot: false, tip: "NVIDIA GeForce RTX 4090 · 4.9 / 24.0 GB · 84 W".into() }
        );
        // English tooltip for the processor
        let cmds = s.on_event(&MemEnv::en(), "sysmon:tick", &sample(1.0, 1, 2, vec![]), T0);
        assert_eq!(meters(&cmds).rows[0].tip, "Processor");
    }

    #[test]
    fn meters_two_gpus_are_numbered_and_a_third_is_dropped() {
        let mut s = SysmonLive::default();
        let env = MemEnv::default();
        let cmds = s.on_event(&env, "sysmon:tick", &sample(1.0, 1, 2, vec![gpu(0, json!(1), json!(40)), gpu(1, json!(90), json!(86)), gpu(2, json!(1), json!(1))]), T0);
        let v = meters(&cmds);
        assert_eq!(v.rows.len(), 4, "cpu + ram + 2 gpus");
        assert_eq!((v.rows[2].label.as_str(), v.rows[3].label.as_str()), ("GPU 0", "GPU 1"));
        assert!(!v.rows[2].hot && v.rows[3].hot, "86 C is hot");
    }

    #[test]
    fn meters_hot_thresholds() {
        let mut s = SysmonLive::default();
        let env = MemEnv::default();
        let cmds = s.on_event(&env, "sysmon:tick", &sample(95.0, 90, 100, vec![]), T0);
        let v = meters(&cmds);
        assert!(v.rows[0].hot, "cpu >= 95");
        assert!(v.rows[1].hot, "ram >= 90");
        assert_eq!(v.rows[1].value, "0.1 GB");
        let cmds = s.on_event(&env, "sysmon:tick", &sample(94.9, 89, 100, vec![]), T0 + 2000);
        assert!(!meters(&cmds).rows[0].hot && !meters(&cmds).rows[1].hot);
    }

    #[test]
    fn meters_missing_data_shows_dashes() {
        let mut s = SysmonLive::default();
        let env = MemEnv::default();
        let cmds = s.on_event(&env, "sysmon:tick", &json!({"gpus": [{"index": 0, "name": "X", "util": null, "tempC": null}]}), T0);
        let v = meters(&cmds);
        assert_eq!((v.rows[0].value.as_str(), v.rows[0].pct, v.rows[0].hot), ("–", 0.0, false));
        assert_eq!((v.rows[1].value.as_str(), v.rows[1].tip.as_str()), ("–", ""));
        assert_eq!((v.rows[2].value.as_str(), v.rows[2].tip.as_str()), ("–", "X"));
        // zero total RAM
        let cmds = s.on_event(&env, "sysmon:tick", &sample(5.0, 0, 0, vec![]), T0);
        assert_eq!(meters(&cmds).rows[1].value, "–");
        // out-of-range values clamp the bar
        let cmds = s.on_event(&env, "sysmon:tick", &sample(250.0, 1, 1, vec![]), T0);
        assert_eq!(meters(&cmds).rows[0].pct, 100.0);
    }

    #[test]
    fn view_before_any_sample_and_when_disabled() {
        let s = SysmonLive::default();
        let env = MemEnv::default();
        let v = s.view(&env);
        assert!(v.visible);
        assert_eq!(v.rows.iter().map(|r| r.value.as_str()).collect::<Vec<_>>(), ["–", "–"]);
        let mut off = MemEnv::default();
        off.set("live.sysmon", json!(false));
        assert_eq!(s.view(&off), MetersView { visible: false, rows: vec![] });
        off.set("live.sysmon", json!(null));
        assert!(enabled(&off), "only an explicit false disables it");
    }

    fn feed(s: &mut SysmonLive, env: &MemEnv, p: &Value, t: i64) -> Vec<LiveCmd> {
        s.on_event(env, "sysmon:tick", p, t)
    }

    /// Samples every 2 s from `from` to `to` (inclusive); the commands of the last one.
    fn run(s: &mut SysmonLive, env: &MemEnv, p: &Value, from: i64, to: i64) -> Vec<LiveCmd> {
        let mut last = Vec::new();
        let mut t = from;
        while t <= to {
            last = feed(s, env, p, t);
            t += 2000;
        }
        last
    }

    #[test]
    fn gpu_chip_after_ten_hot_seconds() {
        let mut s = SysmonLive::default();
        let env = MemEnv::default();
        let hot = sample(10.0, 1, 2, vec![gpu(0, json!(99), json!(88))]);
        for k in 0..5 {
            assert!(chip(&feed(&mut s, &env, &hot, T0 + k * 2000)).is_none(), "not yet at {k}");
        }
        let cmds = feed(&mut s, &env, &hot, T0 + 10_000);
        let r = chip(&cmds).expect("chip");
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms), ("sysmon", 90, Some(8000)));
        assert_eq!(chip_text(r), "GPU 0 · 88°C");
        assert_eq!(r.elements[0], Elem::icon_c(IconName::Warn, 17.0, ColorRole::Dim));
    }

    #[test]
    fn cool_down_is_five_minutes_per_source() {
        let mut s = SysmonLive::default();
        let env = MemEnv::default();
        let hot = sample(10.0, 1, 2, vec![gpu(0, json!(99), json!(90))]);
        let mut shown = Vec::new();
        let mut t = T0;
        while t <= T0 + 6 * 60_000 {
            if chip(&feed(&mut s, &env, &hot, t)).is_some() {
                shown.push(t - T0);
            }
            t += 2000;
        }
        assert_eq!(shown.len(), 2, "{shown:?}");
        assert_eq!(shown[0], 10_000);
        assert!(shown[1] - shown[0] >= COOL_MS);
    }

    #[test]
    fn streak_resets_when_the_temperature_drops_or_ticks_are_missing() {
        let mut s = SysmonLive::default();
        let env = MemEnv::default();
        let hot = sample(10.0, 1, 2, vec![gpu(0, json!(99), json!(90))]);
        let cool = sample(10.0, 1, 2, vec![gpu(0, json!(99), json!(60))]);
        feed(&mut s, &env, &hot, T0);
        feed(&mut s, &env, &hot, T0 + 6000);
        feed(&mut s, &env, &cool, T0 + 8000); // resets
        assert!(chip(&feed(&mut s, &env, &hot, T0 + 10_000)).is_none());
        assert!(chip(&feed(&mut s, &env, &hot, T0 + 14_000)).is_none());
        assert!(chip(&feed(&mut s, &env, &hot, T0 + 18_000)).is_none(), "only 8 s hot since the reset");
        assert!(chip(&feed(&mut s, &env, &hot, T0 + 20_000)).is_some());
        // a gap > 7 s (pill hidden) breaks the streak as well
        let mut s = SysmonLive::default();
        feed(&mut s, &env, &hot, T0);
        feed(&mut s, &env, &hot, T0 + 6000);
        assert!(chip(&feed(&mut s, &env, &hot, T0 + 14_000)).is_none(), "gap of 8 s cleared the streak");
    }

    #[test]
    fn cpu_chip_and_gpu_wins_over_cpu() {
        let mut s = SysmonLive::default();
        let env = MemEnv::default();
        let hot_cpu = sample(97.4, 1, 2, vec![]);
        let cmds = run(&mut s, &env, &hot_cpu, T0, T0 + 10_000);
        assert_eq!(chip_text(chip(&cmds).unwrap()), "CPU 97%");
        // both hot at once: the GPU chip goes first, the CPU keeps waiting and shows next tick
        let mut s = SysmonLive::default();
        let both = sample(99.0, 1, 2, vec![gpu(1, json!(1), json!(86))]);
        assert_eq!(chip_text(chip(&run(&mut s, &env, &both, T0, T0 + 10_000)).unwrap()), "GPU 1 · 86°C");
        assert_eq!(chip_text(chip(&feed(&mut s, &env, &both, T0 + 12_000)).unwrap()), "CPU 99%");
    }

    #[test]
    fn no_chip_while_the_island_is_expanded() {
        let mut s = SysmonLive::default();
        let mut env = MemEnv::default();
        let hot = sample(99.0, 1, 2, vec![]);
        env.state = PillState::Expanded;
        assert!(run(&mut s, &env, &hot, T0, T0 + 12_000).iter().all(|c| !matches!(c, LiveCmd::SetRow(_))));
        // and the cool-down did not start, so the chip comes right after it collapses
        env.state = PillState::Collapsed;
        assert!(chip(&feed(&mut s, &env, &hot, T0 + 14_000)).is_some());
    }

    #[test]
    fn disabled_ignores_samples_and_settings_sync_clears() {
        let mut s = SysmonLive::default();
        let mut env = MemEnv::default();
        env.set("live.sysmon", json!(false));
        assert!(feed(&mut s, &env, &sample(1.0, 1, 2, vec![]), T0).is_empty());
        let cmds = s.on_event(&env, "settings:changed", &Value::Null, T0);
        assert!(!meters(&cmds).visible);
        assert!(cmds.contains(&LiveCmd::clear("sysmon")));
        // switched on with no sample yet: ask for one at once
        env.unset("live.sysmon");
        let cmds = s.on_event(&env, "settings:changed", &Value::Null, T0);
        assert!(meters(&cmds).visible);
        assert!(cmds.contains(&LiveCmd::Query(Query::new("sysmon_get", "sysmon:get"))));
        // the answer fills the meters (no chip logic)
        let cmds = s.on_event(&env, "sysmon:get", &sample(50.0, 1, 2, vec![]), T0);
        assert_eq!(meters(&cmds).rows[0].value, "50%");
        // with a sample in hand a settings change does not ask again
        let cmds = s.on_event(&env, "settings:changed", &Value::Null, T0);
        assert!(!cmds.iter().any(|c| matches!(c, LiveCmd::Query(_))));
        assert!(s.on_event(&env, "sysmon:get", &Value::Null, T0).is_empty());
        assert_eq!(s.boot(), vec![Query::new("sysmon_get", "sysmon:get")]);
    }

    #[test]
    fn garbage_temperatures_are_not_hot() {
        let mut s = SysmonLive::default();
        let env = MemEnv::default();
        let bad = json!({"cpu": "hot", "gpus": [{"index": 0, "tempC": "90"}, {"index": 1, "tempC": null}]});
        feed(&mut s, &env, &bad, T0);
        assert!(chip(&feed(&mut s, &env, &bad, T0 + 20_000)).is_none());
        assert!(feed(&mut s, &env, &Value::Null, T0).is_empty());
    }
}

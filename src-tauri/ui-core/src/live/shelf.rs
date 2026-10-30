//! The live rows of the file shelf (port of the `setLive` / `react` calls of tab-shelf.js; the tab itself belongs to the panel module).
//!
//! * `shelf:changed {count, items}` -> a count chip (row `shelf`, priority 30, 5 s) when the number of files changes (not on first load),
//!   edge `file` for 3 s when a file was added, and the tab badge. The chip is skipped while the drop card has just said it.
//! * The compact drop card (row `shelf-drop`, priority 95, `tall`, 30 s safety) while a file drag hovers the collapsed island. A drag
//!   that rests 0.7 s opens the full island ([`LiveCmd::Broadcast`] `shelf:open-full`). While it is up the shell should sag the edge
//!   toward the cursor and tint it with the accent (`shelf:peek {on}` broadcast).
//!
//! Gates: Modules > File shelf (`module.shelf`) and Settings > Live events > File shelf (`live.shelf`).

use std::any::Any;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::*;

pub const SHELF_PRIORITY: i32 = 30;
pub const SHELF_CHIP_MS: u32 = 5000;
pub const DROP_PRIORITY: i32 = 95;
pub const DWELL_MS: i64 = 700;
pub const SAFETY_MS: i64 = 30_000;

#[derive(Debug)]
pub struct ShelfLive {
    /// Files on the shelf at the last event, -1 = nothing seen yet.
    seen: i64,
    peek: Option<Peek>,
}

#[derive(Debug, Clone, Copy)]
struct Peek {
    since: i64,
    landed: bool,
}

impl Default for ShelfLive {
    fn default() -> Self {
        ShelfLive { seen: -1, peek: None }
    }
}

impl ShelfLive {
    pub fn count(&self) -> i64 {
        self.seen.max(0)
    }

    fn on_items(&mut self, env: &dyn Env, n: i64) -> Vec<LiveCmd> {
        let was = self.seen;
        self.seen = n;
        let mut out = Vec::new();
        if was >= 0 && n > was {
            out.push(LiveCmd::react("shelf", Preset::File, Some(3000), None)); // item added (not on first load, not on removal)
        }
        out.push(LiveCmd::TabBadge { tab: "shelf".into(), count: n as u32 });
        // Transient: a permanent widget would hide the clock for good (the badge is the permanent part).
        let landed = self.peek.is_some_and(|p| p.landed);
        if n > 0 && was >= 0 && n != was && env.module_on("module.shelf") && env.live_on("shelf") && !landed {
            out.push(LiveCmd::set(
                RowSpec::new("shelf", SHELF_PRIORITY)
                    .ttl(SHELF_CHIP_MS)
                    .width(140.0)
                    .anim(Anim::Pop, Anim::None, Anim::Fade)
                    .with(Elem::icon(IconName::Shelf, 18.0))
                    .with(Elem::text(n.to_string(), TextStyle::Count)),
            ));
        } else {
            out.push(LiveCmd::clear("shelf"));
        }
        out
    }

    fn drop_row(&self, env: &dyn Env) -> RowSpec {
        let n = self.count();
        RowSpec::new("shelf-drop", DROP_PRIORITY)
            .ttl(SAFETY_MS as u32)
            .key("drop")
            .tall()
            .anim(Anim::Slide, Anim::Glide(400.0), Anim::Fade)
            .with(Elem::icon(IconName::Download, 22.0))
            .with(Elem::text(env.tr("Отпустите, чтобы положить на полку", "Drop to keep on the shelf"), super::model::TextStyle::Label))
            .with(Elem::text(
                if n > 0 { env.tr(&format!("Уже на полке: {n}"), &format!("On the shelf now: {n}")) } else { env.tr("Файлы останутся на своих местах", "Files stay where they are") },
                super::model::TextStyle::Dim,
            ))
    }

    /// A file drag entered the collapsed island.
    pub fn start_peek(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        if self.peek.is_some() {
            return Vec::new();
        }
        self.peek = Some(Peek { since: now, landed: false });
        vec![LiveCmd::set(self.drop_row(env)), LiveCmd::Broadcast { name: "shelf:peek".into(), payload: json!({ "on": true }) }]
    }

    pub fn end_peek(&mut self) -> Vec<LiveCmd> {
        if self.peek.take().is_none() {
            return Vec::new();
        }
        vec![LiveCmd::clear("shelf-drop"), LiveCmd::Broadcast { name: "shelf:peek".into(), payload: json!({ "on": false }) }]
    }

    /// The drop happened on the card: the chip is suppressed (the card has just said it) and the dwell timer stops.
    pub fn mark_landed(&mut self) {
        if let Some(p) = self.peek.as_mut() {
            p.landed = true;
        }
    }

    pub fn peeking(&self) -> bool {
        self.peek.is_some()
    }
}

impl Provider for ShelfLive {
    fn name(&self) -> &'static str {
        "shelf"
    }
    fn events(&self) -> &'static [&'static str] {
        &["shelf:changed"]
    }
    fn boot(&self) -> Vec<Query> {
        // the first listing sets the baseline without a chip
        vec![Query::new("shelf_list", "shelf:changed")]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        // the event is { count, items }; the boot answer of shelf_list is the bare items array
        let n = match p {
            Value::Array(a) => a.len() as i64,
            Value::Object(_) => p.get("items").and_then(Value::as_array).map(|a| a.len() as i64).unwrap_or_else(|| p.get("count").and_then(Value::as_i64).unwrap_or(0)),
            _ => 0,
        };
        self.on_items(env, n)
    }
    fn on_tick(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        let Some(p) = self.peek else { return Vec::new() };
        if now - p.since >= SAFETY_MS {
            return self.end_peek();
        }
        if !p.landed && now - p.since >= DWELL_MS {
            // the drag rests on the card: open the whole island under it
            let mut out = self.end_peek();
            out.push(LiveCmd::Broadcast { name: "shelf:open-full".into(), payload: Value::Null });
            return out;
        }
        let _ = env;
        Vec::new()
    }
    fn tick_ms(&self, _now: i64) -> Option<u32> {
        self.peek.map(|_| 100)
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::MemEnv;

    const T0: i64 = 1_700_000_000_000;

    fn items(n: usize) -> Value {
        json!({"count": n, "items": (0..n).map(|i| json!({"id": i})).collect::<Vec<_>>()})
    }
    fn has_row(c: &[LiveCmd]) -> Option<&RowSpec> {
        c.iter().find_map(|c| if let LiveCmd::SetRow(r) = c { Some(r) } else { None })
    }

    #[test]
    fn first_load_is_a_baseline_without_chip_or_edge() {
        let mut s = ShelfLive::default();
        let env = MemEnv::default();
        let c = s.on_event(&env, "shelf:changed", &items(3), T0);
        assert_eq!(c, vec![LiveCmd::TabBadge { tab: "shelf".into(), count: 3 }, LiveCmd::clear("shelf")]);
        assert_eq!(s.count(), 3);
    }

    #[test]
    fn adding_a_file_pops_the_chip_and_pulses() {
        let mut s = ShelfLive::default();
        let env = MemEnv::default();
        s.on_event(&env, "shelf:changed", &items(1), T0);
        let c = s.on_event(&env, "shelf:changed", &items(2), T0 + 100);
        assert_eq!(c[0], LiveCmd::react("shelf", Preset::File, Some(3000), None));
        assert_eq!(c[1], LiveCmd::TabBadge { tab: "shelf".into(), count: 2 });
        let r = has_row(&c).expect("chip");
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms, r.width), ("shelf", 30, Some(5000), 140.0));
        assert_eq!(r.elements[1], Elem::text("2", TextStyle::Count));
    }

    #[test]
    fn removal_shows_the_chip_without_the_edge_reaction() {
        let mut s = ShelfLive::default();
        let env = MemEnv::default();
        s.on_event(&env, "shelf:changed", &items(3), T0);
        let c = s.on_event(&env, "shelf:changed", &items(2), T0 + 1);
        assert!(!c.iter().any(|c| matches!(c, LiveCmd::React { .. })));
        assert!(has_row(&c).is_some());
        // emptied: nothing to count
        let c = s.on_event(&env, "shelf:changed", &items(0), T0 + 2);
        assert!(has_row(&c).is_none() && c.contains(&LiveCmd::clear("shelf")));
        // the same count again (a refresh): no chip
        let c = s.on_event(&env, "shelf:changed", &items(0), T0 + 3);
        assert!(has_row(&c).is_none());
    }

    #[test]
    fn gates() {
        let mut s = ShelfLive::default();
        let mut env = MemEnv::default();
        s.on_event(&env, "shelf:changed", &items(1), T0);
        env.set("live.shelf", json!(false));
        assert!(has_row(&s.on_event(&env, "shelf:changed", &items(2), T0)).is_none());
        env.unset("live.shelf");
        env.set("module.shelf", json!(false));
        assert!(has_row(&s.on_event(&env, "shelf:changed", &items(3), T0)).is_none());
        // the boot answer is a bare array
        env.unset("module.shelf");
        assert!(has_row(&s.on_event(&env, "shelf:changed", &json!([{}, {}, {}, {}]), T0)).is_some());
        assert_eq!(s.boot(), vec![Query::new("shelf_list", "shelf:changed")]);
        // a count-only payload
        assert!(has_row(&s.on_event(&env, "shelf:changed", &json!({"count": 9}), T0)).is_some());
        assert_eq!(s.count(), 9);
        assert_eq!(s.on_event(&env, "shelf:changed", &Value::Null, T0)[0], LiveCmd::TabBadge { tab: "shelf".into(), count: 0 });
    }

    #[test]
    fn drop_card_lifecycle() {
        let mut s = ShelfLive::default();
        let env = MemEnv::default();
        let c = s.start_peek(&env, T0);
        let r = has_row(&c).unwrap();
        assert_eq!((r.id.as_str(), r.priority, r.tall, r.ttl_ms, r.key.as_str()), ("shelf-drop", 95, true, Some(30_000), "drop"));
        assert_eq!(r.elements[1], Elem::text("Отпустите, чтобы положить на полку", TextStyle::Label));
        assert_eq!(r.elements[2], Elem::text("Файлы останутся на своих местах", TextStyle::Dim));
        assert!(c.contains(&LiveCmd::Broadcast { name: "shelf:peek".into(), payload: json!({"on": true}) }));
        assert!(s.start_peek(&env, T0).is_empty(), "already up");
        assert_eq!(s.tick_ms(T0), Some(100));
        let e = s.end_peek();
        assert_eq!(e[0], LiveCmd::clear("shelf-drop"));
        assert!(!s.peeking() && s.end_peek().is_empty());
        assert_eq!(s.tick_ms(T0), None);
    }

    #[test]
    fn drop_card_texts_with_files_on_the_shelf() {
        let mut s = ShelfLive::default();
        s.on_event(&MemEnv::default(), "shelf:changed", &items(4), T0);
        let r = has_row(&s.start_peek(&MemEnv::en(), T0)).unwrap().clone();
        assert_eq!(r.elements[1], Elem::text("Drop to keep on the shelf", TextStyle::Label));
        assert_eq!(r.elements[2], Elem::text("On the shelf now: 4", TextStyle::Dim));
        let mut s2 = ShelfLive::default();
        s2.on_event(&MemEnv::default(), "shelf:changed", &items(4), T0);
        assert_eq!(has_row(&s2.start_peek(&MemEnv::default(), T0)).unwrap().elements[2], Elem::text("Уже на полке: 4", TextStyle::Dim));
    }

    #[test]
    fn dwell_opens_the_full_island_and_safety_ends_the_card() {
        let mut s = ShelfLive::default();
        let env = MemEnv::default();
        s.start_peek(&env, T0);
        assert!(s.on_tick(&env, T0 + 699).is_empty());
        let c = s.on_tick(&env, T0 + 700);
        assert!(c.contains(&LiveCmd::clear("shelf-drop")));
        assert!(c.contains(&LiveCmd::Broadcast { name: "shelf:open-full".into(), payload: Value::Null }));
        assert!(!s.peeking());
        // landed: no dwell, only the 30 s safety
        s.start_peek(&env, T0);
        s.mark_landed();
        assert!(s.on_tick(&env, T0 + 5000).is_empty());
        let c = s.on_tick(&env, T0 + 30_000);
        assert!(c.contains(&LiveCmd::clear("shelf-drop")));
        assert!(!c.iter().any(|c| matches!(c, LiveCmd::Broadcast { name, .. } if name == "shelf:open-full")));
    }

    #[test]
    fn landed_card_suppresses_the_count_chip() {
        let mut s = ShelfLive::default();
        let env = MemEnv::default();
        s.on_event(&env, "shelf:changed", &items(1), T0);
        s.start_peek(&env, T0);
        s.mark_landed();
        let c = s.on_event(&env, "shelf:changed", &items(2), T0 + 50);
        assert!(has_row(&c).is_none(), "the drop card has just said it");
        assert!(c.iter().any(|c| matches!(c, LiveCmd::React { .. })), "the edge still reacts");
    }
}

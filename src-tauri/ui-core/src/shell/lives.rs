//! Live-row selection: which `Pill.setLive` slot owns the collapsed capsule (pill.js `lives` + the choice made in `renderLive`).
//!
//! The row CONTENT is a [`LiveSlot::spec`] owned by the `live` area; this module only decides priority, ttl, the re-render key, the
//! wavy-outline accent and the `tall` flag. Highest `priority` wins, ties keep the slot that was inserted first; expired slots are
//! deleted while selecting; `key` is what makes a row re-render (the caller edits it in place), otherwise the spec itself is the key.

use serde_json::Value;

/// One registered live slot.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveSlot {
    pub id: String,
    pub priority: f64,
    /// wall ms; 0 = no ttl
    pub until_ms: i64,
    /// `key`: the row is re-rendered only when it changes ("" = use the spec)
    pub key: String,
    /// CSS colour; also animates the wavy outline ("" = none)
    pub accent: String,
    /// sized by its own height (the shelf drop target)
    pub tall: bool,
    /// opaque row description for the renderer
    pub spec: Value,
}

/// `Pill.setLive(id, w)` argument.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LiveInput {
    pub priority: f64,
    pub spec: Value,
    pub key: String,
    pub ttl_ms: i64,
    pub accent: String,
    pub tall: bool,
    pub urgent: bool,
}

impl LiveSlot {
    pub fn from_input(id: &str, w: &LiveInput, wall_ms: i64) -> LiveSlot {
        LiveSlot {
            id: id.to_string(),
            priority: w.priority,
            until_ms: if w.ttl_ms != 0 { wall_ms + w.ttl_ms } else { 0 },
            key: w.key.clone(),
            accent: w.accent.clone(),
            tall: w.tall,
            spec: w.spec.clone(),
        }
    }
    /// `bid + '|' + (best.key || html)`: the row is re-rendered only when this changes.
    pub fn render_key(&self) -> String {
        let k = if self.key.is_empty() { self.spec.to_string() } else { self.key.clone() };
        format!("{}|{}", self.id, k)
    }
}

/// Result of one selection pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    /// the winner (a copy)
    pub best: Option<LiveSlot>,
    /// earliest `until` among the remaining slots (wall ms), for the re-render timer
    pub next_until: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct Lives {
    /// insertion ordered like the JS `Map` (`set` of an existing id keeps its position)
    slots: Vec<LiveSlot>,
}

impl Lives {
    pub fn new() -> Lives {
        Lives::default()
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    pub fn ids(&self) -> Vec<&str> {
        self.slots.iter().map(|s| s.id.as_str()).collect()
    }
    pub fn get(&self, id: &str) -> Option<&LiveSlot> {
        self.slots.iter().find(|s| s.id == id)
    }
    pub fn set(&mut self, slot: LiveSlot) {
        match self.slots.iter_mut().find(|s| s.id == slot.id) {
            Some(s) => *s = slot,
            None => self.slots.push(slot),
        }
    }
    pub fn delete(&mut self, id: &str) -> bool {
        let n = self.slots.len();
        self.slots.retain(|s| s.id != id);
        self.slots.len() != n
    }
    /// Drop expired slots, pick the winner (strictly higher priority replaces; ties keep the earlier slot).
    pub fn select(&mut self, now_wall: i64) -> Selection {
        self.slots.retain(|s| !(s.until_ms != 0 && s.until_ms <= now_wall));
        let mut best: Option<&LiveSlot> = None;
        for s in &self.slots {
            if best.is_none_or(|b| s.priority > b.priority) {
                best = Some(s);
            }
        }
        Selection { best: best.cloned(), next_until: self.slots.iter().filter(|s| s.until_ms != 0).map(|s| s.until_ms).min() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn slot(id: &str, p: f64, until: i64) -> LiveSlot {
        LiveSlot { id: id.into(), priority: p, until_ms: until, key: String::new(), accent: String::new(), tall: false, spec: json!({"id": id}) }
    }

    #[test]
    fn highest_priority_wins() {
        let mut l = Lives::new();
        l.set(slot("a", 1.0, 0));
        l.set(slot("b", 5.0, 0));
        l.set(slot("c", 3.0, 0));
        assert_eq!(l.select(0).best.unwrap().id, "b");
    }

    #[test]
    fn ties_keep_the_first_inserted_and_set_keeps_position() {
        let mut l = Lives::new();
        l.set(slot("a", 2.0, 0));
        l.set(slot("b", 2.0, 0));
        assert_eq!(l.select(0).best.unwrap().id, "a");
        l.set(slot("a", 2.0, 0)); // re-set: same position, still first
        assert_eq!(l.ids(), vec!["a", "b"]);
        assert_eq!(l.select(0).best.unwrap().id, "a");
        l.delete("a");
        assert_eq!(l.select(0).best.unwrap().id, "b");
    }

    #[test]
    fn ttl_expiry_deletes_and_falls_back() {
        let mut l = Lives::new();
        l.set(slot("low", 1.0, 0));
        l.set(slot("hi", 9.0, 1500));
        let s = l.select(1000);
        assert_eq!(s.best.unwrap().id, "hi");
        assert_eq!(s.next_until, Some(1500));
        let s = l.select(1500); // until <= now: expired
        assert_eq!(s.best.unwrap().id, "low");
        assert_eq!(s.next_until, None);
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn empty_selects_nothing_so_the_clock_shows() {
        let mut l = Lives::new();
        let s = l.select(0);
        assert!(s.best.is_none() && s.next_until.is_none());
        assert!(l.is_empty());
    }

    #[test]
    fn negative_and_zero_priorities_still_win_over_nothing() {
        let mut l = Lives::new();
        l.set(slot("z", 0.0, 0));
        l.set(slot("n", -1.0, 0));
        assert_eq!(l.select(0).best.unwrap().id, "z");
    }

    #[test]
    fn from_input_computes_until_from_the_ttl() {
        let w = LiveInput { priority: 4.0, ttl_ms: 2500, accent: "#fff".into(), tall: true, spec: json!(1), ..Default::default() };
        let s = LiveSlot::from_input("x", &w, 10_000);
        assert_eq!((s.until_ms, s.priority, s.tall, s.accent.as_str()), (12_500, 4.0, true, "#fff"));
        let s = LiveSlot::from_input("x", &LiveInput::default(), 10_000);
        assert_eq!(s.until_ms, 0);
    }

    #[test]
    fn render_key_prefers_the_explicit_key() {
        let mut s = slot("music", 1.0, 0);
        let k1 = s.render_key();
        s.spec = json!({"id": "music", "pos": 12});
        assert_ne!(k1, s.render_key()); // spec change re-renders when no key
        s.key = "track-7".into();
        let k2 = s.render_key();
        s.spec = json!({"id": "music", "pos": 13});
        assert_eq!(k2, s.render_key()); // edited in place under the same key: no re-render
        assert_eq!(k2, "music|track-7");
    }
}

//! The live-row board (`lives` of pill.js) and the quiet-hours filter that sits in front of it.
//!
//! The shell can use [`Board`] as is, or keep its own store and only borrow [`QUIET_SRC`] and
//! [`filter_quiet`]; both behave exactly like `Pill.setLive` / `renderLive` / `Pill.react` /
//! `Pill.toast` in the web UI.

use super::model::{LiveCmd, RowSpec};

/// Sources whose chips, pulses and edge reactions are muted during quiet hours (`QUIET_SRC` of pill.js).
pub const QUIET_SRC: [&str; 7] = ["agents", "agent-done", "copy", "download", "steam", "sysmon", "kbd"];

pub fn is_quiet_src(id: &str) -> bool {
    QUIET_SRC.contains(&id)
}

/// Apply quiet hours to commands a provider produced:
/// * `SetRow` of a quiet source becomes `ClearRow` (a muted chip is never shown, and a standing one goes away),
/// * `React` of a quiet source becomes `Unreact`,
/// * a `Toast` that is not exempt (urgent, or an id starting `timer` / `remind`) is dropped, never shown later.
///
/// `EdgePulse` is not touched: the providers that guard it do so themselves (the Steam "done" pulse is not guarded, as before).
pub fn filter_quiet(cmds: Vec<LiveCmd>, quiet: bool) -> Vec<LiveCmd> {
    if !quiet {
        return cmds;
    }
    cmds.into_iter()
        .filter_map(|c| match c {
            LiveCmd::SetRow(r) if is_quiet_src(&r.id) => Some(LiveCmd::ClearRow(r.id)),
            LiveCmd::React { source, .. } if is_quiet_src(&source) => Some(LiveCmd::Unreact(source)),
            LiveCmd::Toast(card) if !card.quiet_exempt() => None,
            other => Some(other),
        })
        .collect()
}

struct Entry {
    spec: RowSpec,
    /// Epoch ms the row expires at, 0 = stays.
    until: i64,
}

/// Row store: set/clear by id, expiry by ttl, the top row by priority.
#[derive(Default)]
pub struct Board {
    rows: Vec<Entry>,
}

impl Board {
    pub fn new() -> Board {
        Board::default()
    }

    /// `lives.set(id, ...)`: an existing id keeps its place (Map semantics), the ttl restarts at `now`.
    pub fn set(&mut self, spec: RowSpec, now: i64) {
        let until = spec.ttl_ms.filter(|&t| t > 0).map_or(0, |t| now + i64::from(t));
        match self.rows.iter_mut().find(|e| e.spec.id == spec.id) {
            Some(e) => *e = Entry { spec, until },
            None => self.rows.push(Entry { spec, until }),
        }
    }

    pub fn clear(&mut self, id: &str) -> bool {
        let n = self.rows.len();
        self.rows.retain(|e| e.spec.id != id);
        self.rows.len() != n
    }

    /// Feed the row commands of a provider; other commands are ignored here.
    pub fn apply(&mut self, cmd: &LiveCmd, now: i64) {
        match cmd {
            LiveCmd::SetRow(r) => self.set(r.clone(), now),
            LiveCmd::ClearRow(id) => {
                self.clear(id);
            }
            _ => {}
        }
    }

    /// Drop expired rows, return their ids.
    pub fn prune(&mut self, now: i64) -> Vec<String> {
        let gone: Vec<String> = self.rows.iter().filter(|e| e.until != 0 && e.until <= now).map(|e| e.spec.id.clone()).collect();
        self.rows.retain(|e| e.until == 0 || e.until > now);
        gone
    }

    /// The row the capsule shows: the highest priority, the earliest inserted on a tie. Expired rows are dropped first.
    pub fn top(&mut self, now: i64) -> Option<&RowSpec> {
        self.prune(now);
        let mut best: Option<&Entry> = None;
        for e in &self.rows {
            if best.is_none_or(|b| e.spec.priority > b.spec.priority) {
                best = Some(e);
            }
        }
        best.map(|e| &e.spec)
    }

    pub fn get(&self, id: &str) -> Option<&RowSpec> {
        self.rows.iter().find(|e| e.spec.id == id).map(|e| &e.spec)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// When the next ttl runs out (the renderer re-renders then).
    pub fn next_expiry(&self) -> Option<i64> {
        self.rows.iter().filter(|e| e.until != 0).map(|e| e.until).min()
    }

    /// The 20 s sweep of pill.js: quiet hours began, remove the muted sources. True when something went.
    pub fn sweep_quiet(&mut self) -> bool {
        let n = self.rows.len();
        self.rows.retain(|e| !is_quiet_src(&e.spec.id));
        self.rows.len() != n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::model::{CardSpec, Preset};

    fn row(id: &str, p: i32, ttl: Option<u32>) -> RowSpec {
        let mut r = RowSpec::new(id, p);
        r.ttl_ms = ttl;
        r
    }

    #[test]
    fn highest_priority_wins_first_inserted_on_tie() {
        let mut b = Board::new();
        b.set(row("media", 50, None), 0);
        b.set(row("kbd", 90, Some(1400)), 0);
        assert_eq!(b.top(100).unwrap().id, "kbd");
        b.set(row("a", 90, None), 0); // tie with kbd: kbd was inserted first
        assert_eq!(b.top(100).unwrap().id, "kbd");
        // re-setting kbd keeps its place in line
        b.set(row("kbd", 90, Some(1400)), 200);
        assert_eq!(b.top(300).unwrap().id, "kbd");
    }

    #[test]
    fn ttl_expiry_and_renewal() {
        let mut b = Board::new();
        b.set(row("volume", 88, Some(1600)), 1000);
        assert_eq!(b.next_expiry(), Some(2600));
        assert!(b.top(2599).is_some());
        assert!(b.top(2600).is_none()); // until <= now expires
        assert!(b.is_empty());
        b.set(row("volume", 88, Some(1600)), 1000);
        b.set(row("volume", 88, Some(1600)), 2000); // renewed
        assert!(b.top(3000).is_some());
        assert_eq!(b.prune(3600), vec!["volume".to_string()]);
        // no ttl never expires, ttl 0 means none like `ttlMs ? ... : 0`
        b.set(row("rec", 100, None), 0);
        b.set(row("z", 1, Some(0)), 0);
        assert!(b.top(i64::MAX / 2).is_some());
        assert_eq!(b.len(), 2);
        assert_eq!(b.next_expiry(), None);
    }

    #[test]
    fn apply_set_and_clear() {
        let mut b = Board::new();
        b.apply(&LiveCmd::SetRow(row("x", 1, None)), 0);
        assert!(b.get("x").is_some());
        b.apply(&LiveCmd::clear("x"), 0);
        assert!(b.get("x").is_none());
        b.apply(&LiveCmd::EdgePulse(0.5), 0); // ignored
        assert!(b.is_empty());
    }

    #[test]
    fn quiet_source_membership_is_exact() {
        for id in ["agents", "agent-done", "copy", "download", "steam", "sysmon", "kbd"] {
            assert!(is_quiet_src(id), "{id}");
        }
        for id in ["media", "volume", "obs", "rec", "voice", "timer", "shelf", "shelf-drop", "remind", "capture"] {
            assert!(!is_quiet_src(id), "{id}");
        }
        assert_eq!(QUIET_SRC.len(), 7);
    }

    #[test]
    fn filter_quiet_rewrites_and_drops() {
        let cmds = vec![
            LiveCmd::SetRow(row("kbd", 90, Some(1400))),
            LiveCmd::SetRow(row("media", 50, None)),
            LiveCmd::react("agents", Preset::Attention, None, None),
            LiveCmd::react("media", Preset::Music, None, None),
            LiveCmd::Toast(CardSpec::text("download:1", 9000, "t", None)),
            LiveCmd::Toast(CardSpec::text("timer-done", 9000, "t", None)),
            LiveCmd::EdgePulse(0.7),
        ];
        let out = filter_quiet(cmds.clone(), true);
        assert_eq!(out.len(), 6);
        assert_eq!(out[0], LiveCmd::clear("kbd"));
        assert!(matches!(&out[1], LiveCmd::SetRow(r) if r.id == "media"));
        assert_eq!(out[2], LiveCmd::unreact("agents"));
        assert!(matches!(&out[3], LiveCmd::React { source, .. } if source == "media"));
        assert!(matches!(&out[4], LiveCmd::Toast(c) if c.id == "timer-done"));
        assert_eq!(out[5], LiveCmd::EdgePulse(0.7)); // not touched here
        assert_eq!(filter_quiet(cmds.clone(), false), cmds);
    }

    #[test]
    fn sweep_removes_only_quiet_sources() {
        let mut b = Board::new();
        for id in ["kbd", "media", "steam", "timer"] {
            b.set(row(id, 10, None), 0);
        }
        assert!(b.sweep_quiet());
        assert_eq!((b.len(), b.get("media").is_some(), b.get("timer").is_some()), (2, true, true));
        assert!(!b.sweep_quiet());
    }
}
